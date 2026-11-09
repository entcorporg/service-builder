//! Counters for `/metrics` (Prometheus text format). Plain atomics: nothing on the hot path takes a lock for them.

use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

#[derive(Debug, Default)]
pub struct Metrics {
    /// Streams opened.
    pub opened: AtomicU64,
    /// Streams closed, by the reason.
    pub closed_client: AtomicU64,
    pub closed_slow: AtomicU64,
    pub closed_expired: AtomicU64,
    pub closed_revoked: AtomicU64,
    pub closed_evicted: AtomicU64,
    /// Streams refused: the instance is full, or the token is not valid.
    pub rejected_full: AtomicU64,
    pub rejected_auth: AtomicU64,
    /// Messages read from the channel, and those that were not valid.
    pub events_received: AtomicU64,
    pub events_refused: AtomicU64,
    /// Frames put in the queue of a stream (heartbeats and control frames not counted).
    pub frames_delivered: AtomicU64,
    /// Batches of session traces read, and those that failed.
    pub renew_batches: AtomicU64,
    pub renew_errors: AtomicU64,
    /// Times the open streams were told to read their state again, and subscriptions lost.
    pub resyncs: AtomicU64,
    pub subscription_drops: AtomicU64,
}

pub fn bump(counter: &AtomicU64) {
    counter.fetch_add(1, Relaxed);
}

pub fn add(counter: &AtomicU64, n: u64) {
    if n > 0 {
        counter.fetch_add(n, Relaxed);
    }
}

impl Metrics {
    pub fn render(&self, connections: usize, subscribed: bool) -> String {
        let mut out = String::with_capacity(2048);
        let mut put = |kind: &str, name: &str, help: &str, value: u64| {
            let _ = writeln!(out, "# HELP {name} {help}\n# TYPE {name} {kind}\n{name} {value}");
        };
        put("gauge", "realtime_connections", "Streams open on this instance.", connections as u64);
        put("gauge", "realtime_subscribed", "1 when the channel subscription is active.", u64::from(subscribed));
        put("counter", "realtime_opened_total", "Streams opened.", self.opened.load(Relaxed));
        put("counter", "realtime_rejected_full_total", "Streams refused because the instance is full.", self.rejected_full.load(Relaxed));
        put("counter", "realtime_rejected_auth_total", "Streams refused because of the token.", self.rejected_auth.load(Relaxed));
        put("counter", "realtime_events_received_total", "Messages read from the channel.", self.events_received.load(Relaxed));
        put("counter", "realtime_events_refused_total", "Messages of the channel that were not valid.", self.events_refused.load(Relaxed));
        put("counter", "realtime_frames_delivered_total", "Frames put in the queue of a stream.", self.frames_delivered.load(Relaxed));
        put("counter", "realtime_renew_batches_total", "Batches of session traces read.", self.renew_batches.load(Relaxed));
        put("counter", "realtime_renew_errors_total", "Batches of session traces that could not be read.", self.renew_errors.load(Relaxed));
        put("counter", "realtime_resyncs_total", "Times the open streams were told to read their state again.", self.resyncs.load(Relaxed));
        put("counter", "realtime_subscription_drops_total", "Subscriptions to the channel that were lost.", self.subscription_drops.load(Relaxed));
        let _ = writeln!(out, "# HELP realtime_closed_total Streams closed, by reason.\n# TYPE realtime_closed_total counter");
        for (reason, counter) in [
            ("client", &self.closed_client),
            ("slow", &self.closed_slow),
            ("expired", &self.closed_expired),
            ("revoked", &self.closed_revoked),
            ("evicted", &self.closed_evicted),
        ] {
            let _ = writeln!(out, "realtime_closed_total{{reason=\"{reason}\"}} {}", counter.load(Relaxed));
        }
        out
    }
}
