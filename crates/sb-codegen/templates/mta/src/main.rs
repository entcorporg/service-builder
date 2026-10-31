use std::sync::Arc;

use anyhow::{Context, Result};
use mta_service::core::{config::Config, consumer};
use mta_service::handlers::metrics::MetricsAccess;
use mta_service::mta::queue_worker::{self, WorkerState};
use mta_service::{bounce, routes};
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<()> {
    // Requis par sqlx (tls-rustls) — même init que svc-auth.
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("failed to install ring crypto provider");

    dotenvy::from_path(concat!(env!("CARGO_MANIFEST_DIR"), "/.env")).ok();

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "mta_service=debug,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    tracing::info!(version = env!("CARGO_PKG_VERSION"), "Démarrage du MTA");

    let config = Config::from_env().context("chargement de la configuration")?;

    // Séries métier créées à 0 avant toute mesure (voir `metrics::init`).
    mta_service::metrics::init(consumer::TOPICS);

    // Attend le serveur et crée la base si elle n'existe pas encore, comme les autres services.
    let db = eda_runtime::connect_database(&config.database_url)
        .await
        .context("connexion PostgreSQL")?;

    sqlx::migrate!("./migrations")
        .run(&db)
        .await
        .context("migrations SQL")?;

    // Les mails du projet qui ne sont pas encore en base.
    let seeded = mta_service::seed::run(&db).await.context("mails du projet")?;
    if seeded > 0 {
        tracing::info!(count = seeded, "mails du projet ajoutés");
    }

    // Remet les jobs bloqués en `sending` à `pending` (reprise après crash).
    let stale = queue_worker::reset_stale_sending(&db).await?;
    if stale > 0 {
        tracing::warn!(count = stale, "jobs 'sending' réinitialisés à 'pending'");
    }

    let config = Arc::new(config);

    // ── Tâches de fond ────────────────────────────────────────────────────────
    // 1. Consumer Kafka — insère les events dans mail_queue (idempotent).
    // 2. Queue worker  — livre les jobs en attente (SMTP raw + DKIM).
    // 3. Bounce SMTP   — reçoit les NDR/DSN entrants et met à jour la file.
    let kafka_handle = consumer::spawn(&config.brokers, db.clone());

    let worker_handle = {
        let state = WorkerState {
            db: db.clone(),
            config: config.clone(),
        };
        tokio::spawn(async move { queue_worker::run(state).await })
    };

    let bounce_handle = {
        let addr = config.bounce_addr();
        let hostname = Arc::new(config.bounce_hostname.clone());
        let db2 = db.clone();
        tokio::spawn(async move {
            loop {
                if let Err(e) = bounce::server::run(&addr, hostname.clone(), db2.clone()).await {
                    tracing::error!(error = %e, "serveur bounce arrêté, redémarrage dans 5s");
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                }
            }
        })
    };

    // ── Serveur HTTP (health / readiness / metrics) ─────────────────────────────
    let http_addr = format!("0.0.0.0:{}", config.server_port);
    let metrics_access =
        MetricsAccess::new(config.metrics_token.as_deref(), config.metrics_allow_open);
    let app = routes::build(db.clone(), metrics_access).layer(TraceLayer::new_for_http());
    let listener = tokio::net::TcpListener::bind(&http_addr)
        .await
        .with_context(|| format!("bind HTTP {http_addr}"))?;
    tracing::info!(
        "MTA HTTP sur {http_addr} — kafka: {}",
        config.brokers
    );
    let http_handle = tokio::spawn(async move { axum::serve(listener, app).await });

    // Attendre qu'une tâche se termine (crash non géré).
    tokio::select! {
        res = kafka_handle  => tracing::error!("tâche kafka terminée : {res:?}"),
        res = worker_handle => tracing::error!("tâche worker terminée : {res:?}"),
        res = bounce_handle => tracing::error!("tâche bounce terminée : {res:?}"),
        res = http_handle   => tracing::error!("serveur HTTP terminé : {res:?}"),
    }

    Ok(())
}
