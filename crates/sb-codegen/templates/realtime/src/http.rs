//! The HTTP side: just enough HTTP/1.1 for this service, written for streams that stay open for hours.
//!
//! A general HTTP server keeps buffers for every connection (hyper: 8 KiB to read and 8 KiB for the response head, whatever
//! the settings) and a state machine for the next request: about 28 KiB a stream, 1.4 GiB for 50 000. A stream here is a
//! socket and a task that waits for the next frame, a few KiB, because there is nothing else to remember:
//!
//! - One request per connection, always answered with `Connection: close`: no keep-alive, no pipelining, no request
//!   smuggling, and a stream that ends takes its connection with it (nothing is left idle holding a socket).
//! - The request is read into a buffer that is freed before the stream starts, parsed with `httparse`, at most 8 KiB and
//!   10 seconds. Only `GET` (and the `OPTIONS` preflight of the stream path) is understood; there is no body to read.
//! - The response to a stream has no length: the stream is the body, ended by closing the connection (RFC 9112 §6.3).
//! - Nothing waits on a client for ever: a write that takes more than 10 seconds ends the stream, and the client closing
//!   is noticed at once, not at the next heartbeat.
//!
//! TLS is the business of the proxy in front (Traefik, nginx, a load balancer).

use std::{io, time::Duration};

use bytes::Bytes;
use futures::StreamExt;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use uuid::Uuid;

use crate::{
    contract::{ACCESS_COOKIE, PATH},
    errors::AppError,
    events::{MAX_PERMISSIONS, NO_TENANT},
    metrics::bump,
    registry::{Refusal, Who},
    server::AppState,
};

/// A request is its head: more than this is refused (431).
const MAX_REQUEST: usize = 8 * 1024;
const MAX_HEADERS: usize = 48;
/// A client that does not send its request in this time is dropped (slowloris).
const HEADER_TIMEOUT: Duration = Duration::from_secs(10);
/// A client that does not take what is written to it in this time is dropped.
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);
/// What a stream may be sent by its client once it is open: nothing, an `EventSource` never does. A little is let through
/// (a stray byte, a pipelined request nobody reads), a flood is not: it would keep a core busy reading it.
const MAX_SENT_TO_A_STREAM: usize = 1024;

/// The delay the browser is asked to wait before it reconnects, and a first line, so that the response is seen to start by
/// the proxies that wait for a first byte.
const PRELUDE: &[u8] = b"retry: 2000\n: connected\n\n";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    Stream,
    Livez,
    Readyz,
    Metrics,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Options,
}

/// What the service reads of a request.
#[derive(Debug, PartialEq, Eq)]
pub struct Request {
    pub method: Method,
    pub route: Route,
    /// `?client=`: the tab, if it is a UUID.
    pub client: Option<Uuid>,
    /// The value of the access cookie.
    pub token: Option<String>,
    pub origin: Option<String>,
    /// The bearer token, read for `/metrics` only.
    pub bearer: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Bad {
    Malformed,
    TooLarge,
    Method,
}

/// Parses the head of a request. `Ok(None)`: not complete yet.
pub fn parse(buf: &[u8], stream_path: &str, cookie_name: &str) -> Result<Option<Request>, Bad> {
    let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
    let mut req = httparse::Request::new(&mut headers);
    match req.parse(buf) {
        Ok(httparse::Status::Complete(_)) => {}
        Ok(httparse::Status::Partial) => return Ok(None),
        Err(httparse::Error::TooManyHeaders) => return Err(Bad::TooLarge),
        Err(_) => return Err(Bad::Malformed),
    }
    let method = match req.method {
        Some("GET") => Method::Get,
        Some("OPTIONS") => Method::Options,
        _ => return Err(Bad::Method),
    };
    let target = req.path.ok_or(Bad::Malformed)?;
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let route = if path == stream_path {
        Route::Stream
    } else {
        match path {
            "/livez" => Route::Livez,
            "/readyz" => Route::Readyz,
            "/metrics" => Route::Metrics,
            _ => Route::Other,
        }
    };
    let client = query.split('&').find_map(|pair| pair.strip_prefix("client=")).and_then(|v| v.parse::<Uuid>().ok());
    let (mut token, mut origin, mut bearer) = (None, None, None);
    for header in req.headers.iter() {
        let Ok(value) = std::str::from_utf8(header.value) else { continue };
        if header.name.eq_ignore_ascii_case("cookie") && token.is_none() {
            token = cookie_value(value, cookie_name).map(str::to_string);
        } else if header.name.eq_ignore_ascii_case("origin") {
            origin = Some(value.trim().to_string());
        } else if header.name.eq_ignore_ascii_case("authorization") && route == Route::Metrics {
            bearer = value.trim().strip_prefix("Bearer ").map(|t| t.to_string());
        }
    }
    Ok(Some(Request { method, route, client, token, origin, bearer }))
}

/// The value of the cookie `name` in a `Cookie` header.
fn cookie_value<'a>(header: &'a str, name: &str) -> Option<&'a str> {
    header.split(';').filter_map(|pair| pair.trim().split_once('=')).find(|(key, _)| *key == name).map(|(_, value)| value)
}

/// Who may read `/metrics`: the holder of `METRICS_TOKEN` (Bearer). Closed when there is no token, unless
/// `METRICS_ALLOW_OPEN=true` says otherwise.
#[derive(Clone, Debug)]
pub struct MetricsAccess {
    token_hash: Option<[u8; 32]>,
    allow_open: bool,
}

impl MetricsAccess {
    pub fn new(token: Option<&str>, allow_open: bool) -> Self {
        Self { token_hash: token.map(|t| Sha256::digest(t.as_bytes()).into()), allow_open }
    }

    pub fn allows(&self, bearer: Option<&str>) -> bool {
        let Some(expected) = &self.token_hash else { return self.allow_open };
        // Compared by their hashes, in constant time.
        bearer.is_some_and(|given| Sha256::digest(given.as_bytes()).as_slice().ct_eq(expected).into())
    }
}

/// The CORS headers for a request from `origin`: only for an origin that is listed (never `*` with credentials). The value
/// echoed is the configured one, not what the client sent.
fn cors(origin: Option<&str>, allowed: &[String]) -> String {
    match origin.and_then(|o| allowed.iter().find(|a| a.as_str() == o)) {
        Some(origin) => format!("Access-Control-Allow-Origin: {origin}\r\nAccess-Control-Allow-Credentials: true\r\nVary: Origin\r\n"),
        None => "Vary: Origin\r\n".to_string(),
    }
}

/// A complete response with a short body, then the connection closes.
async fn respond(socket: &mut TcpStream, status: &str, headers: &str, body: &str) {
    let content_type = if body.starts_with('{') { "application/json" } else { "text/plain; charset=utf-8" };
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n{headers}Connection: close\r\n\r\n{body}",
        body.len()
    );
    write(socket, response.as_bytes()).await;
    let _ = socket.shutdown().await;
}

async fn write(socket: &mut TcpStream, bytes: &[u8]) -> bool {
    matches!(tokio::time::timeout(WRITE_TIMEOUT, socket.write_all(bytes)).await, Ok(Ok(())))
}

/// Reads the head of the request.
async fn read_request(socket: &mut TcpStream) -> Result<Request, Result<(), Bad>> {
    // On the heap and dropped when this returns: a stream that stays open for hours does not keep its request.
    let mut buf: Vec<u8> = Vec::with_capacity(1024);
    loop {
        match parse(&buf, PATH, ACCESS_COOKIE) {
            Ok(Some(request)) => return Ok(request),
            Ok(None) => {}
            Err(bad) => return Err(Err(bad)),
        }
        if buf.len() >= MAX_REQUEST {
            return Err(Err(Bad::TooLarge));
        }
        match socket.read_buf(&mut buf).await {
            Ok(0) | Err(_) => return Err(Ok(())),
            Ok(_) => {}
        }
    }
}

/// Serves until the listener fails for good.
pub async fn serve(listener: TcpListener, state: AppState) -> io::Result<()> {
    loop {
        let socket = match listener.accept().await {
            Ok((socket, _)) => socket,
            Err(err) => {
                // Out of file descriptors, or of memory for a socket: wait, do not spin.
                tracing::warn!(error = %err, "accept failed");
                tokio::time::sleep(Duration::from_millis(50)).await;
                continue;
            }
        };
        let _ = socket.set_nodelay(true);
        tokio::spawn(connection(socket, state.clone()));
    }
}

async fn connection(mut socket: TcpStream, state: AppState) {
    let request = match tokio::time::timeout(HEADER_TIMEOUT, read_request(&mut socket)).await {
        Ok(Ok(request)) => request,
        Ok(Err(Ok(()))) | Err(_) => return,
        Ok(Err(Err(bad))) => {
            let (status, text) = match bad {
                Bad::Malformed => ("400 Bad Request", "bad request"),
                Bad::TooLarge => ("431 Request Header Fields Too Large", "request too large"),
                Bad::Method => ("405 Method Not Allowed", "method not allowed"),
            };
            return respond(&mut socket, status, "Allow: GET, OPTIONS\r\n", text).await;
        }
    };
    let cors = cors(request.origin.as_deref(), &state.config.cors_allowed_origins);
    match (request.route, request.method) {
        (Route::Stream, Method::Options) => {
            let headers = format!("{cors}Access-Control-Allow-Methods: GET, OPTIONS\r\nAccess-Control-Allow-Headers: accept\r\nAccess-Control-Max-Age: 600\r\n");
            respond(&mut socket, "204 No Content", &headers, "").await
        }
        (Route::Stream, Method::Get) => stream(socket, state, request, cors).await,
        (Route::Livez, Method::Get) => respond(&mut socket, "200 OK", "", "{\"status\":\"alive\"}").await,
        (Route::Readyz, Method::Get) => {
            // Without the subscription the streams would stay open and silent: out of the load balancer until it is back.
            if state.hub.is_subscribed() {
                respond(&mut socket, "200 OK", "", "{\"status\":\"ready\"}").await
            } else {
                respond(&mut socket, "503 Service Unavailable", "", "{\"message\":\"Service unavailable: channel\"}").await
            }
        }
        (Route::Metrics, Method::Get) => {
            if state.metrics_access.allows(request.bearer.as_deref()) {
                let body = state.metrics.render(state.registry.len(), state.hub.is_subscribed());
                respond(&mut socket, "200 OK", "", &body).await
            } else {
                respond(&mut socket, "401 Unauthorized", "", "unauthorized").await
            }
        }
        _ => respond(&mut socket, "404 Not Found", "", "not found").await,
    }
}

/// `GET <path>`: the stream of the events for the account that is signed in, in its active company.
///
/// The account and the company come from the signed access token (`sub`, `tid`), never from a header, a parameter or a
/// body the client chose. The stream lasts as long as the session: when the token that opened it expires, the session is
/// read again (see `sweeper`) and the stream goes on, or ends. A browser reads its state each time a stream opens.
///
/// 503 with a `Retry-After` when the instance holds as many streams as it may: refusing is how the memory stays bounded.
async fn stream(mut socket: TcpStream, state: AppState, mut request: Request, cors: String) {
    let Some(token) = request.token.take() else {
        bump(&state.metrics.rejected_auth);
        return respond(&mut socket, "401 Unauthorized", &cors, "{\"message\":\"Unauthorized\"}").await;
    };
    let claims = match state.verifier.verify(&token).await {
        Ok(claims) => claims,
        Err(err) => {
            bump(&state.metrics.rejected_auth);
            return match err {
                AppError::Unauthorized => respond(&mut socket, "401 Unauthorized", &cors, "{\"message\":\"Unauthorized\"}").await,
                AppError::Unavailable(_) => respond(&mut socket, "503 Service Unavailable", &cors, "{\"message\":\"Service unavailable\"}").await,
            };
        }
    };
    drop(token);
    let mut permissions = claims.permissions;
    permissions.truncate(MAX_PERMISSIONS);
    let who = Who { account: claims.sub, tenant: claims.tid.unwrap_or(NO_TENANT), session: claims.sid, client: request.client, permissions, deadline: claims.exp };
    let mut connection = match state.registry.register(who) {
        Ok(connection) => connection,
        Err(Refusal::Full) => {
            // A wait that differs a little from one refusal to the next, so that the browsers turned away together do not
            // come back together.
            let wait = 2 + std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos()) % 8;
            return respond(&mut socket, "503 Service Unavailable", &format!("{cors}Retry-After: {wait}\r\n"), "instance full").await;
        }
    };
    drop(request);

    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache, no-transform\r\n\
         X-Accel-Buffering: no\r\n{cors}Connection: close\r\n\r\n"
    );
    drop(cors);
    if !write(&mut socket, head.as_bytes()).await || !write(&mut socket, PRELUDE).await {
        return;
    }
    drop(head);

    // From here the task holds a socket and the receiving end of a queue, and waits.
    let mut scratch = [0u8; 64];
    let mut sent_to_us = 0usize;
    loop {
        tokio::select! {
            frame = connection.next() => {
                let Some(frame): Option<Bytes> = frame else { break };
                if !write(&mut socket, &frame).await {
                    break;
                }
            }
            // The client closed (or sent something, which a stream never expects: read and ignore it).
            readable = socket.readable() => {
                if readable.is_err() {
                    break;
                }
                match socket.try_read(&mut scratch) {
                    Ok(0) => break,
                    Ok(n) => {
                        sent_to_us += n;
                        if sent_to_us > MAX_SENT_TO_A_STREAM {
                            break;
                        }
                    }
                    Err(err) if err.kind() == io::ErrorKind::WouldBlock => {}
                    Err(_) => break,
                }
            }
        }
    }
    // Dropping `connection` takes the stream out of the index; dropping the socket closes the connection.
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_get(head: &str) -> Request {
        parse(head.as_bytes(), "/v1/events", "access").unwrap().unwrap()
    }

    #[test]
    fn it_reads_the_route_the_tab_and_the_token() {
        let tab = Uuid::new_v4();
        let r = parse_get(&format!(
            "GET /v1/events?x=1&client={tab} HTTP/1.1\r\nHost: a\r\nCookie: theme=dark; access=abc.def; lang=fr\r\nOrigin: https://app.example.com\r\n\r\n"
        ));
        assert_eq!((r.method, r.route, r.client), (Method::Get, Route::Stream, Some(tab)));
        assert_eq!(r.token.as_deref(), Some("abc.def"));
        assert_eq!(r.origin.as_deref(), Some("https://app.example.com"));
    }

    #[test]
    fn a_client_that_is_not_a_uuid_is_ignored_and_a_longer_cookie_name_is_not_the_cookie() {
        let r = parse_get("GET /v1/events?client=not-a-uuid HTTP/1.1\r\nCookie: not_access=x\r\n\r\n");
        assert_eq!((r.client, r.token), (None, None));
        let r = parse_get("GET /v1/events HTTP/1.1\r\nCookie: a=1\r\nCookie: access=second\r\n\r\n");
        assert_eq!(r.token.as_deref(), Some("second"), "the cookie may be in any Cookie header");
    }

    #[test]
    fn it_knows_its_routes_and_nothing_else() {
        for (target, route) in [("/livez", Route::Livez), ("/readyz", Route::Readyz), ("/metrics", Route::Metrics), ("/v1/events", Route::Stream), ("/v1/events/", Route::Other), ("/", Route::Other), ("/admin", Route::Other)] {
            assert_eq!(parse_get(&format!("GET {target} HTTP/1.1\r\n\r\n")).route, route, "{target}");
        }
        assert_eq!(parse_get("GET /v1/events?client= HTTP/1.1\r\n\r\n").route, Route::Stream);
    }

    #[test]
    fn the_bearer_is_read_for_the_metrics_only() {
        assert_eq!(parse_get("GET /metrics HTTP/1.1\r\nAuthorization: Bearer s3cret\r\n\r\n").bearer.as_deref(), Some("s3cret"));
        assert_eq!(parse_get("GET /v1/events HTTP/1.1\r\nAuthorization: Bearer s3cret\r\n\r\n").bearer, None);
    }

    #[test]
    fn a_request_is_complete_only_when_its_head_is() {
        assert_eq!(parse(b"GET /v1/events HTT", "/v1/events", "a"), Ok(None));
        assert_eq!(parse(b"GET /v1/events HTTP/1.1\r\nHost: a\r\n", "/v1/events", "a"), Ok(None));
        assert!(parse(b"GET /v1/events HTTP/1.1\r\n\r\n", "/v1/events", "a").unwrap().is_some());
    }

    #[test]
    fn only_get_and_options_are_understood_and_garbage_is_refused() {
        for method in ["POST", "PUT", "DELETE", "HEAD", "CONNECT", "PATCH"] {
            assert_eq!(parse(format!("{method} /v1/events HTTP/1.1\r\n\r\n").as_bytes(), "/v1/events", "a"), Err(Bad::Method), "{method}");
        }
        assert_eq!(parse(b"\x16\x03\x01\x02\x00\x01\x00\x01\xfc\x03\x03", "/v1/events", "a"), Err(Bad::Malformed), "a TLS hello");
        assert_eq!(parse(b"GET /v1/events HTTP/1.1\r\nbad header\r\n\r\n", "/v1/events", "a"), Err(Bad::Malformed));
        let many: String = (0..60).map(|i| format!("X-{i}: v\r\n")).collect();
        assert_eq!(parse(format!("GET /v1/events HTTP/1.1\r\n{many}\r\n").as_bytes(), "/v1/events", "a"), Err(Bad::TooLarge));
    }

    #[test]
    fn a_line_break_cannot_be_smuggled_into_the_cors_headers() {
        let allowed = vec!["https://app.example.com".to_string()];
        assert!(cors(Some("https://app.example.com"), &allowed).contains("Access-Control-Allow-Origin: https://app.example.com\r\n"));
        for origin in [Some("https://evil.example.com"), Some("https://app.example.com\r\nX-Evil: 1"), Some("*"), None] {
            let headers = cors(origin, &allowed);
            assert_eq!(headers, "Vary: Origin\r\n", "{origin:?}");
        }
    }

    #[test]
    fn the_token_opens_the_metrics_and_without_a_token_they_are_closed_unless_allowed() {
        let with_token = MetricsAccess::new(Some("s3cret"), false);
        assert!(with_token.allows(Some("s3cret")));
        assert!(!with_token.allows(Some("wrong")));
        assert!(!with_token.allows(None));
        assert!(!MetricsAccess::new(None, false).allows(None));
        assert!(MetricsAccess::new(None, true).allows(None));
    }
}
