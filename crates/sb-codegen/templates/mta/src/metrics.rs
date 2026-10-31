//! Métriques Prometheus métier du MTA : consumer Kafka, livraison SMTP,
//! bounces et état de la file `mail_queue`. Exposées sur GET /metrics avec les
//! métriques HTTP (registre global). Labels à valeurs bornées uniquement
//! (topics, types d'événements, statuts) : jamais d'adresse e-mail.

use std::sync::LazyLock;

use prometheus::{
    Histogram, IntCounterVec, IntGaugeVec, register_histogram, register_int_counter_vec,
    register_int_gauge_vec,
};
use sqlx::PgPool;

/// Messages Kafka traités, par topic et issue (`enqueued`, `duplicate`,
/// `skipped`, `invalid`, `error`).
pub static KAFKA_MESSAGES: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "mta_kafka_messages_total",
        "Kafka messages processed by topic and outcome",
        &["topic", "outcome"]
    )
    .expect("register mta_kafka_messages_total")
});

/// Tentatives de livraison SMTP, par type d'e-mail et issue (`sent`, `retry`,
/// `failed` = abandon après max_attempts).
pub static DELIVERIES: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "mta_deliveries_total",
        "SMTP delivery attempts by event type and outcome",
        &["event_type", "outcome"]
    )
    .expect("register mta_deliveries_total")
});

/// Durée d'une tentative de livraison (rendu, DKIM, résolution MX, SMTP).
pub static DELIVERY_DURATION: LazyLock<Histogram> = LazyLock::new(|| {
    register_histogram!(
        "mta_delivery_duration_seconds",
        "Duration of one SMTP delivery attempt",
        vec![0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0]
    )
    .expect("register mta_delivery_duration_seconds")
});

/// Bounces (NDR/DSN) reçus, par type (`hard`, `soft`...).
pub static BOUNCES: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "mta_bounces_total",
        "Bounces received by bounce type",
        &["bounce_type"]
    )
    .expect("register mta_bounces_total")
});

/// Nombre de jobs dans `mail_queue` par statut (les jobs livrés sont
/// supprimés, `sent` reste donc à 0).
pub static QUEUE_JOBS: LazyLock<IntGaugeVec> = LazyLock::new(|| {
    register_int_gauge_vec!(
        "mta_queue_jobs",
        "Jobs currently in mail_queue by status",
        &["status"]
    )
    .expect("register mta_queue_jobs")
});

/// Valeurs de l'enum PostgreSQL `mail_status`.
const STATUSES: &[&str] = &["pending", "sending", "sent", "failed", "bounced"];

/// Crée à 0 toutes les séries métier aux labels connus d'avance. Sans ça, une
/// série naîtrait à sa première mesure et Prometheus la découvrirait déjà à 1
/// (ou plus) : `increase()` / `rate()` ne compteraient jamais ces premières
/// mesures. `topics` sert aussi de liste des types d'e-mails (`event_type` =
/// topic Kafka d'origine).
pub fn init(topics: &[&str]) {
    for topic in topics.iter().copied().chain(["other"]) {
        for outcome in ["enqueued", "duplicate", "skipped", "invalid", "error"] {
            KAFKA_MESSAGES.with_label_values(&[topic, outcome]);
        }
    }
    for &event_type in topics {
        for outcome in ["sent", "retry", "failed"] {
            DELIVERIES.with_label_values(&[event_type, outcome]);
        }
    }
    // Valeurs renvoyées par `bounce::server::detect_bounce_type`.
    for bounce_type in ["permanent", "transient"] {
        BOUNCES.with_label_values(&[bounce_type]);
    }
    LazyLock::force(&DELIVERY_DURATION);
    for status in STATUSES {
        QUEUE_JOBS.with_label_values(&[status]);
    }
}

/// Rafraîchit `mta_queue_jobs` depuis la base. Les statuts absents du
/// résultat sont remis à 0 pour ne pas garder une ancienne valeur.
pub async fn refresh_queue_jobs(db: &PgPool) -> sqlx::Result<()> {
    let rows: Vec<(String, i64)> =
        sqlx::query_as("SELECT status::text, COUNT(*) FROM mail_queue GROUP BY status")
            .fetch_all(db)
            .await?;
    for status in STATUSES {
        let count = rows
            .iter()
            .find(|(s, _)| s == status)
            .map_or(0, |(_, c)| *c);
        QUEUE_JOBS.with_label_values(&[status]).set(count);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_exposes_known_series_at_zero() {
        init(&["auth.user.registered"]);
        assert_eq!(
            DELIVERIES
                .with_label_values(&["auth.user.registered", "sent"])
                .get(),
            0
        );
        let families = prometheus::gather();
        let names: Vec<_> = families.iter().map(|f| f.name()).collect();
        for name in [
            "mta_kafka_messages_total",
            "mta_deliveries_total",
            "mta_bounces_total",
            "mta_delivery_duration_seconds",
            "mta_queue_jobs",
        ] {
            assert!(names.contains(&name), "{name} absent de /metrics");
        }
    }
}
