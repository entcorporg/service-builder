//! The service put together: shared state, and the listener.

use std::{net::SocketAddr, ops::Deref, sync::Arc};

use anyhow::{Context, Result};
use tokio::net::TcpSocket;

use crate::{
    config::Config,
    contract::PATH,
    http::{self, MetricsAccess},
    hub::{self, Hub},
    jwt::AccessVerifier,
    metrics::Metrics,
    registry::{Limits, Registry},
    sessions::Sessions,
    sweeper,
};

/// Pending connections the kernel holds for the listener. Tokio's default is 1024: a service that has just restarted gets
/// every browser back within seconds, and the ones that do not fit in the queue wait for a SYN retransmit of a second.
const BACKLOG: u32 = 8192;

pub struct Shared {
    pub config: Config,
    pub verifier: Arc<AccessVerifier>,
    pub registry: Arc<Registry>,
    pub hub: Arc<Hub>,
    pub metrics: Arc<Metrics>,
    pub metrics_access: MetricsAccess,
}

/// What every connection shares: one `Arc`, so a connection costs a pointer for it.
#[derive(Clone)]
pub struct AppState(Arc<Shared>);

impl Deref for AppState {
    type Target = Shared;
    fn deref(&self) -> &Shared {
        &self.0
    }
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState").finish_non_exhaustive()
    }
}

impl AppState {
    /// Reaches nothing: the key-value server and the API are joined when needed, the service starts if they are not ready.
    pub fn new(config: Config) -> Self {
        let verifier = Arc::new(AccessVerifier::new(config.jwks_url.clone()));
        Self::with_verifier(config, verifier)
    }

    pub fn with_verifier(config: Config, verifier: Arc<AccessVerifier>) -> Self {
        let metrics = Arc::new(Metrics::default());
        let registry = Registry::new(
            Limits { max_connections: config.max_connections, max_per_account: config.max_per_account, queue: config.queue },
            metrics.clone(),
        );
        Self(Arc::new(Shared {
            metrics_access: MetricsAccess::new(config.metrics_token.as_deref(), config.metrics_allow_open),
            config,
            verifier,
            registry,
            hub: Arc::new(Hub::default()),
            metrics,
        }))
    }
}

/// Raises the limit of open files: a stream is a socket, and the usual soft limit (1024) is a fiftieth of what this
/// service is for. Returns the limit in force.
pub fn raise_file_limit(streams: usize) -> u64 {
    // A socket per stream, and some for the key-value server, the JWKS and the listener.
    let wanted = streams as u64 + 1024;
    match rlimit::increase_nofile_limit(wanted) {
        Ok(limit) => limit,
        Err(err) => {
            tracing::warn!(error = %err, "cannot raise the limit of open files");
            rlimit::getrlimit(rlimit::Resource::NOFILE).map(|(soft, _)| soft).unwrap_or(0)
        }
    }
}

/// Starts everything and serves until the process ends.
pub async fn run(config: Config) -> Result<()> {
    let limit = raise_file_limit(config.max_connections);
    if limit < config.max_connections as u64 + 64 {
        tracing::warn!(limit, max = config.max_connections, "the limit of open files is below REALTIME_MAX_CONNECTIONS: raise `ulimit -n`");
    }
    let state = AppState::new(config.clone());

    hub::spawn_subscriber(&config.dragonfly_url, state.hub.clone(), state.registry.clone(), state.metrics.clone(), config.resync_spread)?;
    sweeper::spawn(
        state.registry.clone(),
        Sessions::new(&config.dragonfly_url).context("DRAGONFLY_URL is not valid")?,
        state.metrics.clone(),
        config.renew_grace_secs,
    );

    let addr = SocketAddr::new(config.address, config.port);
    let socket = if addr.is_ipv4() { TcpSocket::new_v4()? } else { TcpSocket::new_v6()? };
    socket.set_reuseaddr(true)?;
    socket.bind(addr).with_context(|| format!("bind {addr}"))?;
    let listener = socket.listen(BACKLOG)?;
    tracing::info!(%addr, max_connections = config.max_connections, path = PATH, "realtime listening");
    http::serve(listener, state).await?;
    Ok(())
}
