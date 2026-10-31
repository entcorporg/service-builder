//! Instrumentation HTTP Prometheus des services générés.
//!
//! Mêmes noms de métriques (`http_requests_total`...) et mêmes labels dans
//! chaque service : Prometheus ajoute le label `service` au scrape, un seul
//! dashboard Grafana couvre donc tous les services. Fichier
//! dupliqué dans chaque service : toute modification se reporte dans chacun.

use axum::{
    extract::{MatchedPath, Request},
    middleware::Next,
    response::Response,
};
use prometheus::{
    Encoder, HistogramVec, IntCounterVec, IntGauge, TextEncoder, register_histogram_vec,
    register_int_counter_vec, register_int_gauge,
};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, PoisonError};
use subtle::ConstantTimeEq;
use tokio::time::{Duration, Instant};

/// Compteurs `http_*`, registre global, construits une seule fois.
static METRICS: LazyLock<HttpMetrics> = LazyLock::new(HttpMetrics::register);

/// Délai pendant lequel les mesures d'une série neuve sont différées.
///
/// Une série (méthode, route, statut) n'existe qu'à partir de sa première
/// requête, et Prometheus ne la découvre qu'au scrape suivant : si 2 requêtes
/// arrivent entre deux scrapes, la série apparaît directement à 2, sans point
/// à 0, et `increase()` / `rate()` ne comptent jamais ces 2 requêtes. La série
/// est donc créée à 0 tout de suite, et ses mesures ne sont appliquées qu'après
/// ce délai, une fois que Prometheus l'a vue à 0. Doit dépasser deux fois
/// l'intervalle de scrape (30 s, deploy/infra/monitoring/prometheus.yaml).
const NEW_SERIES_DELAY: Duration = Duration::from_secs(65);

pub async fn metrics_middleware(request: Request, next: Next) -> Response {
    METRICS.track(request, next).await
}

/// Compteurs HTTP d'un service. À construire UNE fois (LazyLock) : le
/// registre Prometheus est global, un double register paniquerait.
pub struct HttpMetrics {
    requests: IntCounterVec,
    duration: HistogramVec,
    in_flight: IntGauge,
    /// Instant de création de chaque série (méthode, route, statut).
    born: Mutex<HashMap<(String, String, String), Instant>>,
}

impl HttpMetrics {
    pub fn register() -> Self {
        Self {
            requests: register_int_counter_vec!(
                "http_requests_total",
                "HTTP requests by method, matched route and status code",
                &["method", "path", "status"]
            )
            .expect("register requests counter"),
            duration: register_histogram_vec!(
                "http_request_duration_seconds",
                "HTTP request latency by method, matched route and status code",
                &["method", "path", "status"],
                vec![
                    0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0
                ]
            )
            .expect("register duration histogram"),
            in_flight: register_int_gauge!(
                "http_requests_in_flight",
                "HTTP requests currently being processed"
            )
            .expect("register in-flight gauge"),
            born: Mutex::new(HashMap::new()),
        }
    }

    /// Corps du middleware axum. MatchedPath (template de route, ex:
    /// /me/profil) plutôt que l'URI brute : cardinalité bornée, pas
    /// d'explosion de labels Prometheus.
    pub async fn track(&'static self, request: Request, next: Next) -> Response {
        let path = request
            .extensions()
            .get::<MatchedPath>()
            .map(|p| p.as_str().to_owned())
            .unwrap_or_else(|| "unmatched".to_owned());
        let method = request.method().to_string();

        self.in_flight.inc();
        let start = Instant::now();
        let response = next.run(request).await;
        self.in_flight.dec();

        let status = response.status().as_u16().to_string();
        self.record(method, path, status, start.elapsed().as_secs_f64());

        response
    }

    /// Compte une requête. Pour une série née il y a moins de
    /// `NEW_SERIES_DELAY`, la mesure est différée (voir la constante).
    fn record(&'static self, method: String, path: String, status: String, seconds: f64) {
        let now = Instant::now();
        let born = *self
            .born
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry((method.clone(), path.clone(), status.clone()))
            .or_insert_with(|| {
                // Séries créées à 0 : visibles telles quelles au prochain scrape.
                let labels = [method.as_str(), path.as_str(), status.as_str()];
                self.requests.with_label_values(&labels);
                self.duration.with_label_values(&labels);
                now
            });

        let apply = move || {
            let labels = [method.as_str(), path.as_str(), status.as_str()];
            self.requests.with_label_values(&labels).inc();
            self.duration.with_label_values(&labels).observe(seconds);
        };
        let ready_at = born + NEW_SERIES_DELAY;
        if ready_at <= now {
            apply();
        } else {
            tokio::spawn(async move {
                tokio::time::sleep_until(ready_at).await;
                apply();
            });
        }
    }
}

/// Contrôle d'accès à GET /metrics : Bearer comparé en temps constant au hash
/// SHA-256 attendu. `expected_hash = None` (METRICS_TOKEN absent) → **fail-closed**
/// (refus), sauf dérogation explicite `allow_open` (METRICS_ALLOW_OPEN=true). Un
/// oubli de token n'expose donc jamais les compteurs en silence.
pub fn is_authorized(
    headers: &axum::http::HeaderMap,
    expected_hash: Option<&[u8; 32]>,
    allow_open: bool,
) -> bool {
    let Some(expected) = expected_hash else {
        return allow_open;
    };
    let Some(token) = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
    else {
        return false;
    };
    let provided_hash: [u8; 32] = Sha256::digest(token.as_bytes()).into();
    provided_hash.ct_eq(expected).into()
}

/// Sérialise le registre Prometheus global au format texte d'exposition.
pub fn render() -> (&'static str, Vec<u8>) {
    let mut buffer = Vec::new();
    TextEncoder::new()
        .encode(&prometheus::gather(), &mut buffer)
        .unwrap_or_default();
    ("text/plain; version=0.0.4; charset=utf-8", buffer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderValue, header};

    fn hash(token: &str) -> [u8; 32] {
        Sha256::digest(token.as_bytes()).into()
    }

    fn bearer(token: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
        );
        headers
    }

    #[tokio::test(start_paused = true)]
    async fn new_series_starts_at_zero_then_catches_up() {
        let labels = ["GET", "/test/new-series", "200"];
        let count = || METRICS.requests.with_label_values(&labels).get();
        let record = || {
            METRICS.record(
                labels[0].to_owned(),
                labels[1].to_owned(),
                labels[2].to_owned(),
                0.01,
            )
        };

        // Série neuve : exposée à 0, les 2 requêtes sont différées.
        record();
        record();
        assert_eq!(count(), 0);

        tokio::time::sleep(NEW_SERIES_DELAY + Duration::from_secs(1)).await;
        assert_eq!(count(), 2);

        // Série déjà vue par Prometheus : comptée immédiatement.
        record();
        assert_eq!(count(), 3);
    }

    #[test]
    fn no_token_fails_closed_by_default() {
        // METRICS_TOKEN absent + pas de dérogation → refus.
        assert!(!is_authorized(&HeaderMap::new(), None, false));
    }

    #[test]
    fn no_token_open_only_with_explicit_optin() {
        // METRICS_ALLOW_OPEN=true → ouvert (dev / réseau cloisonné).
        assert!(is_authorized(&HeaderMap::new(), None, true));
    }

    #[test]
    fn correct_token_accepted() {
        let expected = hash("scrape-token");
        assert!(is_authorized(
            &bearer("scrape-token"),
            Some(&expected),
            false
        ));
    }

    #[test]
    fn wrong_or_missing_token_rejected() {
        let expected = hash("scrape-token");
        assert!(!is_authorized(&bearer("autre"), Some(&expected), false));
        assert!(!is_authorized(&HeaderMap::new(), Some(&expected), false));
    }
}
