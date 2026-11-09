//! The one task that looks after the open streams, in place of a timer for each of them.
//!
//! Every second it visits `1/CYCLE` of the streams: each gets a heartbeat (a comment line, so that no proxy in the way
//! closes an idle connection, and so that a dead connection is noticed: its queue fills and it is closed) and those whose
//! deadline has passed have their session read again, in batches. So a stream is visited every `CYCLE` seconds, which is
//! also how late a revoked session can be noticed, on top of the 5 minutes of a token.

use std::{sync::Arc, time::Duration};

use tokio::time::MissedTickBehavior;

use crate::{
    metrics::{Metrics, bump},
    registry::{Due, Registry},
    sessions::{Query, Sessions},
};

/// Seconds for a full visit of the streams.
pub const CYCLE: usize = 15;
/// Sessions asked in one `MGET`.
const BATCH: usize = 256;
/// A stream is kept at most this long past a token whose session is renewed: the API only issues tokens of 5 minutes,
/// a trace further away would be a mistake and must not make a stream immortal.
pub const MAX_EXTENSION_SECS: i64 = 15 * 60;
/// Time before a session that could not be checked is checked again.
const RETRY_SECS: i64 = CYCLE as i64;

pub fn spawn(registry: Arc<Registry>, sessions: Sessions, metrics: Arc<Metrics>, grace_secs: i64) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut n: usize = 0;
        loop {
            tick.tick().await;
            let now = chrono::Utc::now().timestamp();
            let due = registry.sweep(n % CYCLE, CYCLE, now);
            n = n.wrapping_add(1);
            renew(&registry, &sessions, &metrics, due, now, grace_secs).await;
        }
    });
}

/// Reads the sessions of the streams whose deadline passed, and extends, closes or keeps each of them.
pub async fn renew(registry: &Registry, sessions: &Sessions, metrics: &Metrics, due: Vec<Due>, now: i64, grace_secs: i64) {
    for batch in due.chunks(BATCH) {
        let queries: Vec<Query> = batch.iter().map(|d| Query { session: d.session, account: d.account, tenant: d.tenant }).collect();
        bump(&metrics.renew_batches);
        match sessions.active_until_many(&queries, now).await {
            Ok(answers) => {
                for (d, until) in batch.iter().zip(answers) {
                    match until {
                        Some(until) if until > now => registry.renewed(d.id, until.min(now + MAX_EXTENSION_SECS)),
                        _ => registry.expire(d.id),
                    }
                }
            }
            Err(err) => {
                bump(&metrics.renew_errors);
                tracing::warn!(error = ?err, streams = batch.len(), "sessions: cannot be read, streams kept for now");
                for d in batch {
                    registry.unverified(d.id, now, RETRY_SECS, grace_secs);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        registry::{Limits, Who},
        sessions::Trace,
    };
    use uuid::Uuid;

    fn setup() -> (Arc<Registry>, Sessions, Arc<Metrics>) {
        let metrics = Arc::new(Metrics::default());
        (Registry::new(Limits { max_connections: 100, max_per_account: 10, queue: 8 }, metrics.clone()), Sessions::in_memory(), metrics)
    }

    fn open(registry: &Arc<Registry>, sessions: &Sessions, trace_exp: Option<i64>) -> crate::registry::Connection {
        let who = Who { account: Uuid::new_v4(), tenant: Uuid::new_v4(), session: Uuid::new_v4(), client: None, permissions: vec![], deadline: 100 };
        if let Some(exp) = trace_exp {
            sessions.record(who.session, Trace { account_id: who.account, tenant_id: Some(who.tenant), exp });
        }
        registry.register(who).unwrap()
    }

    #[tokio::test]
    async fn a_renewed_session_keeps_its_stream_a_revoked_one_loses_it() {
        let (registry, sessions, metrics) = setup();
        let _alive = open(&registry, &sessions, Some(10_000));
        let _revoked = open(&registry, &sessions, None);
        let due = registry.sweep(0, 1, 500);
        assert_eq!(due.len(), 2);
        renew(&registry, &sessions, &metrics, due, 500, 120).await;
        assert_eq!(registry.len(), 1);
        // The deadline moved: the survivor is not due any more.
        assert!(registry.sweep(0, 1, 500).is_empty());
    }

    #[tokio::test]
    async fn a_far_future_trace_is_capped() {
        let (registry, sessions, metrics) = setup();
        let _s = open(&registry, &sessions, Some(10_000_000));
        let due = registry.sweep(0, 1, 500);
        renew(&registry, &sessions, &metrics, due, 500, 120).await;
        assert!(registry.sweep(0, 1, 500 + MAX_EXTENSION_SECS - 1).is_empty());
        assert_eq!(registry.sweep(0, 1, 500 + MAX_EXTENSION_SECS).len(), 1);
    }

    #[tokio::test]
    async fn a_store_out_of_reach_keeps_the_streams_for_the_grace_period_only() {
        let metrics = Arc::new(Metrics::default());
        let registry = Registry::new(Limits { max_connections: 100, max_per_account: 10, queue: 64 }, metrics.clone());
        let down = Sessions::new("redis://127.0.0.1:1").unwrap();
        let _s = registry.register(Who { account: Uuid::new_v4(), tenant: Uuid::new_v4(), session: Uuid::new_v4(), client: None, permissions: vec![], deadline: 100 }).unwrap();

        let due = registry.sweep(0, 1, 500);
        renew(&registry, &down, &metrics, due, 500, 120).await;
        assert_eq!(registry.len(), 1, "kept: the store blinked, the session is not known to be dead");
        assert_eq!(metrics.renew_errors.load(std::sync::atomic::Ordering::Relaxed), 1);
        // Checked again 15 s later; still no proof after the grace period: closed.
        let due = registry.sweep(0, 1, 520);
        renew(&registry, &down, &metrics, due, 520, 120).await;
        assert_eq!(registry.len(), 1);
        let due = registry.sweep(0, 1, 640);
        renew(&registry, &down, &metrics, due, 640, 120).await;
        assert_eq!(registry.len(), 0);
    }

    #[tokio::test]
    async fn thousands_of_sessions_are_read_in_batches() {
        let (_, sessions, metrics) = setup();
        let registry = Registry::new(Limits { max_connections: 5_000, max_per_account: 10, queue: 8 }, metrics.clone());
        let _streams: Vec<_> = (0..1_000).map(|_| open(&registry, &sessions, Some(10_000))).collect();
        let due = registry.sweep(0, 1, 500);
        renew(&registry, &sessions, &metrics, due, 500, 120).await;
        assert_eq!(metrics.renew_batches.load(std::sync::atomic::Ordering::Relaxed), 4, "1 000 sessions in batches of 256");
        assert_eq!(registry.len(), 1_000);
    }
}
