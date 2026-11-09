//! The sessions that are still alive, from the trace the API leaves in the key-value server each time it issues an access
//! token (sign-in, renewal, change of company): `<prefix>:<session>` = `{accountId, tenantId, exp}`, which expires with
//! that token.
//!
//! A stream lives longer than the token that opened it: when its deadline comes, the trace is read again instead of the
//! stream being closed. A revoked session is not renewed (the API refuses its refresh token), so its trace stops moving
//! and its stream closes at the next deadline, 5 minutes after the revocation at most. It is state with a lifetime, not a
//! message: a renewal whose message was lost is seen all the same.
//!
//! The traces are read in batches (`MGET`) over one connection that is shared and re-established by itself: 50 000
//! streams renewing every 5 minutes are about 170 keys a second, not 170 connections.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use redis::aio::ConnectionManager;
use serde::Deserialize;
use tokio::sync::OnceCell;
use uuid::Uuid;

use crate::{contract::SESSION_PREFIX, events::NO_TENANT};

/// Time allowed to a read, connection included: a store out of reach must not hold the sweeper.
const READ_TIMEOUT: Duration = Duration::from_secs(2);

/// The trace written by the API (camelCase JSON).
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Trace {
    pub account_id: Uuid,
    /// Absent in a system without companies.
    #[serde(default)]
    pub tenant_id: Option<Uuid>,
    /// Expiry of the last access token issued (unix seconds).
    pub exp: i64,
}

impl Trace {
    /// Until when the session stays alive for this account in this company; `None` if it moved to another company (from
    /// another tab) or if its last token expired.
    pub fn active_until(&self, account: Uuid, tenant: Uuid, now: i64) -> Option<i64> {
        (self.account_id == account && self.tenant_id.unwrap_or(NO_TENANT) == tenant && self.exp > now).then_some(self.exp)
    }
}

/// A session to look up.
#[derive(Debug, Clone, Copy)]
pub struct Query {
    pub session: Uuid,
    pub account: Uuid,
    pub tenant: Uuid,
}

#[derive(Clone)]
enum Store {
    Server { client: redis::Client, connection: Arc<OnceCell<ConnectionManager>> },
    /// Tests: nothing to reach.
    Memory(Arc<Mutex<HashMap<Uuid, Trace>>>),
}

#[derive(Clone)]
pub struct Sessions {
    store: Store,
}

impl std::fmt::Debug for Sessions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sessions").finish_non_exhaustive()
    }
}

impl Sessions {
    /// Only checks the URL: the server is reached when it is needed, so the service starts before it is ready.
    pub fn new(url: &str) -> anyhow::Result<Self> {
        Ok(Self { store: Store::Server { client: redis::Client::open(url)?, connection: Arc::new(OnceCell::new()) } })
    }

    pub fn in_memory() -> Self {
        Self { store: Store::Memory(Arc::default()) }
    }

    /// Writes a trace (tests, on `in_memory`).
    pub fn record(&self, session: Uuid, trace: Trace) {
        if let Store::Memory(traces) = &self.store
            && let Ok(mut traces) = traces.lock()
        {
            traces.insert(session, trace);
        }
    }

    pub fn key(session: Uuid) -> String {
        format!("{SESSION_PREFIX}:{session}")
    }

    /// For each query, until when its session is alive (`None`: no trace, expired, or another account or company). An
    /// error means the store could not be read: the sessions are neither alive nor dead, which is not the same thing.
    pub async fn active_until_many(&self, queries: &[Query], now: i64) -> anyhow::Result<Vec<Option<i64>>> {
        if queries.is_empty() {
            return Ok(Vec::new());
        }
        let traces: Vec<Option<Trace>> = match &self.store {
            Store::Memory(traces) => {
                let traces = traces.lock().map_err(|_| anyhow::anyhow!("traces locked"))?;
                queries.iter().map(|q| traces.get(&q.session).cloned()).collect()
            }
            Store::Server { client, connection } => {
                let read = async {
                    let mut connection = connection
                        .get_or_try_init(|| ConnectionManager::new(client.clone()))
                        .await?
                        .clone();
                    let keys: Vec<String> = queries.iter().map(|q| Self::key(q.session)).collect();
                    let raw: Vec<Option<String>> = redis::cmd("MGET").arg(&keys).query_async(&mut connection).await?;
                    anyhow::Ok(raw)
                };
                let raw = tokio::time::timeout(READ_TIMEOUT, read).await??;
                raw.into_iter().map(|json| json.and_then(|j| serde_json::from_str::<Trace>(&j).ok())).collect()
            }
        };
        Ok(queries
            .iter()
            .zip(traces)
            .map(|(q, trace)| trace.and_then(|t| t.active_until(q.account, q.tenant, now)))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trace(account: Uuid, tenant: Uuid, exp: i64) -> Trace {
        Trace { account_id: account, tenant_id: Some(tenant), exp }
    }

    #[test]
    fn keys_are_scoped_by_session() {
        let session = Uuid::new_v4();
        assert_eq!(Sessions::key(session), format!("{SESSION_PREFIX}:{session}"));
    }

    #[test]
    fn it_reads_what_the_api_writes() {
        let (account, tenant) = (Uuid::new_v4(), Uuid::new_v4());
        let json = serde_json::json!({"accountId": account, "tenantId": tenant, "exp": 1_700_000_300});
        assert_eq!(serde_json::from_value::<Trace>(json).unwrap(), trace(account, tenant, 1_700_000_300));
    }

    #[test]
    fn a_trace_without_a_company_vouches_for_a_stream_without_one_only() {
        let account = Uuid::new_v4();
        let t: Trace = serde_json::from_value(serde_json::json!({"accountId": account, "exp": 1_000})).unwrap();
        assert_eq!(t.active_until(account, NO_TENANT, 700), Some(1_000));
        assert_eq!(t.active_until(account, Uuid::new_v4(), 700), None);
        // And a trace with a company does not vouch for a stream that claims none.
        assert_eq!(trace(account, Uuid::new_v4(), 1_000).active_until(account, NO_TENANT, 700), None);
    }

    #[test]
    fn a_trace_vouches_for_its_account_and_company_until_it_expires() {
        let (account, tenant) = (Uuid::new_v4(), Uuid::new_v4());
        let t = trace(account, tenant, 1_000);
        assert_eq!(t.active_until(account, tenant, 700), Some(1_000));
        assert_eq!(t.active_until(account, tenant, 1_000), None);
        assert_eq!(t.active_until(account, Uuid::new_v4(), 700), None);
        assert_eq!(t.active_until(Uuid::new_v4(), tenant, 700), None);
    }

    #[tokio::test]
    async fn many_sessions_are_answered_in_one_call_in_the_order_asked() {
        let sessions = Sessions::in_memory();
        let now = chrono::Utc::now().timestamp();
        let (a, b, c) = ((Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()), (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()), (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()));
        sessions.record(a.0, trace(a.1, a.2, now + 300));
        sessions.record(c.0, trace(c.1, c.2, now - 1));
        let q = |t: (Uuid, Uuid, Uuid)| Query { session: t.0, account: t.1, tenant: t.2 };
        let answers = sessions.active_until_many(&[q(a), q(b), q(c)], now).await.unwrap();
        assert_eq!(answers, vec![Some(now + 300), None, None]);
        assert!(sessions.active_until_many(&[], now).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_store_out_of_reach_is_an_error_not_a_dead_session() {
        let sessions = Sessions::new("redis://127.0.0.1:1").unwrap();
        let q = Query { session: Uuid::new_v4(), account: Uuid::new_v4(), tenant: Uuid::new_v4() };
        assert!(sessions.active_until_many(&[q], 0).await.is_err());
    }
}
