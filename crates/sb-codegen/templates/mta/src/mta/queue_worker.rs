//! Worker de la file d'attente : lit les jobs `pending` dans `mail_queue`,
//! construit le message MIME, le signe en DKIM, résout les MX et livre via SMTP.

use std::{collections::HashMap, sync::Arc, time::Duration};

use crate::core::config::Config;
use anyhow::{Context, Result};
use futures::stream::{FuturesUnordered, StreamExt};
use rand::RngExt;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    message,
    metrics::{self, DELIVERIES, DELIVERY_DURATION},
    mta::{dkim, dns, smtp},
    template::{EmailEvent, TemplateEngine},
};

// ─── Types de statuts (miroir de l'enum PostgreSQL `mail_status`) ─────────────

#[derive(Debug, Clone, PartialEq, Eq, sqlx::Type)]
#[sqlx(type_name = "mail_status", rename_all = "lowercase")]
pub enum MailStatus {
    Pending,
    Sending,
    Sent,
    Failed,
    Bounced,
}

// ─── Struct de la ligne DB ────────────────────────────────────────────────────

#[derive(Debug, sqlx::FromRow)]
struct MailJob {
    id: Uuid,
    event_type: String,
    recipient: String,
    payload: Value, // JSONB → serde_json::Value
}

// ─── État partagé du worker ───────────────────────────────────────────────────

pub struct WorkerState {
    pub db: PgPool,
    pub config: Arc<Config>,
}

// ─── Boucle principale ────────────────────────────────────────────────────────

/// Démarre la boucle infinie du worker.
///
/// Comportement :
/// - Charge jusqu'à `worker_batch_size` jobs et les traite tous en parallèle
///   (concurrence limitée par `worker_concurrency` connexions SMTP simultanées).
/// - Si le batch était plein → il reste probablement des jobs → relance immédiatement.
/// - Si le batch était vide ou partiel → dort 5s avant le prochain cycle.
pub async fn run(state: WorkerState) -> ! {
    let batch_size = state.config.worker_batch_size;
    let concurrency = state.config.worker_concurrency;
    tracing::info!(batch_size, concurrency, "Queue worker démarré");

    let state = Arc::new(state);

    loop {
        if let Err(e) = metrics::refresh_queue_jobs(&state.db).await {
            tracing::warn!(error = %e, "Rafraîchissement de mta_queue_jobs échoué");
        }
        match tick(state.clone(), batch_size, concurrency).await {
            Ok(n) if n == batch_size as usize => {
                tracing::debug!(processed = n, "Batch plein, enchaînement immédiat");
            }
            Ok(n) if n > 0 => {
                tracing::info!(processed = n, "Batch traité");
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
            Ok(_) => {
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
            Err(e) => {
                tracing::error!(error = %e, "Erreur dans le queue worker");
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }
}

// ─── Une itération de la boucle ───────────────────────────────────────────────

async fn tick(state: Arc<WorkerState>, batch_size: u32, concurrency: usize) -> Result<usize> {
    // Templates relus à chaque cycle (une modification en base s'applique sans
    // redémarrage), avant de verrouiller : une erreur ici ne laisse aucun job en `sending`.
    let templates = Arc::new(TemplateEngine::load(&state.db).await?);

    // ── 1. Récupérer et verrouiller un batch de jobs ──────────────────────────
    let mut tx = state.db.begin().await?;

    let jobs: Vec<MailJob> = sqlx::query_as(
        r#"
        SELECT id, event_type, recipient, payload
        FROM   mail_queue
        WHERE  status = 'pending'
          AND  next_attempt_at <= NOW()
        ORDER  BY next_attempt_at
        LIMIT  $1
        FOR UPDATE SKIP LOCKED
        "#,
    )
    .bind(batch_size as i64)
    .fetch_all(&mut *tx)
    .await?;

    if jobs.is_empty() {
        tx.rollback().await?;
        return Ok(0);
    }

    let count = jobs.len();

    // Marquer tout le batch comme `sending` en une seule requête
    let ids: Vec<Uuid> = jobs.iter().map(|j| j.id).collect();
    sqlx::query(
        "UPDATE mail_queue
         SET    status = 'sending', attempts = attempts + 1
         WHERE  id = ANY($1)",
    )
    .bind(&ids)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    // ── 2. Traiter tous les jobs en parallèle, concurrence limitée ────────────
    let semaphore = Arc::new(tokio::sync::Semaphore::new(concurrency));

    let mut futures: FuturesUnordered<_> = jobs
        .into_iter()
        .map(|job| {
            let sem = semaphore.clone();
            let state = state.clone();
            let templates = templates.clone();
            tokio::spawn(async move {
                let _permit = sem.acquire_owned().await;
                let id = job.id;
                let event_type = job.event_type.clone();
                let timer = DELIVERY_DURATION.start_timer();
                let outcome = process_job(&state, &templates, job).await;
                timer.observe_duration();
                (id, event_type, outcome)
            })
        })
        .collect();

    while let Some(result) = futures.next().await {
        let (id, event_type, outcome) = result.map_err(|e| anyhow::anyhow!("tokio join: {e}"))?;
        match outcome {
            Ok(message_id) => {
                DELIVERIES
                    .with_label_values(&[event_type.as_str(), "sent"])
                    .inc();
                match delete_sent(&state.db, id).await {
                    Ok(()) => tracing::info!(%id, %message_id, "Email livré ✓"),
                    Err(e) => tracing::error!(%id, error = %e, "Suppression BDD échouée"),
                }
            }
            Err(e) => {
                tracing::warn!(%id, error = %e, "Échec de livraison");
                match reschedule_or_fail(&state.db, id).await {
                    Ok(Retry::Scheduled) => {
                        DELIVERIES
                            .with_label_values(&[event_type.as_str(), "retry"])
                            .inc();
                    }
                    Ok(Retry::GaveUp) => {
                        DELIVERIES
                            .with_label_values(&[event_type.as_str(), "failed"])
                            .inc();
                    }
                    Err(e) => tracing::error!(%id, error = %e, "Replanification échouée"),
                }
            }
        }
    }

    Ok(count)
}

// ─── Traitement d'un job ──────────────────────────────────────────────────────

async fn process_job(
    state: &WorkerState,
    templates: &TemplateEngine,
    job: MailJob,
) -> Result<String> {
    // 1. récupérer le recipient
    let recipient = &job.recipient;

    // 2. Identifier l'événement NATS et vérifier qu'il est supporté
    let event = EmailEvent::from_subject(&job.event_type)
        .ok_or_else(|| anyhow::anyhow!("type d'événement inconnu : '{}'", job.event_type))?;

    // 4. Rendu des templates HTML + texte
    let vars = payload_to_map(&job.payload)?;
    let rendered = templates.render(&event, &vars)?;

    // 5. Construire le nom d'affichage du destinataire depuis le payload
    let display_name = build_display_name(&vars);
    let sender_name = event.sender_name(&vars, &state.config.smtp_from_name);
    let reply_to = event.reply_to(&vars);

    // 6. Construire le message MIME
    let msg = message::build(
        &state.config.smtp_from_email,
        &sender_name,
        recipient,
        display_name.as_deref(),
        reply_to,
        &rendered.subject,
        &rendered.html,
        &rendered.text,
        &state.config.dkim_domain,
    )?;

    // 7. Signer avec DKIM
    let signed = dkim::sign(
        &msg.raw,
        &state.config.dkim_private_key_pem,
        &state.config.dkim_domain,
        &state.config.dkim_selector,
    )?;

    // 8. Déterminer la liste des hôtes SMTP cibles
    //    - Mode relay (SMTP_HOST défini) : livraison directe via le smarthost (ex: Mailpit en dev)
    //    - Mode MTA direct : lookup DNS MX puis essai dans l'ordre de priorité
    let smtp_targets: Vec<(String, u16)> = match &state.config.smtp_relay_host {
        Some(relay_host) => {
            tracing::debug!(
                relay = %relay_host,
                port = state.config.smtp_relay_port,
                "Mode relay SMTP — bypass DNS MX"
            );
            vec![(relay_host.clone(), state.config.smtp_relay_port)]
        }
        None => {
            let domain = dns::domain_of(recipient)?;
            let mx_records = dns::resolve_mx(domain).await?;
            if mx_records.is_empty() {
                anyhow::bail!("Aucun MX trouvé pour {domain}");
            }
            mx_records.into_iter().map(|mx| (mx.host, 25)).collect()
        }
    };

    // 9. Livrer via SMTP en essayant les cibles dans l'ordre
    let mut last_err = None;
    for (host, port) in &smtp_targets {
        match smtp::deliver(
            host,
            *port,
            &state.config.smtp_from_email,
            recipient,
            &signed,
            &state.config.dkim_domain,
        )
        .await
        {
            Ok(()) => return Ok(msg.message_id),
            Err(e) => {
                tracing::warn!(%host, %port, error = %e, "Hôte SMTP suivant essayé");
                last_err = Some(e);
            }
        }
    }

    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("tous les hôtes SMTP ont échoué")))
}

// ─── Replanification / abandon ────────────────────────────────────────────────

/// Suite donnée à un échec de livraison.
enum Retry {
    /// Nouvelle tentative planifiée (backoff exponentiel).
    Scheduled,
    /// Abandon : `max_attempts` atteint, job passé en `failed`.
    GaveUp,
}

async fn reschedule_or_fail(db: &PgPool, id: Uuid) -> Result<Retry> {
    // Récupère le nombre de tentatives actuelles
    let row: (i32, i32) =
        sqlx::query_as("SELECT attempts, max_attempts FROM mail_queue WHERE id = $1")
            .bind(id)
            .fetch_one(db)
            .await?;
    let (attempts, max_attempts) = row;

    if attempts >= max_attempts {
        sqlx::query(
            "UPDATE mail_queue SET status = 'failed', error = 'max_attempts atteints' WHERE id = $1",
        )
        .bind(id)
        .execute(db)
        .await?;
        tracing::warn!(%id, "Job marqué failed (max_attempts atteints)");
        Ok(Retry::GaveUp)
    } else {
        // Backoff exponentiel : 2^attempts * 60s + jitter(0..60s)
        let delay_secs = backoff_secs(attempts);
        sqlx::query(
            "UPDATE mail_queue
             SET status = 'pending',
                 next_attempt_at = NOW() + make_interval(secs => $2)
             WHERE id = $1",
        )
        .bind(id)
        .bind(delay_secs as f64)
        .execute(db)
        .await?;
        tracing::debug!(%id, delay_secs, "Job replanifié");
        Ok(Retry::Scheduled)
    }
}

/// Formule de backoff : `min(2^n * 60, 86400) + jitter(0..60)` secondes
fn backoff_secs(attempts: i32) -> u64 {
    let base: u64 = 2u64
        .saturating_pow(attempts as u32)
        .saturating_mul(60)
        .min(86400); // max 24h
    let jitter = rand::rng().random_range(0..60u64);
    base + jitter
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn payload_to_map(value: &Value) -> Result<HashMap<String, Value>> {
    match value {
        Value::Object(map) => Ok(map.iter().map(|(k, v)| (k.clone(), v.clone())).collect()),
        _ => anyhow::bail!("payload JSONB n'est pas un objet"),
    }
}

/// Nom affiché du destinataire (en-tête `To`), d'après les champs de l'événement.
fn build_display_name(vars: &HashMap<String, Value>) -> Option<String> {
    let firstname = vars.get("firstname").and_then(|v| v.as_str());
    let name = vars.get("name").and_then(|v| v.as_str());
    match (firstname, name) {
        (Some(f), Some(n)) => Some(format!("{f} {n}")),
        (Some(f), None) => Some(f.to_string()),
        (None, Some(n)) => Some(n.to_string()),
        // Ni `firstname` ni `name` : `clientName`, si l'événement le porte.
        _ => vars
            .get("clientName")
            .and_then(|v| v.as_str())
            .map(str::to_string),
    }
}

// ─── Insertion dans la file ───────────────────────────────────────────────────

/// Insère un nouveau job dans `mail_queue`.
/// `recipient_enc` doit déjà être chiffré (produit par `crypto::encrypt`).
pub async fn enqueue(
    db: &PgPool,
    event_type: &str,
    recipient: &str,
    payload: &Value,
) -> Result<Uuid> {
    let id: (Uuid,) = sqlx::query_as(
        r#"
        INSERT INTO mail_queue (event_type, recipient, payload)
        VALUES ($1, $2, $3)
        RETURNING id
        "#,
    )
    .bind(event_type)
    .bind(recipient)
    .bind(payload)
    .fetch_one(db)
    .await?;

    Ok(id.0)
}

// ─── Corrélation bounce ───────────────────────────────────────────────────────

/// Met à jour le statut d'un job à `bounced` en corrélant via le `message_id`.
/// Insère également une ligne dans la table `bounces`.
pub async fn record_bounce(
    db: &PgPool,
    message_id: &str,
    bounce_type: &str,
    bounce_code: Option<i32>,
    raw_dsn: &str,
) -> Result<()> {
    // 1. Mettre à jour mail_queue
    sqlx::query("UPDATE mail_queue SET status = 'bounced' WHERE message_id = $1")
        .bind(message_id)
        .execute(db)
        .await?;

    // 2. Insérer dans bounces
    sqlx::query(
        r#"
        INSERT INTO bounces (message_id, bounce_type, bounce_code, raw_dsn)
        VALUES ($1, $2, $3, $4)
        "#,
    )
    .bind(message_id)
    .bind(bounce_type)
    .bind(bounce_code)
    .bind(raw_dsn)
    .execute(db)
    .await?;

    Ok(())
}

// ─── Correction statut sending → pending au démarrage ─────────────────────────
// ─── Suppression BDD après envoi ─────────────────────────────────────────────

/// Supprime la ligne de `mail_queue` après livraison réussie : la BDD ne
/// conserve que les jobs en attente, en erreur ou en cours de retry.
async fn delete_sent(db: &PgPool, id: Uuid) -> Result<()> {
    sqlx::query("DELETE FROM mail_queue WHERE id = $1")
        .bind(id)
        .execute(db)
        .await
        .context("DELETE mail_queue après envoi")?;
    Ok(())
}

// ─── Correction statut sending → pending au démarrage ─────────────────────────
/// Remet les jobs bloqués en `sending` à `pending` au redémarrage du service.
/// (Prévu pour les crashs en cours de traitement)
pub async fn reset_stale_sending(db: &PgPool) -> Result<u64> {
    let res = sqlx::query(
        "UPDATE mail_queue SET status = 'pending', next_attempt_at = NOW()
         WHERE status = 'sending'",
    )
    .execute(db)
    .await?;
    Ok(res.rows_affected())
}

#[cfg(test)]
mod extra_tests {
    use super::*;
    use serde_json::json;

    fn vars(value: Value) -> HashMap<String, Value> {
        payload_to_map(&value).unwrap()
    }

    #[test]
    fn backoff_doubles_from_two_minutes_with_under_a_minute_of_jitter() {
        for (attempts, base) in [(0, 60), (1, 120), (2, 240), (3, 480), (4, 960)] {
            for _ in 0..50 {
                let delay = backoff_secs(attempts);
                assert!(
                    (base..base + 60).contains(&delay),
                    "tentative {attempts} : {delay}"
                );
            }
        }
    }

    #[test]
    fn backoff_is_capped_at_24h_and_never_overflows() {
        for attempts in [11, 20, 63, 64, 1000, i32::MAX] {
            let delay = backoff_secs(attempts);
            assert!(
                (86_400..86_460).contains(&delay),
                "tentative {attempts} : {delay}"
            );
        }
    }

    #[test]
    fn only_a_json_object_is_a_valid_payload() {
        assert!(payload_to_map(&json!({ "a": 1 })).is_ok());
        for bad in [json!([1]), json!("x"), json!(null), json!(3)] {
            assert!(payload_to_map(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn display_name_prefers_firstname_and_name_then_falls_back_to_client_name() {
        assert_eq!(
            build_display_name(&vars(json!({ "firstname": "Camille", "name": "Dupont" })))
                .as_deref(),
            Some("Camille Dupont")
        );
        assert_eq!(
            build_display_name(&vars(json!({ "firstname": "Camille" }))).as_deref(),
            Some("Camille")
        );
        assert_eq!(
            build_display_name(&vars(json!({ "name": "Dupont" }))).as_deref(),
            Some("Dupont")
        );
        assert_eq!(
            build_display_name(&vars(json!({ "clientName": "Mme Dubois" }))).as_deref(),
            Some("Mme Dubois")
        );
        assert_eq!(build_display_name(&vars(json!({ "name": 3 }))), None);
        assert_eq!(build_display_name(&vars(json!({}))), None);
    }
}
