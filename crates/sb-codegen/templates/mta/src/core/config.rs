use anyhow::{Context, Result};
use std::{collections::HashMap, env};

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

    // ── Expéditeurs supplémentaires (`senders` du projet) ─────────────────────
    /// Par identifiant : adresse, nom et identité DKIM propres. Les clés viennent de
    /// `SENDER_<ID>_DKIM_PRIVATE_KEY_PATH` ou `SENDER_<ID>_DKIM_PRIVATE_KEY_PEM`.
    pub senders: HashMap<String, Identity>,
}

/// Ce qui sort dans un mail : l'adresse `From`, le nom affiché et l'identité DKIM qui signe.
#[derive(Clone, Debug)]
pub struct Identity {
    pub email: String,
    pub name: String,
    pub dkim_private_key_pem: String,
    pub dkim_selector: String,
    pub dkim_domain: String,
}

/// Un expéditeur demandé par une règle n'existe pas : échec définitif, jamais de repli sur l'adresse du service
/// (un mail ne part pas sous une identité que le projet n'a pas choisie).
#[derive(Debug)]
pub struct UnknownSender(pub String);

impl std::fmt::Display for UnknownSender {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "expéditeur inconnu : '{}'", self.0)
    }
}

impl std::error::Error for UnknownSender {}

impl Config {
    /// L'identité d'un mail : celle de l'expéditeur nommé par la règle, ou celle du service sans règle d'expéditeur.
    pub fn identity(&self, sender: Option<&str>) -> Result<Identity, UnknownSender> {
        match sender {
            None => Ok(Identity {
                email: self.smtp_from_email.clone(),
                name: self.smtp_from_name.clone(),
                dkim_private_key_pem: self.dkim_private_key_pem.clone(),
                dkim_selector: self.dkim_selector.clone(),
                dkim_domain: self.dkim_domain.clone(),
            }),
            Some(id) => self
                .senders
                .get(id)
                .cloned()
                .ok_or_else(|| UnknownSender(id.to_owned())),
        }
    }

    fn load_senders(default_name: &str) -> Result<HashMap<String, Identity>> {
        let mut senders = HashMap::new();
        for decl in crate::mta_rules::SENDERS {
            let var = format!("SENDER_{}_DKIM_PRIVATE_KEY", decl.id.to_uppercase());
            let pem = if let Ok(path) = env::var(format!("{var}_PATH")) {
                std::fs::read_to_string(&path).with_context(|| {
                    format!("expéditeur '{}' : lecture de la clé depuis '{path}'", decl.id)
                })?
            } else {
                env::var(format!("{var}_PEM")).with_context(|| {
                    format!("expéditeur '{}' : {var}_PATH ou {var}_PEM requis", decl.id)
                })?
            };
            senders.insert(
                decl.id.to_owned(),
                Identity {
                    email: decl.email.to_owned(),
                    name: decl.name.unwrap_or(default_name).to_owned(),
                    dkim_private_key_pem: pem,
                    dkim_selector: decl.dkim_selector.to_owned(),
                    dkim_domain: decl.dkim_domain.to_owned(),
                },
            );
        }
        Ok(senders)
    }

    pub fn from_env() -> Result<Self> {
        // ── Clé privée DKIM : chemin de fichier OU valeur PEM directe ──────────
        let dkim_private_key_pem = if let Ok(path) = env::var("DKIM_PRIVATE_KEY_PATH") {
            std::fs::read_to_string(&path)
                .with_context(|| format!("DKIM : lecture de la clé depuis '{path}'"))?
        } else {
            env::var("DKIM_PRIVATE_KEY_PEM")
                .context("DKIM_PRIVATE_KEY_PEM ou DKIM_PRIVATE_KEY_PATH requis")?
        };

        let smtp_from_name = env::var("SMTP_FROM_NAME").context("SMTP_FROM_NAME manquant")?;
        let senders = Self::load_senders(&smtp_from_name)?;

        Ok(Config {
            senders,
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
            smtp_from_name,

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

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> Config {
        let mut senders = HashMap::new();
        senders.insert(
            "achats".to_owned(),
            Identity {
                email: "achats@mon-entreprise.fr".into(),
                name: "Achats".into(),
                dkim_private_key_pem: "achats-key".into(),
                dkim_selector: "achats".into(),
                dkim_domain: "mon-entreprise.fr".into(),
            },
        );
        Config {
            server_port: 0,
            metrics_token: None,
            metrics_allow_open: false,
            database_url: String::new(),
            brokers: String::new(),
            smtp_from_email: "no-reply@platform.test".into(),
            smtp_from_name: "Platform".into(),
            smtp_relay_host: None,
            smtp_relay_port: 25,
            dkim_private_key_pem: "platform-key".into(),
            dkim_selector: "mail".into(),
            dkim_domain: "platform.test".into(),
            bounce_listen_port: 0,
            bounce_hostname: String::new(),
            worker_batch_size: 1,
            worker_concurrency: 1,
            senders,
        }
    }

    #[test]
    fn without_a_sender_the_mail_leaves_from_the_service_address() {
        let id = config().identity(None).unwrap();
        assert_eq!(id.email, "no-reply@platform.test");
        assert_eq!(id.dkim_private_key_pem, "platform-key");
        assert_eq!(id.dkim_domain, "platform.test");
    }

    #[test]
    fn a_named_sender_brings_its_own_address_and_dkim_identity() {
        let id = config().identity(Some("achats")).unwrap();
        assert_eq!(id.email, "achats@mon-entreprise.fr");
        assert_eq!(id.name, "Achats");
        assert_eq!(id.dkim_private_key_pem, "achats-key");
        assert_eq!(id.dkim_selector, "achats");
        assert_eq!(id.dkim_domain, "mon-entreprise.fr");
    }

    #[test]
    fn an_unknown_sender_is_refused_and_never_replaced_by_the_default() {
        let err = config().identity(Some("ghost")).unwrap_err();
        assert_eq!(err.0, "ghost");
        assert!(crate::mta::queue_worker::is_permanent(&err.into()));
    }
}
