//! The whole service against a real DragonflyDB (or Valkey, or Redis), over real sockets.
//!
//! `REALTIME_TEST_DRAGONFLY_URL=redis://localhost:6379 cargo test -p <service> --test dragonfly -- --ignored`

use std::{sync::Arc, time::Duration};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::SigningKey;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode, jwk::JwkSet};
use realtime_service::{
    config::Config,
    contract::{ACCESS_COOKIE, CHANNEL, ISSUER, PATH, SESSION_PREFIX},
    http, hub, server, sessions, sweeper,
};
use serde_json::json;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};
use uuid::Uuid;

const PKCS8_ED25519_PREFIX: [u8; 16] = [0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20];

fn url() -> String {
    std::env::var("REALTIME_TEST_DRAGONFLY_URL").expect("REALTIME_TEST_DRAGONFLY_URL")
}

fn jwks() -> JwkSet {
    let public = SigningKey::from_bytes(&[7; 32]).verifying_key().to_bytes();
    serde_json::from_value(json!({"keys": [{"kty": "OKP", "crv": "Ed25519", "use": "sig", "alg": "EdDSA", "kid": "k", "x": URL_SAFE_NO_PAD.encode(public)}]})).unwrap()
}

fn token(account: Uuid, tenant: Uuid, session: Uuid, exp: i64, permissions: &[&str]) -> String {
    let mut pkcs8 = PKCS8_ED25519_PREFIX.to_vec();
    pkcs8.extend_from_slice(&[7; 32]);
    let mut header = Header::new(Algorithm::EdDSA);
    header.kid = Some("k".into());
    let claims = json!({"sub": account, "tid": tenant, "sid": session, "exp": exp, "iss": ISSUER, "permissions": permissions});
    encode(&header, &claims, &EncodingKey::from_ed_der(&pkcs8)).unwrap()
}

struct Running {
    addr: std::net::SocketAddr,
    state: server::AppState,
}

/// The service on an ephemeral port, with the subscription and the sweeper running.
async fn start(configure: impl FnOnce(&mut Config)) -> Running {
    let mut config = Config::for_tests();
    config.dragonfly_url = url();
    configure(&mut config);
    let verifier = Arc::new(realtime_service::jwt::AccessVerifier::with_jwks(config.jwks_url.clone(), &jwks()));
    let state = server::AppState::with_verifier(config.clone(), verifier);
    hub::spawn_subscriber(&config.dragonfly_url, state.hub.clone(), state.registry.clone(), state.metrics.clone(), config.resync_spread).unwrap();
    sweeper::spawn(state.registry.clone(), sessions::Sessions::new(&config.dragonfly_url).unwrap(), state.metrics.clone(), config.renew_grace_secs);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(http::serve(listener, state.clone()));
    // The subscription is active when the service says it is ready.
    for _ in 0..100 {
        if state.hub.is_subscribed() {
            return Running { addr, state };
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the subscription never became active");
}

/// An open stream, read as the browser would.
struct Stream {
    socket: TcpStream,
    text: String,
}

impl Stream {
    async fn open(addr: std::net::SocketAddr, token: &str) -> Stream {
        let mut socket = TcpStream::connect(addr).await.unwrap();
        let request = format!("GET {PATH} HTTP/1.1\r\nHost: x\r\nCookie: {ACCESS_COOKIE}={token}\r\nAccept: text/event-stream\r\n\r\n");
        socket.write_all(request.as_bytes()).await.unwrap();
        let mut stream = Stream { socket, text: String::new() };
        stream.until("connected").await;
        stream
    }

    /// Reads until `needle` shows up (or 3 seconds pass); everything read is kept in `text`.
    async fn until(&mut self, needle: &str) -> bool {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        let mut buf = [0u8; 4096];
        while !self.text.contains(needle) {
            let Ok(Ok(n)) = tokio::time::timeout_at(deadline, self.socket.read(&mut buf)).await else { return false };
            if n == 0 {
                return false;
            }
            self.text.push_str(&String::from_utf8_lossy(&buf[..n]));
        }
        true
    }

    /// Whether the server closed the stream within `secs` seconds.
    async fn closed_within(&mut self, secs: u64) -> bool {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
        let mut buf = [0u8; 4096];
        loop {
            match tokio::time::timeout_at(deadline, self.socket.read(&mut buf)).await {
                Ok(Ok(0)) | Ok(Err(_)) => return true,
                Ok(Ok(_)) => {}
                Err(_) => return false,
            }
        }
    }

    async fn closed(&mut self) -> bool {
        self.closed_within(3).await
    }
}

async fn publish(message: serde_json::Value) {
    let client = redis::Client::open(url()).unwrap();
    let mut connection = client.get_multiplexed_async_connection().await.unwrap();
    let _: i64 = redis::cmd("PUBLISH").arg(CHANNEL).arg(message.to_string()).query_async(&mut connection).await.unwrap();
}

async fn trace(session: Uuid, account: Uuid, tenant: Uuid, exp: i64, ttl: u64) {
    let client = redis::Client::open(url()).unwrap();
    let mut connection = client.get_multiplexed_async_connection().await.unwrap();
    let key = format!("{SESSION_PREFIX}:{session}");
    let value = json!({"accountId": account, "tenantId": tenant, "exp": exp}).to_string();
    let _: () = redis::cmd("SET").arg(key).arg(value).arg("EX").arg(ttl).query_async(&mut connection).await.unwrap();
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

#[tokio::test]
#[ignore = "needs REALTIME_TEST_DRAGONFLY_URL"]
async fn an_event_published_on_the_channel_reaches_the_streams_it_is_for_and_only_them() {
    let app = start(|_| {}).await;
    let tenant = Uuid::new_v4();
    let open = |tenant, permissions: &'static [&'static str]| {
        let t = token(Uuid::new_v4(), tenant, Uuid::new_v4(), now() + 300, permissions);
        Stream::open(app.addr, Box::leak(t.into_boxed_str()))
    };
    let mut allowed = open(tenant, &["contacts.view"]).await;
    let mut no_right = open(tenant, &["tasks.view"]).await;
    let elsewhere = open(Uuid::new_v4(), &["contacts.view"]).await;

    publish(json!({"type": "contact.updated", "tenantId": tenant, "permission": "contacts.view", "data": {"id": "c1"}})).await;
    assert!(allowed.until("event: contact.updated\ndata: {\"id\":\"c1\"}\n\n").await, "{}", allowed.text);
    // A marker published after: seeing it without the event proves the event did not skip the line.
    publish(json!({"type": "marker.sent", "tenantId": tenant, "permission": "tasks.view"})).await;
    assert!(no_right.until("marker.sent").await);
    assert!(!no_right.text.contains("contact.updated"));
    assert!(!elsewhere.text.contains("contact.updated"));
    assert_eq!(app.state.registry.len(), 3);
}

#[tokio::test]
#[ignore = "needs REALTIME_TEST_DRAGONFLY_URL"]
async fn a_missing_or_bad_token_is_a_401_and_a_full_instance_a_503_with_a_wait() {
    let app = start(|c| c.max_connections = 1).await;
    let get = |cookie: String| async move {
        let mut socket = TcpStream::connect(app.addr).await.unwrap();
        socket.write_all(format!("GET {PATH} HTTP/1.1\r\nHost: x\r\n{cookie}Connection: close\r\n\r\n").as_bytes()).await.unwrap();
        let mut text = String::new();
        socket.read_to_string(&mut text).await.ok();
        text
    };
    assert!(get(String::new()).await.starts_with("HTTP/1.1 401"));
    assert!(get(format!("Cookie: {ACCESS_COOKIE}=garbage\r\n")).await.starts_with("HTTP/1.1 401"));

    let t = |account| token(account, Uuid::new_v4(), Uuid::new_v4(), now() + 300, &[]);
    let _first = Stream::open(app.addr, &t(Uuid::new_v4())).await;
    let refused = get(format!("Cookie: {ACCESS_COOKIE}={}\r\n", t(Uuid::new_v4()))).await.to_lowercase();
    assert!(refused.starts_with("http/1.1 503") && refused.contains("retry-after:"), "{refused}");
}

#[tokio::test]
#[ignore = "needs REALTIME_TEST_DRAGONFLY_URL"]
async fn losing_the_subscription_tells_the_streams_to_read_their_state_and_closes_none() {
    let app = start(|_| {}).await;
    let tenant = Uuid::new_v4();
    let t = token(Uuid::new_v4(), tenant, Uuid::new_v4(), now() + 300, &["contacts.view"]);
    let mut stream = Stream::open(app.addr, &t).await;

    // Cut the subscription from the server side, as a restart or a network blip would.
    let client = redis::Client::open(url()).unwrap();
    let mut admin = client.get_multiplexed_async_connection().await.unwrap();
    // (`CLIENT KILL TYPE` is not in every server: list the clients, they are the ones with the `P` flag, and kill them by id.)
    let list: String = redis::cmd("CLIENT").arg("LIST").query_async(&mut admin).await.unwrap();
    let subscribers: Vec<&str> = list
        .lines()
        .filter(|line| line.split(' ').any(|field| field == "flags=P"))
        .filter_map(|line| line.split(' ').find_map(|field| field.strip_prefix("id=")))
        .collect();
    assert!(!subscribers.is_empty(), "{list}");
    for id in subscribers {
        let _: redis::Value = redis::cmd("CLIENT").arg("KILL").arg("ID").arg(id).query_async(&mut admin).await.unwrap();
    }

    assert!(stream.until("event: resync\ndata: {}\n\n").await, "{}", stream.text);
    assert_eq!(app.state.registry.len(), 1, "the stream is still open");
    // And it works after: the subscription came back.
    for _ in 0..100 {
        if app.state.hub.is_subscribed() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    publish(json!({"type": "contact.updated", "tenantId": tenant, "permission": "contacts.view"})).await;
    assert!(stream.until("event: contact.updated").await, "{}", stream.text);
}

#[tokio::test]
#[ignore = "needs REALTIME_TEST_DRAGONFLY_URL"]
async fn a_stream_outlives_its_token_while_the_session_is_renewed_and_ends_when_it_is_not() {
    let app = start(|_| {}).await;
    let (account, tenant, session) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    // The token of this stream is about to expire; the API has renewed the session since.
    let t = token(account, tenant, session, now() + 2, &["contacts.view"]);
    trace(session, account, tenant, now() + 600, 600).await;
    let mut alive = Stream::open(app.addr, &t).await;

    let (account2, session2) = (Uuid::new_v4(), Uuid::new_v4());
    let t2 = token(account2, tenant, session2, now() + 2, &["contacts.view"]);
    let mut revoked = Stream::open(app.addr, &t2).await;
    // No trace for session2: revoked.

    // The sweeper visits every stream within 15 seconds.
    assert!(revoked.closed_within(20).await, "{}", revoked.text);
    publish(json!({"type": "contact.updated", "tenantId": tenant, "permission": "contacts.view"})).await;
    assert!(alive.until("event: contact.updated").await, "{}", alive.text);
    assert_eq!(app.state.registry.len(), 1);
}

#[tokio::test]
#[ignore = "needs REALTIME_TEST_DRAGONFLY_URL"]
async fn the_rights_of_an_account_change_in_flight_and_a_removed_member_loses_the_stream() {
    let app = start(|_| {}).await;
    let (account, tenant) = (Uuid::new_v4(), Uuid::new_v4());
    let t = token(account, tenant, Uuid::new_v4(), now() + 300, &["tasks.view"]);
    let mut stream = Stream::open(app.addr, &t).await;

    publish(json!({"type": "grants.updated", "tenantId": tenant, "accountId": account, "permissions": ["contacts.view"]})).await;
    assert!(stream.until("event: grants.updated").await);
    publish(json!({"type": "contact.updated", "tenantId": tenant, "permission": "contacts.view"})).await;
    assert!(stream.until("event: contact.updated").await);

    publish(json!({"type": "grants.updated", "tenantId": tenant, "accountId": account})).await;
    assert!(stream.closed().await);
    assert_eq!(app.state.registry.len(), 0);
}
