use anyhow::{Context, Result};
use realtime_service::{config::Config, server};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::from_path(concat!(env!("CARGO_MANIFEST_DIR"), "/.env")).ok();
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "realtime_service=info,tower_http=info".into()))
        .with(tracing_subscriber::fmt::layer())
        .init();
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "realtime starting");

    let config = Config::from_env().context("configuration")?;
    server::run(config).await
}
