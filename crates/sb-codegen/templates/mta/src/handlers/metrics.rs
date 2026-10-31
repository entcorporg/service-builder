use std::sync::Arc;

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use sha2::{Digest, Sha256};

use crate::middlewares::metrics::{is_authorized, render};

/// Contrôle d'accès de GET /metrics, construit une fois depuis la config.
#[derive(Clone, Debug)]
pub struct MetricsAccess {
    pub token_hash: Option<[u8; 32]>,
    pub allow_open: bool,
}

impl MetricsAccess {
    pub fn new(token: Option<&str>, allow_open: bool) -> Self {
        Self {
            token_hash: token.map(|t| Sha256::digest(t.as_bytes()).into()),
            allow_open,
        }
    }
}

/// Endpoint de scraping Prometheus. Protégé par `METRICS_TOKEN` (Bearer),
/// fail-closed si le token est absent sauf `METRICS_ALLOW_OPEN=true`.
pub async fn metrics(State(access): State<Arc<MetricsAccess>>, headers: HeaderMap) -> Response {
    if !is_authorized(&headers, access.token_hash.as_ref(), access.allow_open) {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    let (content_type, body) = render();
    ([(header::CONTENT_TYPE, content_type)], body).into_response()
}
