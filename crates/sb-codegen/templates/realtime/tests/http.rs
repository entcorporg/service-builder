//! The service over real sockets, with no key-value server: events are handed to the registry directly.

use std::{sync::Arc, time::Duration};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::SigningKey;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode, jwk::JwkSet};
use realtime_service::{
    config::Config,
    contract::{ACCESS_COOKIE, ISSUER, PATH},
    events::{Audience, Data, LiveEvent},
    http,
    jwt::AccessVerifier,
    server::AppState,
};
use serde_json::json;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};
use uuid::Uuid;

const PKCS8_ED25519_PREFIX: [u8; 16] = [0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20];

fn jwks() -> JwkSet {
    let public = SigningKey::from_bytes(&[3; 32]).verifying_key().to_bytes();
    serde_json::from_value(json!({"keys": [{"kty": "OKP", "crv": "Ed25519", "use": "sig", "alg": "EdDSA", "kid": "k", "x": URL_SAFE_NO_PAD.encode(public)}]})).unwrap()
}

fn token(tenant: Uuid, exp_offset: i64, permissions: &[&str]) -> String {
    let mut pkcs8 = PKCS8_ED25519_PREFIX.to_vec();
    pkcs8.extend_from_slice(&[3; 32]);
    let mut header = Header::new(Algorithm::EdDSA);
    header.kid = Some("k".into());
    let claims = json!({
        "sub": Uuid::new_v4(), "tid": tenant, "sid": Uuid::new_v4(), "iss": ISSUER,
        "exp": chrono::Utc::now().timestamp() + exp_offset, "permissions": permissions,
    });
    encode(&header, &claims, &EncodingKey::from_ed_der(&pkcs8)).unwrap()
}

async fn start(configure: impl FnOnce(&mut Config)) -> (std::net::SocketAddr, AppState) {
    let mut config = Config::for_tests();
    configure(&mut config);
    let verifier = Arc::new(AccessVerifier::with_jwks(config.jwks_url.clone(), &jwks()));
    let state = AppState::with_verifier(config, verifier);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(http::serve(listener, state.clone()));
    (addr, state)
}

/// One request, read to the end: the service always closes the connection after the response.
async fn request(addr: std::net::SocketAddr, head: &str) -> String {
    let mut socket = TcpStream::connect(addr).await.unwrap();
    socket.write_all(head.as_bytes()).await.unwrap();
    let mut text = String::new();
    tokio::time::timeout(Duration::from_secs(3), socket.read_to_string(&mut text)).await.expect("the connection is closed after the response").ok();
    text
}

/// An open stream, read as the browser would.
struct Stream {
    socket: TcpStream,
    text: String,
}

impl Stream {
    async fn open(addr: std::net::SocketAddr, token: &str, extra: &str) -> Stream {
        let mut socket = TcpStream::connect(addr).await.unwrap();
        let head = format!("GET {PATH} HTTP/1.1\r\nHost: x\r\nCookie: {ACCESS_COOKIE}={token}\r\n{extra}\r\n");
        socket.write_all(head.as_bytes()).await.unwrap();
        let mut stream = Stream { socket, text: String::new() };
        assert!(stream.until(": connected").await, "{}", stream.text);
        stream
    }

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

    /// Whether the server closed the connection within `secs` seconds.
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
}

fn contact_updated(tenant: Uuid) -> LiveEvent {
    LiveEvent {
        kind: "contact.updated".into(),
        audience: Audience::Tenant { tenant_id: tenant, permission: Some("contacts.view".into()), accounts: vec![] },
        origin: None,
        data: Data::from([("id".to_string(), "c1".to_string())]),
    }
}

#[tokio::test]
async fn a_stream_is_a_response_without_a_length_that_the_events_are_written_to() {
    let (addr, state) = start(|_| {}).await;
    let tenant = Uuid::new_v4();
    let mut stream = Stream::open(addr, &token(tenant, 300, &["contacts.view"]), "").await;
    let head = stream.text.to_lowercase();
    assert!(head.starts_with("http/1.1 200 ok\r\n"), "{head}");
    for line in ["content-type: text/event-stream", "cache-control: no-cache, no-transform", "x-accel-buffering: no", "connection: close"] {
        assert!(head.contains(line), "{line}: {head}");
    }
    assert!(!head.contains("content-length") && !head.contains("transfer-encoding"), "the stream is the body: {head}");
    assert!(head.contains("retry: 2000\n"), "{head}");

    state.registry.dispatch(&contact_updated(tenant));
    assert!(stream.until("event: contact.updated\ndata: {\"id\":\"c1\"}\n\n").await, "{}", stream.text);
}

#[tokio::test]
async fn a_system_without_companies_works_with_tokens_that_have_no_tid() {
    let (addr, state) = start(|_| {}).await;
    let mut pb8 = PKCS8_ED25519_PREFIX.to_vec();
    pb8.extend_from_slice(&[3; 32]);
    let mut header = Header::new(Algorithm::EdDSA);
    header.kid = Some("k".into());
    // No `tid`, no `permissions`: just an account and a session.
    let account = Uuid::new_v4();
    let claims = json!({"sub": account, "sid": Uuid::new_v4(), "iss": ISSUER, "exp": chrono::Utc::now().timestamp() + 300});
    let bare = encode(&header, &claims, &EncodingKey::from_ed_der(&pb8)).unwrap();
    let mut stream = Stream::open(addr, &bare, "").await;

    let named = LiveEvent {
        kind: "news.posted".into(),
        audience: Audience::Tenant { tenant_id: realtime_service::events::NO_TENANT, permission: None, accounts: vec![account] },
        origin: None,
        data: Data::new(),
    };
    state.registry.dispatch(&named);
    assert!(stream.until("event: news.posted").await, "{}", stream.text);
    // An event for a company is not for it.
    state.registry.dispatch(&contact_updated(Uuid::new_v4()));
    state.registry.dispatch(&LiveEvent { kind: "marker.sent".into(), ..named });
    assert!(stream.until("marker.sent").await);
    assert!(!stream.text.contains("contact.updated"));
}

#[tokio::test]
async fn the_connection_ends_with_the_stream_and_the_stream_with_the_connection() {
    let (addr, state) = start(|_| {}).await;
    let tenant = Uuid::new_v4();
    // The server ends the stream (a session that is over): the connection closes, it is not kept for another request.
    let mut ended = Stream::open(addr, &token(tenant, 300, &[]), "").await;
    assert_eq!(state.registry.len(), 1);
    state.registry.expire(state.registry.ids()[0]);
    assert!(ended.closed_within(3).await);
    assert_eq!(state.registry.len(), 0);
    // The client goes away: noticed at once, not at the next heartbeat.
    let gone = Stream::open(addr, &token(tenant, 300, &[]), "").await;
    assert_eq!(state.registry.len(), 1);
    drop(gone);
    for _ in 0..50 {
        if state.registry.is_empty() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the stream of a client that left is still registered");
}

#[tokio::test]
async fn a_stream_that_does_not_read_is_closed_when_its_queue_is_full_and_the_others_are_not_held_up() {
    let (addr, state) = start(|c| c.queue = 4).await;
    let tenant = Uuid::new_v4();
    let head = |t: &str| format!("GET {PATH} HTTP/1.1\r\nHost: x\r\nCookie: {ACCESS_COOKIE}={t}\r\n\r\n");

    // A reader that keeps up, drained by a task.
    let reader = Stream::open(addr, &token(tenant, 300, &["contacts.view"]), "").await;
    let (mut read_half, _write_half) = reader.socket.into_split();
    let drained = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = drained.clone();
    tokio::spawn(async move {
        let mut buf = vec![0u8; 65536];
        while let Ok(n) = read_half.read(&mut buf).await {
            if n == 0 {
                break;
            }
            counter.fetch_add(n, std::sync::atomic::Ordering::Relaxed);
        }
    });

    // One that never reads, with the smallest receive buffer: its socket fills, the writer waits, its queue fills.
    let socket = tokio::net::TcpSocket::new_v4().unwrap();
    socket.set_recv_buffer_size(2048).unwrap();
    let mut stuck = socket.connect(addr).await.unwrap();
    stuck.write_all(head(&token(tenant, 300, &["contacts.view"])).as_bytes()).await.unwrap();
    for _ in 0..100 {
        if state.registry.len() == 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(state.registry.len(), 2);

    // One event at a time, as they come from the channel, with the writers getting their turn in between.
    let started = std::time::Instant::now();
    while state.registry.len() == 2 && started.elapsed() < Duration::from_secs(20) {
        state.registry.dispatch(&contact_updated(tenant));
        tokio::task::yield_now().await;
    }
    assert_eq!(state.registry.len(), 1, "only the reader is left");
    assert_eq!(state.metrics.closed_slow.load(std::sync::atomic::Ordering::Relaxed), 1);
    assert!(drained.load(std::sync::atomic::Ordering::Relaxed) > 0, "the reader kept getting its events");
    drop(stuck);
}

#[tokio::test]
async fn a_client_that_floods_an_open_stream_is_cut_off() {
    let (addr, state) = start(|_| {}).await;
    let mut stream = Stream::open(addr, &token(Uuid::new_v4(), 300, &[]), "").await;
    // A few stray bytes are let through; a flood is not.
    stream.socket.write_all(b"\r\n").await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(state.registry.len(), 1);
    stream.socket.write_all(&vec![b'x'; 4096]).await.unwrap();
    assert!(stream.closed_within(3).await);
    assert_eq!(state.registry.len(), 0);
}

#[tokio::test]
async fn refusals_are_plain_responses_that_close() {
    let (addr, _state) = start(|c| c.max_connections = 1).await;
    let get = |extra: &str| format!("GET {PATH} HTTP/1.1\r\nHost: x\r\n{extra}\r\n");

    let no_token = request(addr, &get("")).await;
    assert!(no_token.starts_with("HTTP/1.1 401 Unauthorized\r\n") && no_token.ends_with("{\"message\":\"Unauthorized\"}"), "{no_token}");
    let bad = request(addr, &get(&format!("Cookie: {ACCESS_COOKIE}=garbage\r\n"))).await;
    assert!(bad.starts_with("HTTP/1.1 401"), "{bad}");

    let t = |tenant| format!("Cookie: {ACCESS_COOKIE}={}\r\n", token(tenant, 300, &[]));
    let _first = Stream::open(addr, &token(Uuid::new_v4(), 300, &[]), "").await;
    let full = request(addr, &get(&t(Uuid::new_v4()))).await.to_lowercase();
    assert!(full.starts_with("http/1.1 503") && full.contains("retry-after: "), "{full}");

    let post = request(addr, "POST /v1/events HTTP/1.1\r\nHost: x\r\nContent-Length: 0\r\n\r\n").await;
    assert!(post.starts_with("HTTP/1.1 405"), "{post}");
    let unknown = request(addr, "GET /admin HTTP/1.1\r\nHost: x\r\n\r\n").await;
    assert!(unknown.starts_with("HTTP/1.1 404"), "{unknown}");
    let junk = request(addr, "\u{16}\u{3}\u{1}\u{2}\u{0}\u{1}\r\n\r\n").await;
    assert!(junk.starts_with("HTTP/1.1 400"), "{junk}");
    let huge = request(addr, &format!("GET {PATH} HTTP/1.1\r\nX-Padding: {}\r\n\r\n", "a".repeat(20_000))).await;
    assert!(huge.starts_with("HTTP/1.1 431"), "{}", &huge[..huge.len().min(60)]);
}

#[tokio::test]
async fn cors_headers_are_for_the_listed_origins_only_and_the_preflight_is_answered() {
    let (addr, _state) = start(|_| {}).await;
    let tenant = Uuid::new_v4();
    let listed = Stream::open(addr, &token(tenant, 300, &[]), "Origin: http://localhost:5173\r\n").await;
    let head = listed.text.to_lowercase();
    assert!(head.contains("access-control-allow-origin: http://localhost:5173\r\n") && head.contains("access-control-allow-credentials: true\r\n"), "{head}");
    let other = Stream::open(addr, &token(tenant, 300, &[]), "Origin: https://evil.example.com\r\n").await;
    assert!(!other.text.to_lowercase().contains("access-control-allow-origin"), "{}", other.text);

    let preflight = request(addr, &format!("OPTIONS {PATH} HTTP/1.1\r\nHost: x\r\nOrigin: http://localhost:5173\r\nAccess-Control-Request-Method: GET\r\n\r\n")).await.to_lowercase();
    assert!(preflight.starts_with("http/1.1 204"), "{preflight}");
    assert!(preflight.contains("access-control-allow-origin: http://localhost:5173") && preflight.contains("access-control-allow-methods: get, options"), "{preflight}");
}

#[tokio::test]
async fn health_and_metrics_are_on_the_same_port() {
    let (addr, state) = start(|_| {}).await;
    assert!(request(addr, "GET /livez HTTP/1.1\r\nHost: x\r\n\r\n").await.starts_with("HTTP/1.1 200"));
    // No subscription to the channel in this test: not ready.
    assert!(request(addr, "GET /readyz HTTP/1.1\r\nHost: x\r\n\r\n").await.starts_with("HTTP/1.1 503"));

    assert!(request(addr, "GET /metrics HTTP/1.1\r\nHost: x\r\n\r\n").await.starts_with("HTTP/1.1 401"));
    assert!(request(addr, "GET /metrics HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer nope\r\n\r\n").await.starts_with("HTTP/1.1 401"));
    let _stream = Stream::open(addr, &token(Uuid::new_v4(), 300, &[]), "").await;
    let metrics = request(addr, "GET /metrics HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer secret\r\n\r\n").await;
    assert!(metrics.starts_with("HTTP/1.1 200"), "{metrics}");
    assert!(metrics.contains("realtime_connections 1\n") && metrics.contains("realtime_opened_total 1\n"), "{metrics}");
    assert_eq!(state.registry.len(), 1);
}

#[tokio::test]
async fn a_client_that_never_finishes_its_request_does_not_hold_a_registration() {
    let (addr, state) = start(|_| {}).await;
    let mut socket = TcpStream::connect(addr).await.unwrap();
    socket.write_all(b"GET /v1/events HTTP/1.1\r\nHost: x\r\n").await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(state.registry.len(), 0, "nothing is registered before a valid request");
}
