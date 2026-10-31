use anyhow::{Context, Result};
use std::env;

/// Configuration du service MTA.
///
/// Chargée depuis les variables d'environnement. Le `.env` du crate est lu en
/// amont dans `main` (`dotenvy::from_path`).
#[derive(Clone, Debug)]
pub struct Config {
    // ── Serveur HTTP (health / readiness / metrics) ───────────────────────────
    pub server_port: u16,
    /// Jeton Bearer attendu sur GET /metrics (METRICS_TOKEN).
    pub metrics_token: Option<String>,
    /// Dérogation explicite : /metrics ouvert sans jeton (METRICS_ALLOW_OPEN).
    pub metrics_allow_open: bool,

    // ── Base de données (file mail_queue + bounces) ───────────────────────────
    pub database_url: String,

    // ── Event bus (Kafka) ─────────────────────────────────────────────────────
    /// `KAFKA_BROKERS` (défaut `localhost:9092`).
    pub brokers: String,

    // ── Expéditeur ────────────────────────────────────────────────────────────
    pub smtp_from_email: String,
    pub smtp_from_name: String,

    // ── Relay SMTP (smarthost) ────────────────────────────────────────────────
    /// Si défini, tous les emails sont livrés via ce relay (ex: Mailpit en dev)
    /// au lieu d'un lookup DNS MX direct. Défini via SMTP_HOST / SMTP_PORT.
    pub smtp_relay_host: Option<String>,
    pub smtp_relay_port: u16,

    // ── DKIM ──────────────────────────────────────────────────────────────────
    /// Contenu PEM de la clé privée RSA ou ED25519.
    pub dkim_private_key_pem: String,
    /// Sélecteur DNS (ex: "mail", "2026").
    pub dkim_selector: String,
    /// Domaine signataire (ex: "example.com").
    pub dkim_domain: String,

    // ── Serveur SMTP entrant (bounces) ────────────────────────────────────────
    /// Port d'écoute pour les NDR/DSN entrants (défaut : 2525 en dev, 25 en prod).
    pub bounce_listen_port: u16,
    /// Nom d'hôte annoncé par le serveur SMTP entrant (banner 220).
    pub bounce_hostname: String,

    // ── Worker ────────────────────────────────────────────────────────────────
    /// Nombre de jobs chargés par itération (WORKER_BATCH_SIZE, défaut 500).
    pub worker_batch_size: u32,
    /// Nombre de jobs traités en parallèle (WORKER_CONCURRENCY, défaut 50).
    pub worker_concurrency: usize,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        // ── Clé privée DKIM : chemin de fichier OU valeur PEM directe ──────────
        let dkim_private_key_pem = if let Ok(path) = env::var("DKIM_PRIVATE_KEY_PATH") {
            std::fs::read_to_string(&path)
                .with_context(|| format!("DKIM : lecture de la clé depuis '{path}'"))?
        } else {
            env::var("DKIM_PRIVATE_KEY_PEM")
                .context("DKIM_PRIVATE_KEY_PEM ou DKIM_PRIVATE_KEY_PATH requis")?
        };

        Ok(Config {
            server_port: env::var("SERVER_PORT")
                .unwrap_or_else(|_| "::SERVER_PORT::".into())
                .parse()
                .context("SERVER_PORT invalide")?,
            metrics_token: env::var("METRICS_TOKEN").ok().filter(|s| !s.is_empty()),
            metrics_allow_open: env::var("METRICS_ALLOW_OPEN")
                .map(|v| v == "true" || v == "1")
                .unwrap_or(false),

            database_url: env::var("::DATABASE_ENV::")
                .unwrap_or_else(|_| "::DATABASE_DEFAULT::".into()),

            brokers: env::var("KAFKA_BROKERS").unwrap_or_else(|_| "localhost:9092".into()),

            smtp_from_email: env::var("SMTP_FROM_EMAIL").context("SMTP_FROM_EMAIL manquant")?,
            smtp_from_name: env::var("SMTP_FROM_NAME").context("SMTP_FROM_NAME manquant")?,

            smtp_relay_host: env::var("SMTP_HOST").ok(),
            smtp_relay_port: env::var("SMTP_PORT")
                .unwrap_or_else(|_| "25".into())
                .parse()
                .context("SMTP_PORT invalide")?,

            dkim_private_key_pem,
            dkim_selector: env::var("DKIM_SELECTOR").context("DKIM_SELECTOR manquant")?,
            dkim_domain: env::var("DKIM_DOMAIN").context("DKIM_DOMAIN manquant")?,

            bounce_listen_port: env::var("BOUNCE_LISTEN_PORT")
                .unwrap_or_else(|_| "2525".into())
                .parse()
                .context("BOUNCE_LISTEN_PORT invalide")?,
            bounce_hostname: env::var("BOUNCE_HOSTNAME").unwrap_or_else(|_| "localhost".into()),

            worker_batch_size: env::var("WORKER_BATCH_SIZE")
                .unwrap_or_else(|_| "500".into())
                .parse()
                .context("WORKER_BATCH_SIZE invalide")?,
            worker_concurrency: env::var("WORKER_CONCURRENCY")
                .unwrap_or_else(|_| "50".into())
                .parse()
                .context("WORKER_CONCURRENCY invalide")?,
        })
    }

    pub fn bounce_addr(&self) -> String {
        format!("0.0.0.0:{}", self.bounce_listen_port)
    }
}
