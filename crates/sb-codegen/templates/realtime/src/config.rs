use std::{net::IpAddr, time::Duration};

use anyhow::{Context, Result};

/// Configuration, from the environment (`.env` is read by `main`).
#[derive(Clone, Debug)]
pub struct Config {
    pub app_env: String,
    pub address: IpAddr,
    pub port: u16,
    /// Origins that may open the stream with the cookie. Never `*`: the browser refuses credentials with it.
    pub cors_allowed_origins: Vec<String>,
    /// The key-value server (`DRAGONFLY_URL`): the channel the events come on and the traces of the sessions. DragonflyDB,
    /// Valkey or Redis: anything that speaks the Redis protocol.
    pub dragonfly_url: String,
    /// The public keys of the access tokens (`JWKS_URL`).
    pub jwks_url: String,
    /// Streams this instance holds (`REALTIME_MAX_CONNECTIONS`). The memory of the process is bounded by it: size the
    /// container for this many streams and a new one beyond it is refused with a 503.
    pub max_connections: usize,
    /// Streams of one account (`REALTIME_MAX_PER_ACCOUNT`).
    pub max_per_account: usize,
    /// Frames waiting in the queue of a stream (`REALTIME_QUEUE`) before it is called too slow and closed. It is a number of
    /// places, not memory set aside: a queue that is empty costs the same whatever its size. It must hold a burst (a bulk
    /// import publishes hundreds of events for the same company at once) while the writers get their turn.
    pub queue: usize,
    /// Time over which the open streams are told to read their state again after the channel was lost
    /// (`REALTIME_RESYNC_SPREAD_SECS`): their browsers do not all call the API at the same moment.
    pub resync_spread: Duration,
    /// How long a stream is kept when its session cannot be checked (`REALTIME_RENEW_GRACE_SECS`).
    pub renew_grace_secs: i64,
    pub metrics_token: Option<String>,
    pub metrics_allow_open: bool,
}

fn env_non_empty(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

fn number<T: std::str::FromStr>(name: &str, default: T) -> Result<T> {
    match env_non_empty(name) {
        Some(value) => value.trim().parse().map_err(|_| anyhow::anyhow!("{name} is not valid: {value:?}")),
        None => Ok(default),
    }
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let app_env = env_non_empty("APP_ENV").unwrap_or_else(|| "local".into());
        let production = app_env == "production";

        // In production there is no default: a wrong server or a wrong set of keys would refuse every stream.
        let required = |name: &str, dev_default: &str| -> Result<String> {
            match env_non_empty(name) {
                Some(value) => Ok(value),
                None if production => anyhow::bail!("{name} is required in production"),
                None => Ok(dev_default.to_string()),
            }
        };

        let mut cors_allowed_origins: Vec<String> = env_non_empty("CORS_ALLOWED_ORIGINS")
            .map(|v| v.split(',').map(|o| o.trim().to_string()).filter(|o| !o.is_empty()).collect())
            .unwrap_or_default();
        if cors_allowed_origins.is_empty() {
            if production {
                anyhow::bail!("CORS_ALLOWED_ORIGINS is required in production");
            }
            cors_allowed_origins = vec!["http://localhost:5173".into()];
        }

        let config = Self {
            address: env_non_empty("SERVER_HOST").unwrap_or_else(|| "0.0.0.0".into()).parse().context("SERVER_HOST is not valid")?,
            port: number("SERVER_PORT", ::SERVER_PORT::)?,
            cors_allowed_origins,
            dragonfly_url: required("DRAGONFLY_URL", "redis://localhost:6379")?,
            jwks_url: required("JWKS_URL", "http://localhost:3000/.well-known/jwks.json")?,
            max_connections: number("REALTIME_MAX_CONNECTIONS", 60_000)?,
            max_per_account: number("REALTIME_MAX_PER_ACCOUNT", 20)?,
            queue: number("REALTIME_QUEUE", 256)?,
            resync_spread: Duration::from_secs(number("REALTIME_RESYNC_SPREAD_SECS", 10)?),
            renew_grace_secs: number("REALTIME_RENEW_GRACE_SECS", 120)?,
            metrics_token: env_non_empty("METRICS_TOKEN"),
            metrics_allow_open: env_non_empty("METRICS_ALLOW_OPEN").is_some_and(|v| v == "true" || v == "1"),
            app_env,
        };
        anyhow::ensure!(config.max_connections > 0, "REALTIME_MAX_CONNECTIONS must be at least 1");
        anyhow::ensure!(config.max_per_account > 0, "REALTIME_MAX_PER_ACCOUNT must be at least 1");
        anyhow::ensure!(config.queue > 0, "REALTIME_QUEUE must be at least 1");
        Ok(config)
    }

    /// A configuration for tests: nothing read from the environment.
    pub fn for_tests() -> Self {
        Self {
            app_env: "test".into(),
            address: [127, 0, 0, 1].into(),
            port: 0,
            cors_allowed_origins: vec!["http://localhost:5173".into()],
            dragonfly_url: "redis://127.0.0.1:1".into(),
            jwks_url: "http://127.0.0.1:1/jwks".into(),
            max_connections: 1_000,
            max_per_account: 5,
            queue: 16,
            resync_spread: Duration::from_millis(50),
            renew_grace_secs: 120,
            metrics_token: Some("secret".into()),
            metrics_allow_open: false,
        }
    }
}
