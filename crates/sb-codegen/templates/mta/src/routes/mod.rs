use std::sync::Arc;

use crate::handlers::{self, metrics::MetricsAccess};
use crate::middlewares::metrics::metrics_middleware;
use axum::{Router, middleware, routing::get};
use sqlx::PgPool;

/// Routeur HTTP minimal : le MTA n'expose aucune route métier (il consomme
/// Kafka et livre par SMTP). Seules les sondes de santé et GET /metrics sont
/// publiées, alignées sur svc-auth.
pub fn build(db: PgPool, metrics: MetricsAccess) -> Router {
    let metrics_router = Router::new()
        .route("/metrics", get(handlers::metrics::metrics))
        .with_state(Arc::new(metrics));

    Router::new()
        .route("/health", get(handlers::health::health))
        .route("/livez", get(handlers::health::livez))
        .route("/readyz", get(handlers::health::readyz))
        .with_state(db)
        .merge(metrics_router)
        .layer(middleware::from_fn(metrics_middleware))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode, header},
    };
    use sqlx::postgres::PgPoolOptions;
    use tower::ServiceExt;

    /// Pool paresseux : aucune connexion n'est ouverte tant qu'aucune requête
    /// SQL ne part, /livez et /metrics n'en font pas.
    fn app() -> Router {
        let db = PgPoolOptions::new()
            .connect_lazy("postgres://test@localhost/test")
            .expect("pool paresseux");
        build(db, MetricsAccess::new(Some("scrape-token"), false))
    }

    fn get(uri: &str, token: Option<&str>) -> Request<Body> {
        let mut req = Request::get(uri);
        if let Some(t) = token {
            req = req.header(header::AUTHORIZATION, format!("Bearer {t}"));
        }
        req.body(Body::empty()).unwrap()
    }

    #[tokio::test]
    async fn metrics_requires_token() {
        let res = app().oneshot(get("/metrics", None)).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

        let res = app().oneshot(get("/metrics", Some("autre"))).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn metrics_exposes_http_counters() {
        let app = app();
        app.clone().oneshot(get("/livez", None)).await.unwrap();

        let res = app
            .oneshot(get("/metrics", Some("scrape-token")))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let body = String::from_utf8(body.to_vec()).unwrap();
        // Série neuve : exposée tout de suite (à 0, la mesure est différée).
        assert!(body.contains(r#"http_requests_total{method="GET",path="/livez",status="200"}"#));
        // Feature `process` du crate prometheus : mémoire/CPU du process.
        assert!(body.contains("process_resident_memory_bytes"));
    }
}
