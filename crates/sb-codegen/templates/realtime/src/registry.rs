//! The streams open on this instance, and who each event goes to.
//!
//! The point of this module is what it does *not* do: it never looks at a stream an event is not for. The streams are
//! indexed by account and by `(company, permission)`, so an event is looked up and handed to its recipients only; the
//! work is proportional to the number of recipients, not to the number of open streams. With 50 000 streams and 1 000
//! events a second, a broadcast that every stream filters for itself is 50 million wake-ups and checks a second; this
//! is a few thousand queue pushes.
//!
//! - A frame is built once per event and shared (`Arc`) by the queues it goes to.
//! - Every stream has a small bounded queue. Pushing never waits: a stream whose client does not read (its queue is full)
//!   is closed, and the browser, which reads its state again when it reconnects, loses nothing it cannot recover.
//! - A stream costs a slot, a few index entries and its queue: no task, no timer. The [`sweeper`](crate::sweeper) visits
//!   the slots a slice at a time for heartbeats and for the sessions to renew.
//! - Nothing here waits or does I/O while the lock is held.

use std::{
    collections::{HashMap, HashSet},
    hash::{BuildHasherDefault, Hasher},
    pin::Pin,
    sync::{Arc, Mutex, MutexGuard},
    task::{Context, Poll},
};

use bytes::Bytes;
use futures::Stream;
use tokio::sync::mpsc::{self, error::TrySendError};
use uuid::Uuid;

use crate::{
    events::{Audience, GRANTS_UPDATED, LiveEvent, RESYNC, frame, is_permission},
    metrics::{Metrics, add, bump},
};

/// A frame of the stream, shared by every queue it is pushed to.
#[derive(Debug)]
pub struct Frame(pub Bytes);

/// Slot and generation of a stream: a slot is reused, and a stale id must not reach the stream that took it.
pub type ConnId = u64;
type PermId = u32;

/// The permissions are a handful of names (`module.level`); a publisher that invented them without end would otherwise
/// make the table grow without end.
const MAX_DISTINCT_PERMISSIONS: usize = 4096;

/// Ids are sequence numbers: hashing them again is a waste.
#[derive(Default)]
pub struct IdHasher(u64);

impl Hasher for IdHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 = self.0.rotate_left(8) ^ u64::from(*b);
        }
    }
    fn write_u64(&mut self, n: u64) {
        self.0 = n.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    }
}

type Ids = HashSet<ConnId, BuildHasherDefault<IdHasher>>;

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Streams on this instance. Beyond it, a new one is refused (503): that is what keeps the memory bounded.
    pub max_connections: usize,
    /// Streams of one account. Beyond it, the oldest one is closed to make room: a browser that reconnects while the
    /// server has not noticed that the old connection is dead must not be locked out.
    pub max_per_account: usize,
    /// Frames waiting in a stream.
    pub queue: usize,
}

/// Who opens a stream, from its verified token.
#[derive(Debug, Clone)]
pub struct Who {
    pub account: Uuid,
    pub tenant: Uuid,
    pub session: Uuid,
    /// The tab (`X-Client-Id`), whose own events are not sent back to it.
    pub client: Option<Uuid>,
    pub permissions: Vec<String>,
    /// Until when the token that opened it vouches for the session (unix seconds).
    pub deadline: i64,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The instance holds as many streams as it may.
    Full,
}

struct Conn {
    /// Order of opening, to find the oldest of an account.
    seq: u64,
    account: Uuid,
    tenant: Uuid,
    session: Uuid,
    client: Option<Uuid>,
    perms: Box<[PermId]>,
    deadline: i64,
    /// Since when the session cannot be checked (the store is out of reach), 0 if it can.
    stale_since: i64,
    tx: mpsc::Sender<Arc<Frame>>,
}

struct Slot {
    generation: u32,
    conn: Option<Conn>,
}

#[derive(Default)]
struct Inner {
    slots: Vec<Slot>,
    free: Vec<u32>,
    live: usize,
    next_seq: u64,
    by_account: HashMap<Uuid, Ids>,
    by_audience: HashMap<(Uuid, PermId), Ids>,
    perm_ids: HashMap<String, PermId>,
}

fn pack(index: u32, generation: u32) -> ConnId {
    (u64::from(generation) << 32) | u64::from(index)
}

fn unpack(id: ConnId) -> (usize, u32) {
    ((id & 0xFFFF_FFFF) as usize, (id >> 32) as u32)
}

impl Inner {
    fn conn(&self, id: ConnId) -> Option<&Conn> {
        let (index, generation) = unpack(id);
        self.slots.get(index).filter(|s| s.generation == generation).and_then(|s| s.conn.as_ref())
    }

    fn conn_mut(&mut self, id: ConnId) -> Option<&mut Conn> {
        let (index, generation) = unpack(id);
        self.slots.get_mut(index).filter(|s| s.generation == generation).and_then(|s| s.conn.as_mut())
    }

    fn intern(&mut self, name: &str) -> Option<PermId> {
        if let Some(id) = self.perm_ids.get(name) {
            return Some(*id);
        }
        if !is_permission(name) || self.perm_ids.len() >= MAX_DISTINCT_PERMISSIONS {
            return None;
        }
        let id = self.perm_ids.len() as PermId;
        self.perm_ids.insert(name.to_string(), id);
        Some(id)
    }

    fn intern_all(&mut self, names: &[String]) -> Box<[PermId]> {
        let mut ids: Vec<PermId> = names.iter().filter_map(|n| self.intern(n)).collect();
        ids.sort_unstable();
        ids.dedup();
        ids.into_boxed_slice()
    }

    fn index(&mut self, id: ConnId, account: Uuid, tenant: Uuid, perms: &[PermId]) {
        self.by_account.entry(account).or_default().insert(id);
        for p in perms {
            self.by_audience.entry((tenant, *p)).or_default().insert(id);
        }
    }

    fn unindex(&mut self, id: ConnId, account: Uuid, tenant: Uuid, perms: &[PermId]) {
        if let Some(ids) = self.by_account.get_mut(&account) {
            ids.remove(&id);
            if ids.is_empty() {
                self.by_account.remove(&account);
            }
        }
        for p in perms {
            if let Some(ids) = self.by_audience.get_mut(&(tenant, *p)) {
                ids.remove(&id);
                if ids.is_empty() {
                    self.by_audience.remove(&(tenant, *p));
                }
            }
        }
    }

    /// Takes a stream out: dropping its sender ends the stream of the browser.
    fn remove(&mut self, id: ConnId) -> Option<Conn> {
        let (index, generation) = unpack(id);
        let slot = self.slots.get_mut(index).filter(|s| s.generation == generation)?;
        let conn = slot.conn.take()?;
        slot.generation = slot.generation.wrapping_add(1);
        self.free.push(index as u32);
        self.live -= 1;
        self.unindex(id, conn.account, conn.tenant, &conn.perms);
        Some(conn)
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Delivered {
    /// Streams the frame was queued for.
    pub recipients: usize,
    /// Streams closed because their queue was full.
    pub slow: usize,
}

/// A stream the sweeper must check the session of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Due {
    pub id: ConnId,
    pub session: Uuid,
    pub account: Uuid,
    pub tenant: Uuid,
}

pub struct Registry {
    inner: Mutex<Inner>,
    limits: Limits,
    metrics: Arc<Metrics>,
    heartbeat: Arc<Frame>,
    grants: Arc<Frame>,
    resync: Arc<Frame>,
}

impl std::fmt::Debug for Registry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Registry").field("limits", &self.limits).finish_non_exhaustive()
    }
}

impl Registry {
    pub fn new(limits: Limits, metrics: Arc<Metrics>) -> Arc<Self> {
        let none = crate::events::Data::new();
        Arc::new(Self {
            inner: Mutex::new(Inner::default()),
            limits,
            metrics,
            heartbeat: Arc::new(Frame(Bytes::from_static(b": keep-alive\n\n"))),
            grants: Arc::new(Frame(frame(GRANTS_UPDATED, &none))),
            resync: Arc::new(Frame(frame(RESYNC, &none))),
        })
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // A panic elsewhere must not take every stream down with it: the index is rebuilt by each operation.
        self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn len(&self) -> usize {
        self.lock().live
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Opens a stream. When the account already has `max_per_account`, its oldest one is closed; when the instance is
    /// full, the new one is refused.
    pub fn register(self: &Arc<Self>, who: Who) -> Result<Connection, Refusal> {
        let (tx, rx) = mpsc::channel(self.limits.queue);
        let mut g = self.lock();
        let account_count = g.by_account.get(&who.account).map_or(0, HashSet::len);
        let evict = account_count >= self.limits.max_per_account;
        if g.live - usize::from(evict) >= self.limits.max_connections {
            bump(&self.metrics.rejected_full);
            return Err(Refusal::Full);
        }
        if evict {
            let oldest = g
                .by_account
                .get(&who.account)
                .into_iter()
                .flatten()
                .filter_map(|id| g.conn(*id).map(|c| (c.seq, *id)))
                .min()
                .map(|(_, id)| id);
            if let Some(id) = oldest {
                g.remove(id);
                bump(&self.metrics.closed_evicted);
            }
        }
        let perms = g.intern_all(&who.permissions);
        let seq = g.next_seq;
        g.next_seq += 1;
        let conn = Conn {
            seq,
            account: who.account,
            tenant: who.tenant,
            session: who.session,
            client: who.client,
            perms,
            deadline: who.deadline,
            stale_since: 0,
            tx,
        };
        let index = match g.free.pop() {
            Some(index) => index,
            None => {
                g.slots.push(Slot { generation: 0, conn: None });
                (g.slots.len() - 1) as u32
            }
        };
        let generation = g.slots[index as usize].generation;
        let id = pack(index, generation);
        let (account, tenant, perms) = (conn.account, conn.tenant, conn.perms.clone());
        g.slots[index as usize].conn = Some(conn);
        g.live += 1;
        g.index(id, account, tenant, &perms);
        drop(g);
        bump(&self.metrics.opened);
        Ok(Connection { rx, registry: self.clone(), id })
    }

    /// Queues an event for the streams it is for.
    pub fn dispatch(&self, event: &LiveEvent) -> Delivered {
        let frame = Arc::new(Frame(frame(&event.kind, &event.data)));
        let mut g = self.lock();
        let mut targets: Vec<ConnId> = Vec::new();
        match &event.audience {
            Audience::Account(account) => {
                targets.extend(g.by_account.get(account).into_iter().flatten());
            }
            Audience::Tenant { tenant_id, permission, accounts } => {
                let perm = permission.as_deref().and_then(|p| g.perm_ids.get(p)).copied();
                if let Some(ids) = perm.and_then(|p| g.by_audience.get(&(*tenant_id, p))) {
                    targets.extend(ids);
                }
                // The named accounts get it without the permission, in this company only.
                for account in accounts {
                    for id in g.by_account.get(account).into_iter().flatten() {
                        if g.conn(*id).is_some_and(|c| c.tenant == *tenant_id) {
                            targets.push(*id);
                        }
                    }
                }
                if !accounts.is_empty() {
                    targets.sort_unstable();
                    targets.dedup();
                }
            }
        }
        let mut out = Delivered::default();
        let mut slow = Vec::new();
        let mut gone = Vec::new();
        for id in targets {
            let Some(conn) = g.conn(id) else { continue };
            if event.origin.is_some() && event.origin == conn.client {
                continue;
            }
            match conn.tx.try_send(frame.clone()) {
                Ok(()) => out.recipients += 1,
                Err(TrySendError::Full(_)) => slow.push(id),
                Err(TrySendError::Closed(_)) => gone.push(id),
            }
        }
        out.slow = slow.len();
        for id in slow {
            g.remove(id);
        }
        for id in gone {
            g.remove(id);
        }
        drop(g);
        add(&self.metrics.closed_slow, out.slow as u64);
        add(&self.metrics.frames_delivered, out.recipients as u64);
        out
    }

    /// The rights of an account in a company changed. With a list, its streams there adopt it without closing and
    /// their browser is told to read its state again; without, they close.
    pub fn grants_updated(&self, tenant: Uuid, account: Uuid, permissions: Option<Vec<String>>) {
        let mut g = self.lock();
        let ids: Vec<ConnId> = g.by_account.get(&account).into_iter().flatten().copied().collect();
        let (mut revoked, mut slow) = (0u64, 0u64);
        for id in ids {
            if g.conn(id).is_none_or(|c| c.tenant != tenant) {
                continue;
            }
            let Some(permissions) = &permissions else {
                g.remove(id);
                revoked += 1;
                continue;
            };
            let new = g.intern_all(permissions);
            let Some(conn) = g.conn_mut(id) else { continue };
            let old = std::mem::replace(&mut conn.perms, new.clone());
            let told = conn.tx.try_send(self.grants.clone());
            g.unindex(id, account, tenant, &old);
            g.index(id, account, tenant, &new);
            if told.is_err() {
                g.remove(id);
                slow += 1;
            }
        }
        drop(g);
        add(&self.metrics.closed_revoked, revoked);
        add(&self.metrics.closed_slow, slow);
    }

    /// Heartbeat for the streams of one slice of the slots, and the ones whose session must be checked. A slice is
    /// `slice / slices` of the slots: the sweeper takes one a second, so every stream is visited once per cycle.
    pub fn sweep(&self, slice: usize, slices: usize, now: i64) -> Vec<Due> {
        let mut g = self.lock();
        let len = g.slots.len();
        let (from, to) = (len * slice / slices, len * (slice + 1) / slices);
        let (mut due, mut stuck) = (Vec::new(), Vec::new());
        for index in from..to {
            let slot = &g.slots[index];
            let Some(conn) = &slot.conn else { continue };
            let id = pack(index as u32, slot.generation);
            match conn.tx.try_send(self.heartbeat.clone()) {
                Ok(()) => {}
                // Not reading even a heartbeat: the connection is dead or stuck.
                Err(_) => {
                    stuck.push(id);
                    continue;
                }
            }
            if conn.deadline <= now {
                due.push(Due { id, session: conn.session, account: conn.account, tenant: conn.tenant });
            }
        }
        let slow = stuck.len() as u64;
        for id in stuck {
            g.remove(id);
        }
        drop(g);
        add(&self.metrics.closed_slow, slow);
        due
    }

    /// The session is still valid until `until`.
    pub fn renewed(&self, id: ConnId, until: i64) {
        if let Some(conn) = self.lock().conn_mut(id) {
            conn.deadline = until;
            conn.stale_since = 0;
        }
    }

    /// The session could not be checked (the store is out of reach). The stream stays, checked again in `retry` seconds,
    /// until it has gone `grace` seconds without proof: closing every stream because the store blinked would make every
    /// browser reconnect at once.
    pub fn unverified(&self, id: ConnId, now: i64, retry: i64, grace: i64) {
        let mut g = self.lock();
        let Some(conn) = g.conn_mut(id) else { return };
        if conn.stale_since == 0 {
            conn.stale_since = now;
        }
        if now - conn.stale_since >= grace {
            g.remove(id);
            drop(g);
            bump(&self.metrics.closed_expired);
        } else if let Some(conn) = g.conn_mut(id) {
            conn.deadline = now + retry;
        }
    }

    /// The session ended (revoked, or another company): the stream closes.
    pub fn expire(&self, id: ConnId) {
        if self.lock().remove(id).is_some() {
            bump(&self.metrics.closed_expired);
        }
    }

    /// Every open stream, to tell them to read their state again.
    pub fn ids(&self) -> Vec<ConnId> {
        let g = self.lock();
        g.slots
            .iter()
            .enumerate()
            .filter(|(_, s)| s.conn.is_some())
            .map(|(i, s)| pack(i as u32, s.generation))
            .collect()
    }

    /// Tells these streams to read their state again (the ones that closed since are skipped).
    pub fn resync(&self, ids: &[ConnId]) {
        let mut g = self.lock();
        let mut slow = Vec::new();
        for id in ids {
            let Some(conn) = g.conn(*id) else { continue };
            if conn.tx.try_send(self.resync.clone()).is_err() {
                slow.push(*id);
            }
        }
        let slow_count = slow.len() as u64;
        for id in slow {
            g.remove(id);
        }
        drop(g);
        add(&self.metrics.closed_slow, slow_count);
    }

    fn drop_connection(&self, id: ConnId) {
        if self.lock().remove(id).is_some() {
            bump(&self.metrics.closed_client);
        }
    }
}

/// The open stream: the frames queued for it, as bytes. Dropping it (the browser went away,
/// or the server ended it) takes the stream out of the index.
pub struct Connection {
    rx: mpsc::Receiver<Arc<Frame>>,
    registry: Arc<Registry>,
    id: ConnId,
}

impl Stream for Connection {
    type Item = Bytes;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.rx.poll_recv(cx).map(|frame| frame.map(|f| f.0.clone()))
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.registry.drop_connection(self.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::Data;
    use futures::StreamExt;

    fn registry(max: usize, per_account: usize, queue: usize) -> (Arc<Registry>, Arc<Metrics>) {
        let metrics = Arc::new(Metrics::default());
        (Registry::new(Limits { max_connections: max, max_per_account: per_account, queue }, metrics.clone()), metrics)
    }

    fn who(tenant: Uuid, permissions: &[&str]) -> Who {
        Who {
            account: Uuid::new_v4(),
            tenant,
            session: Uuid::new_v4(),
            client: None,
            permissions: permissions.iter().map(|p| p.to_string()).collect(),
            deadline: 1_000,
        }
    }

    fn event(tenant: Uuid, permission: &str) -> LiveEvent {
        LiveEvent {
            kind: "contact.updated".into(),
            audience: Audience::Tenant { tenant_id: tenant, permission: Some(permission.into()), accounts: vec![] },
            origin: None,
            data: Data::from([("id".to_string(), "c1".to_string())]),
        }
    }

    async fn next(c: &mut Connection) -> Option<String> {
        let item = tokio::time::timeout(std::time::Duration::from_millis(50), c.next()).await.ok()??;
        Some(String::from_utf8(item.to_vec()).unwrap())
    }

    fn indexed(r: &Registry) -> (usize, usize, usize) {
        let g = r.lock();
        (g.live, g.by_account.values().map(HashSet::len).sum(), g.by_audience.values().map(HashSet::len).sum())
    }

    #[tokio::test]
    async fn an_event_reaches_the_streams_with_the_permission_in_the_company_only() {
        let (r, _) = registry(10, 5, 8);
        let tenant = Uuid::new_v4();
        let mut yes = r.register(who(tenant, &["contacts.view"])).unwrap();
        let mut no_permission = r.register(who(tenant, &["tasks.view"])).unwrap();
        let mut other_company = r.register(who(Uuid::new_v4(), &["contacts.view"])).unwrap();

        let out = r.dispatch(&event(tenant, "contacts.view"));
        assert_eq!(out, Delivered { recipients: 1, slow: 0 });
        assert_eq!(next(&mut yes).await.as_deref(), Some("event: contact.updated\ndata: {\"id\":\"c1\"}\n\n"));
        assert_eq!(next(&mut no_permission).await, None);
        assert_eq!(next(&mut other_company).await, None);
    }

    #[tokio::test]
    async fn without_companies_the_streams_are_in_one_scope_that_the_companies_do_not_share() {
        use crate::events::NO_TENANT;
        let (r, _) = registry(10, 5, 8);
        let company = Uuid::new_v4();
        let mut plain = r.register(who(NO_TENANT, &["news.view"])).unwrap();
        let mut plain_other_right = r.register(who(NO_TENANT, &["tasks.view"])).unwrap();
        let mut in_company = r.register(who(company, &["news.view"])).unwrap();

        // An event with no company reaches the streams with no company that have the permission, and no other.
        assert_eq!(r.dispatch(&event(NO_TENANT, "news.view")).recipients, 1);
        assert!(next(&mut plain).await.is_some());
        assert_eq!(next(&mut plain_other_right).await, None);
        assert_eq!(next(&mut in_company).await, None);
        // And an event for a company does not reach the streams that have none.
        assert_eq!(r.dispatch(&event(company, "news.view")).recipients, 1);
        assert!(next(&mut in_company).await.is_some());
        assert_eq!(next(&mut plain).await, None);
    }

    #[tokio::test]
    async fn an_event_with_a_permission_nobody_has_reaches_nobody() {
        let (r, _) = registry(10, 5, 8);
        let tenant = Uuid::new_v4();
        let _c = r.register(who(tenant, &["contacts.view"])).unwrap();
        assert_eq!(r.dispatch(&event(tenant, "billing.view")), Delivered::default());
    }

    #[tokio::test]
    async fn named_accounts_get_the_event_without_the_permission_and_only_once() {
        let (r, _) = registry(10, 5, 8);
        let tenant = Uuid::new_v4();
        let target = who(tenant, &["contacts.view"]);
        let account = target.account;
        let mut both = r.register(target).unwrap();
        let named = who(tenant, &[]);
        let named_account = named.account;
        let mut only_named = r.register(named).unwrap();
        // The same account in another company is not named there.
        let mut elsewhere = r.register(Who { account: named_account, ..who(Uuid::new_v4(), &[]) }).unwrap();

        let e = LiveEvent {
            kind: "contact.updated".into(),
            audience: Audience::Tenant { tenant_id: tenant, permission: Some("contacts.view".into()), accounts: vec![account, named_account] },
            origin: None,
            data: Data::new(),
        };
        assert_eq!(r.dispatch(&e).recipients, 2);
        assert!(next(&mut both).await.is_some());
        assert_eq!(next(&mut both).await, None, "once, not twice");
        assert!(next(&mut only_named).await.is_some());
        assert_eq!(next(&mut elsewhere).await, None);
    }

    #[tokio::test]
    async fn an_account_event_follows_the_account_in_every_company() {
        let (r, _) = registry(10, 5, 8);
        let account = Uuid::new_v4();
        let mut a = r.register(Who { account, ..who(Uuid::new_v4(), &[]) }).unwrap();
        let mut b = r.register(Who { account, ..who(Uuid::new_v4(), &[]) }).unwrap();
        let mut stranger = r.register(who(Uuid::new_v4(), &[])).unwrap();
        let e = LiveEvent { kind: "account.updated".into(), audience: Audience::Account(account), origin: None, data: Data::new() };
        assert_eq!(r.dispatch(&e).recipients, 2);
        assert!(next(&mut a).await.is_some() && next(&mut b).await.is_some());
        assert_eq!(next(&mut stranger).await, None);
    }

    #[tokio::test]
    async fn the_author_tab_is_left_out_but_not_the_other_tabs() {
        let (r, _) = registry(10, 5, 8);
        let tenant = Uuid::new_v4();
        let tab = Uuid::new_v4();
        let mut author = r.register(Who { client: Some(tab), ..who(tenant, &["contacts.view"]) }).unwrap();
        let mut other = r.register(Who { client: Some(Uuid::new_v4()), ..who(tenant, &["contacts.view"]) }).unwrap();
        let mut none = r.register(who(tenant, &["contacts.view"])).unwrap();
        let mut e = event(tenant, "contacts.view");
        e.origin = Some(tab);
        assert_eq!(r.dispatch(&e).recipients, 2);
        assert_eq!(next(&mut author).await, None);
        assert!(next(&mut other).await.is_some() && next(&mut none).await.is_some());
    }

    #[tokio::test]
    async fn a_stream_that_does_not_read_is_closed_and_the_others_are_not_held_up() {
        let (r, metrics) = registry(10, 5, 4);
        let tenant = Uuid::new_v4();
        let mut slow = r.register(who(tenant, &["contacts.view"])).unwrap();
        let mut reader = r.register(who(tenant, &["contacts.view"])).unwrap();
        for _ in 0..4 {
            r.dispatch(&event(tenant, "contacts.view"));
            // The reader keeps up; the slow one does not read.
            assert!(next(&mut reader).await.is_some());
        }
        let out = r.dispatch(&event(tenant, "contacts.view"));
        assert_eq!(out, Delivered { recipients: 1, slow: 1 });
        assert_eq!(r.len(), 1);
        assert_eq!(metrics.closed_slow.load(std::sync::atomic::Ordering::Relaxed), 1);
        // The frames it had are still there, then its stream ends.
        for _ in 0..4 {
            assert!(next(&mut slow).await.is_some());
        }
        assert!(slow.next().await.is_none());
        assert!(next(&mut reader).await.is_some());
    }

    #[tokio::test]
    async fn the_instance_refuses_streams_beyond_its_limit_and_takes_them_again_when_one_closes() {
        let (r, metrics) = registry(2, 5, 8);
        let tenant = Uuid::new_v4();
        let a = r.register(who(tenant, &[])).unwrap();
        let _b = r.register(who(tenant, &[])).unwrap();
        assert_eq!(r.register(who(tenant, &[])).err(), Some(Refusal::Full));
        assert_eq!(metrics.rejected_full.load(std::sync::atomic::Ordering::Relaxed), 1);
        drop(a);
        assert!(r.register(who(tenant, &[])).is_ok());
    }

    #[tokio::test]
    async fn the_oldest_stream_of_an_account_makes_room_for_a_new_one() {
        let (r, metrics) = registry(10, 2, 8);
        let account = Uuid::new_v4();
        let tenant = Uuid::new_v4();
        let open = || r.register(Who { account, ..who(tenant, &["contacts.view"]) }).unwrap();
        let mut first = open();
        let mut second = open();
        let mut third = open();
        assert_eq!(r.len(), 2);
        assert!(first.next().await.is_none(), "the oldest was closed");
        assert_eq!(metrics.closed_evicted.load(std::sync::atomic::Ordering::Relaxed), 1);
        r.dispatch(&event(tenant, "contacts.view"));
        assert!(next(&mut second).await.is_some() && next(&mut third).await.is_some());
    }

    #[tokio::test]
    async fn a_full_instance_still_takes_the_stream_that_replaces_one_of_the_same_account() {
        let (r, _) = registry(2, 1, 8);
        let tenant = Uuid::new_v4();
        let account = Uuid::new_v4();
        let _old = r.register(Who { account, ..who(tenant, &[]) }).unwrap();
        let _other = r.register(who(tenant, &[])).unwrap();
        let replacement = r.register(Who { account, ..who(tenant, &[]) });
        assert!(replacement.is_ok());
        assert_eq!(r.len(), 2);
    }

    #[tokio::test]
    async fn new_grants_are_adopted_without_closing_and_the_browser_is_told() {
        let (r, _) = registry(10, 5, 8);
        let tenant = Uuid::new_v4();
        let mine = who(tenant, &["tasks.view"]);
        let account = mine.account;
        let mut stream = r.register(mine).unwrap();

        assert_eq!(r.dispatch(&event(tenant, "contacts.view")).recipients, 0);
        r.grants_updated(tenant, account, Some(vec!["contacts.view".into()]));
        assert_eq!(next(&mut stream).await.as_deref(), Some("event: grants.updated\ndata: {}\n\n"));
        assert_eq!(r.dispatch(&event(tenant, "contacts.view")).recipients, 1);
        assert!(next(&mut stream).await.is_some());
        // The old right is gone from the index.
        assert_eq!(r.dispatch(&event(tenant, "tasks.view")).recipients, 0);
        assert_eq!(indexed(&r), (1, 1, 1));
    }

    #[tokio::test]
    async fn grants_without_permissions_close_that_accounts_streams_in_that_company_only() {
        let (r, metrics) = registry(10, 5, 8);
        let (tenant, other_tenant) = (Uuid::new_v4(), Uuid::new_v4());
        let mine = who(tenant, &["contacts.view"]);
        let account = mine.account;
        let mut concerned = r.register(mine).unwrap();
        let mut elsewhere = r.register(Who { account, ..who(other_tenant, &["contacts.view"]) }).unwrap();
        let mut other_account = r.register(who(tenant, &["contacts.view"])).unwrap();

        r.grants_updated(tenant, account, None);
        assert!(concerned.next().await.is_none());
        assert_eq!(metrics.closed_revoked.load(std::sync::atomic::Ordering::Relaxed), 1);
        r.dispatch(&event(other_tenant, "contacts.view"));
        r.dispatch(&event(tenant, "contacts.view"));
        assert!(next(&mut elsewhere).await.is_some());
        assert!(next(&mut other_account).await.is_some());
        assert_eq!(r.len(), 2);
    }

    #[tokio::test]
    async fn a_dropped_stream_leaves_no_trace_in_the_index() {
        let (r, metrics) = registry(100, 100, 8);
        let tenant = Uuid::new_v4();
        let streams: Vec<_> = (0..50).map(|_| r.register(who(tenant, &["contacts.view", "tasks.view"])).unwrap()).collect();
        assert_eq!(indexed(&r), (50, 50, 100));
        drop(streams);
        assert_eq!(indexed(&r), (0, 0, 0));
        assert_eq!(metrics.closed_client.load(std::sync::atomic::Ordering::Relaxed), 50);
        // The slots are taken again, with other generations: an old id reaches nothing.
        let fresh = r.register(who(tenant, &[])).unwrap();
        assert!(r.lock().conn(fresh.id ^ (1 << 32)).is_none());
        assert_eq!(r.lock().slots.len(), 50);
    }

    #[tokio::test]
    async fn the_sweep_sends_heartbeats_to_every_stream_once_per_cycle_and_finds_the_sessions_to_renew() {
        let (r, _) = registry(100, 100, 8);
        let tenant = Uuid::new_v4();
        let mut streams: Vec<_> = (0..30).map(|_| r.register(who(tenant, &[])).unwrap()).collect();
        // Half of them have a deadline that passed.
        let mut due_total = 0;
        for slice in 0..5 {
            due_total += r.sweep(slice, 5, 2_000).len();
        }
        assert_eq!(due_total, 30);
        for s in &mut streams {
            assert_eq!(next(s).await.as_deref(), Some(": keep-alive\n\n"));
            assert_eq!(next(s).await, None, "one heartbeat per cycle");
        }
        // Renewed streams are not due any more.
        let ids = r.ids();
        for id in &ids {
            r.renewed(*id, 5_000);
        }
        assert!((0..5).all(|slice| r.sweep(slice, 5, 2_000).is_empty()));
    }

    #[tokio::test]
    async fn a_stream_that_does_not_even_read_its_heartbeats_is_closed() {
        let (r, _) = registry(10, 5, 2);
        let tenant = Uuid::new_v4();
        let mut stuck = r.register(who(tenant, &[])).unwrap();
        for _ in 0..3 {
            r.sweep(0, 1, 0);
        }
        assert_eq!(r.len(), 0);
        assert!(next(&mut stuck).await.is_some());
    }

    #[tokio::test]
    async fn a_session_that_cannot_be_checked_keeps_its_stream_for_a_while_then_closes_it() {
        let (r, _) = registry(10, 5, 8);
        let _s = r.register(who(Uuid::new_v4(), &[])).unwrap();
        let id = r.ids()[0];
        r.unverified(id, 100, 15, 120);
        assert_eq!(r.len(), 1);
        assert_eq!(r.lock().conn(id).unwrap().deadline, 115);
        r.unverified(id, 200, 15, 120);
        assert_eq!(r.len(), 1);
        r.unverified(id, 221, 15, 120);
        assert_eq!(r.len(), 0, "no proof for 121 s");
        // A proof in between starts the count again.
        let _t = r.register(who(Uuid::new_v4(), &[])).unwrap();
        let id = r.ids()[0];
        r.unverified(id, 100, 15, 120);
        r.renewed(id, 400);
        r.unverified(id, 300, 15, 120);
        assert_eq!(r.len(), 1);
    }

    #[tokio::test]
    async fn an_expired_session_closes_its_stream() {
        let (r, metrics) = registry(10, 5, 8);
        let mut s = r.register(who(Uuid::new_v4(), &[])).unwrap();
        r.expire(r.ids()[0]);
        assert!(s.next().await.is_none());
        assert_eq!(metrics.closed_expired.load(std::sync::atomic::Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn a_resync_tells_every_open_stream_without_closing_any() {
        let (r, _) = registry(10, 5, 8);
        let tenant = Uuid::new_v4();
        let mut a = r.register(who(tenant, &[])).unwrap();
        let mut b = r.register(who(tenant, &[])).unwrap();
        let ids = r.ids();
        r.resync(&ids);
        assert_eq!(next(&mut a).await.as_deref(), Some("event: resync\ndata: {}\n\n"));
        assert_eq!(next(&mut b).await.as_deref(), Some("event: resync\ndata: {}\n\n"));
        assert_eq!(r.len(), 2);
    }

    /// The cost of an event must follow its recipients, not the streams that are open: this is the property the module
    /// exists for. A lookup that scanned the streams would take a hundred times longer on the large side.
    #[test]
    fn the_cost_of_an_event_does_not_depend_on_how_many_streams_are_open() {
        fn time_per_event(open: usize) -> std::time::Duration {
            let (r, _) = registry(open + 10, open + 10, 4);
            let busy = Uuid::new_v4();
            let mut keep = Vec::with_capacity(open);
            for i in 0..open {
                // Streams of many companies, 10 in the one that the events are for.
                let tenant = if i < 10 { busy } else { Uuid::new_v4() };
                keep.push(r.register(who(tenant, &["contacts.view"])).unwrap());
            }
            let e = event(busy, "contacts.view");
            let start = std::time::Instant::now();
            let rounds = 200;
            for _ in 0..rounds {
                r.dispatch(&e);
                // Keep the 10 queues from filling.
                for s in keep.iter_mut().take(10) {
                    while s.rx.try_recv().is_ok() {}
                }
            }
            start.elapsed() / rounds
        }
        let (small, large) = (time_per_event(1_000), time_per_event(50_000));
        assert!(large < small * 20 + std::time::Duration::from_micros(200), "{small:?} for 1 000 streams, {large:?} for 50 000");
    }
}
