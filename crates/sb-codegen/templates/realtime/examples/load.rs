//! Load test: starts the service, opens many streams, publishes events, and reports what it cost.
//!
//! ```sh
//! docker run -d --ulimit memlock=-1 -p 6379:6379 docker.dragonflydb.io/dragonflydb/dragonfly
//! cargo build --release -p <service> --bins --examples
//! target/release/examples/load --connections 50000 --events-per-sec 1000 --seconds 30
//! ```
//!
//! It signs its own tokens (it serves the JWKS the service reads), writes the session traces the service looks up, starts
//! the service binary of the same build as a child process, and measures *that process*: its resident memory, its CPU time
//! and how late the events arrive. Options: `--connections N` (50000), `--events-per-sec N` (1000), `--seconds N` (30),
//! `--tenant-size N` (10: streams per company, so an event goes to about that many), `--dragonfly URL`
//! (redis://127.0.0.1:6379), `--port N` (18085), `--short-tokens` (tokens of 20 s, so that sessions are renewed during the
//! run), `--keep-ratio-slow N` (0: the share in thousandths of streams that never read, to see them closed),
//! `--server-bin PATH` (measure another build of the service, or another implementation of the same contract, in place of
//! this one: the environment it gets has the names of both `SERVER_PORT`/`DRAGONFLY_URL` and `REALTIME_PORT`/`VALKEY_URL`).
//!
//! The streams are opened from several loopback addresses (127.0.1.x): one address has about 28 000 source ports towards one
//! destination. The machine needs `ulimit -n` above twice the number of connections (this raises it as far as it can).

use std::{
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering::Relaxed},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::SigningKey;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use realtime_service::contract::{ACCESS_COOKIE, CHANNEL, ISSUER, PATH, SESSION_PREFIX};
use serde_json::json;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpSocket, TcpStream},
};
use uuid::Uuid;

const PKCS8_ED25519_PREFIX: [u8; 16] = [0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20];
/// Latency resolution: 0.1 ms, up to 10 s.
const BUCKETS: usize = 100_000;

struct Args {
    connections: usize,
    events_per_sec: u64,
    seconds: u64,
    tenant_size: usize,
    dragonfly: String,
    port: u16,
    short_tokens: bool,
    slow_per_thousand: usize,
    server_bin: Option<String>,
}

fn args() -> Args {
    let mut a = Args {
        connections: 50_000,
        events_per_sec: 1_000,
        seconds: 30,
        tenant_size: 10,
        dragonfly: "redis://127.0.0.1:6379".into(),
        port: 18085,
        short_tokens: false,
        slow_per_thousand: 0,
        server_bin: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut value = || it.next().unwrap_or_else(|| panic!("{flag} needs a value"));
        match flag.as_str() {
            "--connections" => a.connections = value().parse().unwrap(),
            "--events-per-sec" => a.events_per_sec = value().parse().unwrap(),
            "--seconds" => a.seconds = value().parse().unwrap(),
            "--tenant-size" => a.tenant_size = value().parse().unwrap(),
            "--dragonfly" => a.dragonfly = value(),
            "--port" => a.port = value().parse().unwrap(),
            "--keep-ratio-slow" => a.slow_per_thousand = value().parse().unwrap(),
            "--short-tokens" => a.short_tokens = true,
            "--server-bin" => a.server_bin = Some(value()),
            other => panic!("unknown option {other}"),
        }
    }
    a
}

fn micros() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_micros() as u64
}

/// What the server process costs, from `/proc`.
struct Proc {
    rss_kb: u64,
    peak_kb: u64,
    threads: u64,
    cpu_secs: f64,
}

fn proc_stats(pid: u32) -> Proc {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap_or_default();
    let field = |name: &str| status.lines().find_map(|l| l.strip_prefix(name)).and_then(|v| v.trim().trim_end_matches("kB").trim().parse::<u64>().ok()).unwrap_or(0);
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
    // After the command name (in parentheses): state is field 3, utime and stime are 14 and 15.
    let after = stat.rsplit_once(')').map(|(_, rest)| rest).unwrap_or("");
    let ticks = |n: usize| after.split_whitespace().nth(n - 3).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
    Proc { rss_kb: field("VmRSS:"), peak_kb: field("VmHWM:"), threads: field("Threads:"), cpu_secs: (ticks(14) + ticks(15)) / 100.0 }
}

struct Latency {
    buckets: Vec<AtomicU64>,
    received: AtomicU64,
    /// `resync` events received: the service telling a stream to read its state again.
    resyncs: AtomicU64,
}

impl Latency {
    fn record(&self, micros: u64) {
        self.buckets[((micros / 100) as usize).min(BUCKETS - 1)].fetch_add(1, Relaxed);
        self.received.fetch_add(1, Relaxed);
    }

    fn percentile(&self, p: f64) -> f64 {
        let total = self.received.load(Relaxed);
        let target = (total as f64 * p).ceil() as u64;
        let mut seen = 0;
        for (i, b) in self.buckets.iter().enumerate() {
            seen += b.load(Relaxed);
            if seen >= target {
                return (i as f64 + 1.0) / 10.0;
            }
        }
        BUCKETS as f64 / 10.0
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// A client: opens the stream and counts the `load.tick` events it gets, with how late they are. A slow one never reads.
async fn client(addr: SocketAddr, from: std::net::Ipv4Addr, token: String, slow: bool, latency: Arc<Latency>, connected: Arc<AtomicU64>) -> bool {
    let socket = TcpSocket::new_v4().unwrap();
    if socket.bind(SocketAddr::new(from.into(), 0)).is_err() {
        return false;
    }
    let Ok(mut stream) = socket.connect(addr).await else { return false };
    let request = format!("GET {PATH}?client={} HTTP/1.1\r\nHost: load\r\nCookie: {ACCESS_COOKIE}={token}\r\nAccept: text/event-stream\r\n\r\n", Uuid::new_v4());
    if stream.write_all(request.as_bytes()).await.is_err() {
        return false;
    }
    let mut buf = vec![0u8; 2048];
    let mut pending: Vec<u8> = Vec::new();
    // Until the first line of the stream: the response is a 200 with the stream started.
    loop {
        let Ok(n) = stream.read(&mut buf).await else { return false };
        if n == 0 {
            return false;
        }
        pending.extend_from_slice(&buf[..n]);
        // The first line of the stream: `retry: ...` (every implementation of this contract sends it first).
        if find(&pending, b"200 OK").is_some() && find(&pending, b"retry: ").is_some() {
            break;
        }
        if find(&pending, b"HTTP/1.1 5").is_some() || find(&pending, b"HTTP/1.1 4").is_some() {
            return false;
        }
    }
    connected.fetch_add(1, Relaxed);
    pending.clear();
    if slow {
        // Never reads again: the server must close this one when its queue is full, and keep the others fast.
        std::future::pending::<()>().await;
    }
    const MARK: &[u8] = b"event: load.tick\ndata: {\"t\":\"";
    loop {
        let Ok(n) = stream.read(&mut buf).await else { return true };
        if n == 0 {
            return true;
        }
        pending.extend_from_slice(&buf[..n]);
        while let Some(at) = find(&pending, MARK) {
            let rest = &pending[at + MARK.len()..];
            let Some(end) = rest.iter().position(|b| *b == b'"') else {
                pending.drain(..at);
                break;
            };
            if let Some(sent) = std::str::from_utf8(&rest[..end]).ok().and_then(|t| t.parse::<u64>().ok()) {
                latency.record(micros().saturating_sub(sent));
            }
            pending.drain(..at + MARK.len() + end);
        }
        const RESYNC: &[u8] = b"event: resync\ndata: {}\n\n";
        while let Some(at) = find(&pending, RESYNC) {
            latency.resyncs.fetch_add(1, Relaxed);
            pending.drain(..at + RESYNC.len());
        }
        if pending.len() > 256 {
            pending.drain(..pending.len() - 256);
        }
    }
}

fn sign(account: Uuid, tenant: Uuid, session: Uuid, exp: i64) -> String {
    let mut pkcs8 = PKCS8_ED25519_PREFIX.to_vec();
    pkcs8.extend_from_slice(&[9; 32]);
    let mut header = Header::new(Algorithm::EdDSA);
    header.kid = Some("load".into());
    let claims = json!({"sub": account, "tid": tenant, "sid": session, "exp": exp, "iss": ISSUER, "permissions": ["contacts.view", "tasks.view", "billing.view"]});
    encode(&header, &claims, &EncodingKey::from_ed_der(&pkcs8)).unwrap()
}

/// The JWKS the service reads its key from.
async fn serve_jwks() -> u16 {
    let public = SigningKey::from_bytes(&[9; 32]).verifying_key().to_bytes();
    let body = json!({"keys": [{"kty": "OKP", "crv": "Ed25519", "use": "sig", "alg": "EdDSA", "kid": "load", "x": URL_SAFE_NO_PAD.encode(public)}]}).to_string();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else { continue };
            let body = body.clone();
            tokio::spawn(async move {
                let mut buf = [0u8; 1024];
                let _ = socket.read(&mut buf).await;
                let reply = format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len());
                let _ = socket.write_all(reply.as_bytes()).await;
            });
        }
    });
    port
}

async fn http_get(addr: SocketAddr, path: &str) -> String {
    let Ok(mut s) = TcpStream::connect(addr).await else { return String::new() };
    let _ = s.write_all(format!("GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").as_bytes()).await;
    let mut out = String::new();
    let _ = s.read_to_string(&mut out).await;
    out
}

#[tokio::main]
async fn main() {
    let a = args();
    let limit = rlimit::increase_nofile_limit(a.connections as u64 * 2 + 4096).unwrap_or(0);
    if limit < a.connections as u64 + 2048 {
        eprintln!("`ulimit -n` is {limit}: too low for {} connections", a.connections);
    }

    // The service, the same build as this tool.
    let jwks_port = serve_jwks().await;
    let exe = std::env::current_exe().unwrap();
    let server_bin = match &a.server_bin {
        Some(path) => std::path::PathBuf::from(path),
        None => exe.parent().unwrap().parent().unwrap().join(env!("CARGO_PKG_NAME")),
    };
    let mut child = std::process::Command::new(&server_bin)
        .env("APP_ENV", "local")
        .env("SERVER_PORT", a.port.to_string())
        .env("REALTIME_HOST", "127.0.0.1")
        .env("REALTIME_PORT", a.port.to_string())
        .env("VALKEY_URL", &a.dragonfly)
        .env("CORS_ALLOWED_ORIGINS", "http://localhost:5173")
        .env("DRAGONFLY_URL", &a.dragonfly)
        .env("JWKS_URL", format!("http://127.0.0.1:{jwks_port}/jwks"))
        .env("REALTIME_MAX_CONNECTIONS", (a.connections + 1_000).to_string())
        .env("REALTIME_MAX_PER_ACCOUNT", "5")
        .env("METRICS_ALLOW_OPEN", "true")
        .env("RUST_LOG", "realtime_service=warn")
        .spawn()
        .unwrap_or_else(|e| panic!("cannot start {}: {e} (build it: cargo build --release -p {} --bins)", server_bin.display(), env!("CARGO_PKG_NAME")));
    let pid = child.id();
    let addr: SocketAddr = ([127, 0, 0, 1], a.port).into();
    let started = Instant::now();
    while !http_get(addr, "/readyz").await.starts_with("HTTP/1.1 200") {
        assert!(started.elapsed() < Duration::from_secs(30), "the service did not become ready: is DragonflyDB at {}?", a.dragonfly);
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let idle = proc_stats(pid);
    println!("service up (pid {pid}); idle: {:.1} MiB resident, {} threads", idle.rss_kb as f64 / 1024.0, idle.threads);

    // The people: `tenant_size` accounts per company, a session each, and the trace of it that the API would have left.
    let redis = redis::Client::open(a.dragonfly.as_str()).unwrap();
    let mut db = redis.get_multiplexed_async_connection().await.unwrap();
    let now = chrono::Utc::now().timestamp();
    let token_life = if a.short_tokens { 20 } else { 300 };
    let people: Vec<(Uuid, Uuid, Uuid)> = (0..a.connections).map(|i| (Uuid::new_v4(), Uuid::from_u128((i / a.tenant_size) as u128 + 1), Uuid::new_v4())).collect();
    for chunk in people.chunks(2000) {
        let mut pipe = redis::pipe();
        for (account, tenant, session) in chunk {
            let trace = json!({"accountId": account, "tenantId": tenant, "exp": now + 3600}).to_string();
            pipe.cmd("SET").arg(format!("{SESSION_PREFIX}:{session}")).arg(trace).arg("EX").arg(7200).ignore();
        }
        let _: () = pipe.query_async(&mut db).await.unwrap();
    }

    // Opening the streams, in waves.
    let latency = Arc::new(Latency { buckets: (0..BUCKETS).map(|_| AtomicU64::new(0)).collect(), received: AtomicU64::new(0), resyncs: AtomicU64::new(0) });
    let connected = Arc::new(AtomicU64::new(0));
    let failed = Arc::new(AtomicU64::new(0));
    let opening = Instant::now();
    let mut tasks = Vec::with_capacity(a.connections);
    for (i, (account, tenant, session)) in people.iter().enumerate() {
        let token = sign(*account, *tenant, *session, now + token_life);
        let from = std::net::Ipv4Addr::new(127, 0, 1, (i % 8) as u8 + 1);
        let slow = a.slow_per_thousand > 0 && i % 1000 < a.slow_per_thousand;
        let (task_latency, task_connected, task_failed) = (latency.clone(), connected.clone(), failed.clone());
        tasks.push(tokio::spawn(async move {
            if !client(addr, from, token, slow, task_latency, task_connected).await {
                task_failed.fetch_add(1, Relaxed);
            }
        }));
        if i % 500 == 499 {
            // A wave of 500 at a time: a service that has just restarted gets its browsers back in about this way.
            while (connected.load(Relaxed) + failed.load(Relaxed)) + 500 < (i as u64 + 1) {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }
    }
    while connected.load(Relaxed) + failed.load(Relaxed) < a.connections as u64 && opening.elapsed() < Duration::from_secs(120) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let open = proc_stats(pid);
    let open_streams = connected.load(Relaxed);
    println!(
        "{open_streams} streams open in {:.1} s ({} refused or failed); service: {:.1} MiB resident (peak {:.1}), {} threads",
        opening.elapsed().as_secs_f64(),
        failed.load(Relaxed),
        open.rss_kb as f64 / 1024.0,
        open.peak_kb as f64 / 1024.0,
        open.threads
    );
    println!("  per stream: {:.1} KiB resident in the service", (open.rss_kb.saturating_sub(idle.rss_kb)) as f64 / open_streams.max(1) as f64);

    // The events: each to one company, so to `tenant_size` streams.
    let tenants = a.connections.div_ceil(a.tenant_size);
    let cpu_before = proc_stats(pid).cpu_secs;
    let run = Instant::now();
    let mut published = 0u64;
    let mut expected = 0u64;
    let mut tick = tokio::time::interval(Duration::from_millis(10));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Burst);
    let per_tick = (a.events_per_sec / 100).max(1);
    let mut rng = 0x2545_F491_4F6C_DD1Du64;
    let mut publish_errors = 0u64;
    while run.elapsed() < Duration::from_secs(a.seconds) {
        tick.tick().await;
        let mut pipe = redis::pipe();
        let mut batch_expected = 0u64;
        for _ in 0..per_tick {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            let tenant = (rng % tenants as u64) as usize;
            let size = (a.connections - tenant * a.tenant_size).min(a.tenant_size);
            batch_expected += size as u64;
            let message = json!({"type": "load.tick", "tenantId": Uuid::from_u128(tenant as u128 + 1), "permission": "contacts.view", "data": {"t": micros().to_string()}});
            pipe.cmd("PUBLISH").arg(CHANNEL).arg(message.to_string()).ignore();
            published += 1;
        }
        // The server may be restarted under the test: what could not be published is not expected.
        match pipe.query_async::<()>(&mut db).await {
            Ok(()) => expected += batch_expected,
            Err(_) => {
                publish_errors += 1;
                published -= per_tick;
                if let Ok(Ok(connection)) = tokio::time::timeout(Duration::from_millis(500), redis.get_multiplexed_async_connection()).await {
                    db = connection;
                }
            }
        }
    }
    tokio::time::sleep(Duration::from_secs(3)).await;
    let after = proc_stats(pid);
    let received = latency.received.load(Relaxed);
    let secs = run.elapsed().as_secs_f64() - 3.0;
    println!("\npublished {published} events in {secs:.1} s ({:.0}/s); streams that should have got them: {expected}", published as f64 / secs);
    println!("delivered {received} ({:.2} %)", received as f64 * 100.0 / expected.max(1) as f64);
    if publish_errors > 0 || latency.resyncs.load(Relaxed) > 0 {
        println!("batches that could not be published: {publish_errors}; `resync` events received by the streams: {}", latency.resyncs.load(Relaxed));
    }
    println!("latency, publish to arrival: p50 {:.1} ms, p99 {:.1} ms, p99.9 {:.1} ms", latency.percentile(0.5), latency.percentile(0.99), latency.percentile(0.999));
    println!("service CPU: {:.2} cores on average over the run ({} threads)", (after.cpu_secs - cpu_before) / secs, after.threads);
    println!("service memory: {:.1} MiB resident, peak {:.1} MiB", after.rss_kb as f64 / 1024.0, after.peak_kb as f64 / 1024.0);

    let metrics = http_get(addr, "/metrics").await;
    println!("\nfrom /metrics:");
    for line in metrics.lines().filter(|l| l.starts_with("realtime_") && !l.starts_with('#')) {
        println!("  {line}");
    }
    let _ = child.kill();
    let _ = child.wait();
    for t in tasks {
        t.abort();
    }
}
