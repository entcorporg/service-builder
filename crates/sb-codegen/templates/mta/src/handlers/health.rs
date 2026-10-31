use axum::{Json, extract::State, http::StatusCode};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::time::Duration;

const READYZ_DEP_TIMEOUT: Duration = Duration::from_secs(2);

/// Liveness : le process répond. Aucune dépendance externe — un échec doit
/// déclencher un restart du pod, pas un retrait du load balancer.
pub async fn livez() -> Json<Value> {
    Json(json!({"status": "alive"}))
}

/// Alias historique de `livez` (healthcheck Docker `curl /health`).
pub async fn health() -> Json<Value> {
    Json(json!({"status": "ok"}))
}

/// Readiness : sonde réelle de Postgres (file `mail_queue` / `bounces`), seule
/// dépendance synchrone critique. Kafka volontairement exclu : le consumer
/// rattrape son retard tout seul.
pub async fn readyz(State(db): State<PgPool>) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    match tokio::time::timeout(READYZ_DEP_TIMEOUT, sqlx::query("SELECT 1").execute(&db)).await {
        Ok(Ok(_)) => Ok(Json(json!({"status": "ready"}))),
        Ok(Err(e)) => {
            tracing::warn!("readyz: postgres error: {e}");
            Err(unavailable("database"))
        }
        Err(_) => {
            tracing::warn!("readyz: postgres timeout (> {READYZ_DEP_TIMEOUT:?})");
            Err(unavailable("database timeout"))
        }
    }
}

fn unavailable(dep: &str) -> (StatusCode, Json<Value>) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({"status": "unavailable", "dependency": dep})),
    )
}
