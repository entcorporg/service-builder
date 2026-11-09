//! The subscription to the channel, and what comes of a message.
//!
//! One subscription per instance feeds the [registry](crate::registry), which hands each event to its recipients.
//! Pub/sub keeps nothing: an event published while the subscription is down is lost. Two rules follow. When the
//! subscription comes back, the open streams are *told* to read their state again (the `resync` event, spread over a few
//! seconds) and are not closed: closing 50 000 streams at once would make 50 000 browsers reconnect, and verify a token,
//! at once. And a browser reads its state each time it opens a stream.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use futures::StreamExt;
use redis::aio::{PubSubSink, PubSubStream};
use tokio::time::MissedTickBehavior;

use crate::{
    contract::CHANNEL,
    events::Message,
    metrics::{Metrics, bump},
    registry::Registry,
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// A connection that died without closing produces nothing and never ends: a regular PING shows it.
const PING_INTERVAL: Duration = Duration::from_secs(30);
const PING_TIMEOUT: Duration = Duration::from_secs(5);
const MIN_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(30);
/// Streams told at a time while spreading a resync.
const RESYNC_CHUNK: usize = 500;
/// Messages relayed before the task gives the others their turn: a burst read in one go from the socket would otherwise be
/// queued for the streams before their writers could run once, and fill the queues of streams that are not slow at all.
const YIELD_EVERY: u32 = 64;

/// Whether the subscription is active (readiness: without it the streams would stay open and silent).
#[derive(Debug, Default)]
pub struct Hub {
    subscribed: AtomicBool,
}

impl Hub {
    pub fn is_subscribed(&self) -> bool {
        self.subscribed.load(Ordering::Relaxed)
    }
}

/// Starts the subscription for the whole life of the process, reconnecting (with a growing wait) after a failure or a
/// cut. It never blocks start-up; until it is active the instance is not ready (`/readyz`).
pub fn spawn_subscriber(
    url: &str,
    hub: Arc<Hub>,
    registry: Arc<Registry>,
    metrics: Arc<Metrics>,
    resync_spread: Duration,
) -> anyhow::Result<()> {
    let client = redis::Client::open(url)?;
    tokio::spawn(async move {
        let mut backoff = MIN_BACKOFF;
        loop {
            match subscribe(&client).await {
                Ok((sink, messages)) => {
                    tracing::info!(channel = CHANNEL, "subscription active");
                    backoff = MIN_BACKOFF;
                    hub.subscribed.store(true, Ordering::Relaxed);
                    // What was published before this subscription (start-up, a cut) escaped the streams already open.
                    if !registry.is_empty() {
                        bump(&metrics.resyncs);
                        tokio::spawn(resync_all(registry.clone(), resync_spread));
                    }
                    relay(sink, messages, &registry, &metrics).await;
                    hub.subscribed.store(false, Ordering::Relaxed);
                    bump(&metrics.subscription_drops);
                    tracing::warn!(channel = CHANNEL, "subscription lost");
                }
                Err(err) => tracing::warn!(error = ?err, channel = CHANNEL, "subscription failed"),
            }
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(MAX_BACKOFF);
        }
    });
    Ok(())
}

async fn subscribe(client: &redis::Client) -> anyhow::Result<(PubSubSink, PubSubStream)> {
    let pubsub = tokio::time::timeout(CONNECT_TIMEOUT, client.get_async_pubsub()).await??;
    let (mut sink, messages) = pubsub.split();
    tokio::time::timeout(CONNECT_TIMEOUT, sink.subscribe(CHANNEL)).await??;
    Ok((sink, messages))
}

/// Relays the messages until the connection is lost.
async fn relay(mut sink: PubSubSink, mut messages: PubSubStream, registry: &Registry, metrics: &Metrics) {
    let mut ping = tokio::time::interval(PING_INTERVAL);
    ping.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut since_yield = 0u32;
    loop {
        tokio::select! {
            message = messages.next() => {
                let Some(message) = message else { return };
                handle(message.get_payload_bytes(), registry, metrics);
                since_yield += 1;
                if since_yield >= YIELD_EVERY {
                    since_yield = 0;
                    tokio::task::yield_now().await;
                }
            }
            _ = ping.tick() => {
                let pong = tokio::time::timeout(PING_TIMEOUT, sink.ping::<redis::Value>()).await;
                if !matches!(pong, Ok(Ok(_))) {
                    return;
                }
            }
        }
    }
}

/// One message of the channel: an invalid one is logged and ignored.
pub fn handle(payload: &[u8], registry: &Registry, metrics: &Metrics) {
    bump(&metrics.events_received);
    match Message::parse(payload) {
        Ok(Message::Event(event)) => {
            registry.dispatch(&event);
        }
        Ok(Message::GrantsUpdated { tenant_id, account_id, permissions }) => {
            registry.grants_updated(tenant_id, account_id, permissions);
        }
        Err(refused) => {
            bump(&metrics.events_refused);
            tracing::warn!(error = %refused, "message ignored");
        }
    }
}

/// Tells every open stream to read its state again, `RESYNC_CHUNK` at a time over `spread`, so that their browsers do not
/// all call the API in the same instant.
pub async fn resync_all(registry: Arc<Registry>, spread: Duration) {
    let ids = registry.ids();
    let chunks = ids.len().div_ceil(RESYNC_CHUNK).max(1);
    let pause = spread / chunks as u32;
    for chunk in ids.chunks(RESYNC_CHUNK) {
        registry.resync(chunk);
        tokio::time::sleep(pause).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{Limits, Who};
    use std::sync::atomic::Ordering::Relaxed;
    use uuid::Uuid;

    #[tokio::test]
    async fn a_valid_message_reaches_its_stream_and_garbage_is_counted_and_ignored() {
        let metrics = Arc::new(Metrics::default());
        let registry = Registry::new(Limits { max_connections: 10, max_per_account: 5, queue: 8 }, metrics.clone());
        let tenant = Uuid::new_v4();
        let mut stream = registry
            .register(Who { account: Uuid::new_v4(), tenant, session: Uuid::new_v4(), client: None, permissions: vec!["contacts.view".into()], deadline: 0 })
            .unwrap();
        handle(b"not json", &registry, &metrics);
        handle(serde_json::json!({"type": "contact.updated", "tenantId": tenant, "permission": "contacts.view"}).to_string().as_bytes(), &registry, &metrics);
        assert_eq!((metrics.events_received.load(Relaxed), metrics.events_refused.load(Relaxed)), (2, 1));
        let frame = tokio::time::timeout(Duration::from_millis(50), futures::StreamExt::next(&mut stream)).await.unwrap().unwrap();
        assert_eq!(&frame[..], b"event: contact.updated\ndata: {}\n\n");
    }

    #[tokio::test(start_paused = true)]
    async fn a_resync_is_spread_over_the_time_given() {
        let metrics = Arc::new(Metrics::default());
        let registry = Registry::new(Limits { max_connections: 2000, max_per_account: 2000, queue: 8 }, metrics);
        let streams: Vec<_> = (0..1200)
            .map(|_| registry.register(Who { account: Uuid::new_v4(), tenant: Uuid::new_v4(), session: Uuid::new_v4(), client: None, permissions: vec![], deadline: 0 }).unwrap())
            .collect();
        let start = tokio::time::Instant::now();
        resync_all(registry.clone(), Duration::from_secs(9)).await;
        // 1 200 streams are 3 chunks of 500 at most: 3 pauses of 3 s.
        assert_eq!(start.elapsed(), Duration::from_secs(9));
        assert_eq!(registry.len(), 1200, "no stream was closed");
        drop(streams);
    }
}
