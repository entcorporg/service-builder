//! The Garage that Service Builder keeps its projects in. It is private to the application: the binary travels inside the
//! AppImage, its folder, its keys and its ports are the application's own, and nothing about it is a setting.
//!
//! [`ensure`] makes it ready and is cheap once it is: it starts the node if it is not running (a process of its own, which
//! outlives the window so that the MCP server and `sb` find it), and prepares it the first time (layout, bucket, access
//! key). Every process that needs the store calls it, under a lock, so two of them never start two nodes.

use std::fs;
use std::io::Write;
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::s3::S3Config;
use crate::store::Local;

/// Unusual ports, on the loopback only: nothing else of the machine is likely to hold them.
pub const S3_PORT: u16 = 39000;
const RPC_PORT: u16 = 39001;
const BUCKET: &str = "service-builder";
const KEY_NAME: &str = "service-builder";

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Secrets {
    rpc_secret: String,
    access_key: String,
    secret_key: String,
}

fn dir(local: &Local) -> PathBuf {
    local.data.join("garage")
}

/// Where the binary is: given by `SB_GARAGE_BIN` (tests, development), else the one packed with the application
/// (`$APPDIR/usr/bin/garage` in an AppImage), else the one next to the program.
pub fn binary() -> Option<PathBuf> {
    let mut candidates = vec![];
    if let Some(p) = std::env::var_os("SB_GARAGE_BIN").filter(|v| !v.is_empty()) {
        candidates.push(PathBuf::from(p));
    }
    if let Some(appdir) = std::env::var_os("APPDIR") {
        candidates.push(Path::new(&appdir).join("usr/bin/garage"));
    }
    if let Some(parent) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
    {
        candidates.push(parent.join("garage"));
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// Whether the store can be had at all: Garage is there, or was made ready before. Cheap (no process, no network).
pub fn available(local: &Local) -> bool {
    binary().is_some() || ready_marker(local).is_file()
}

fn ready_marker(local: &Local) -> PathBuf {
    dir(local).join("ready")
}

fn listening() -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], S3_PORT));
    TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_ok()
}

fn random_hex(bytes: usize) -> Result<String, String> {
    use std::io::Read;
    let mut buffer = vec![0u8; bytes];
    fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut buffer))
        .map_err(|e| format!("cannot read /dev/urandom: {e}"))?;
    Ok(buffer.iter().map(|b| format!("{b:02x}")).collect())
}

/// The secrets, drawn once and kept in a file only the user reads.
fn secrets(dir: &Path) -> Result<Secrets, String> {
    let path = dir.join("secrets.json");
    if let Some(s) = fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
    {
        return Ok(s);
    }
    let s = Secrets {
        rpc_secret: random_hex(32)?,
        access_key: format!("GK{}", random_hex(12)?),
        secret_key: random_hex(32)?,
    };
    let text = serde_json::to_string_pretty(&s).map_err(|e| e.to_string())? + "\n";
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options
        .open(&path)
        .and_then(|mut f| f.write_all(text.as_bytes()))
        .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok(s)
}

fn config_toml(dir: &Path, rpc_secret: &str) -> String {
    format!(
        "metadata_dir = \"{0}/meta\"\ndata_dir = \"{0}/data\"\ndb_engine = \"sqlite\"\nreplication_factor = 1\n\
         rpc_bind_addr = \"127.0.0.1:{RPC_PORT}\"\nrpc_public_addr = \"127.0.0.1:{RPC_PORT}\"\nrpc_secret = \"{rpc_secret}\"\n\n\
         [s3_api]\ns3_region = \"garage\"\napi_bind_addr = \"127.0.0.1:{S3_PORT}\"\n",
        dir.display()
    )
}

/// Runs the command line of Garage against the node, and gives what it printed.
fn cli(bin: &Path, toml: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new(bin)
        .arg("-c")
        .arg(toml)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("cannot run {}: {e}", bin.display()))?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    if out.status.success() {
        Ok(text)
    } else {
        Err(format!(
            "garage {}: {}",
            args.first().copied().unwrap_or(""),
            String::from_utf8_lossy(&out.stderr)
                .lines()
                .last()
                .unwrap_or("failed")
        ))
    }
}

/// The settings the store reaches the node with.
fn settings(s: &Secrets) -> S3Config {
    S3Config {
        endpoint: format!("http://127.0.0.1:{S3_PORT}"),
        region: "garage".into(),
        bucket: BUCKET.into(),
        access_key: s.access_key.clone(),
        secret_key: s.secret_key.clone(),
    }
}

/// Makes the store ready (started, prepared) and gives the settings to reach it.
pub fn ensure(local: &Local) -> Result<S3Config, String> {
    let dir = dir(local);
    fs::create_dir_all(dir.join("meta"))
        .and_then(|()| fs::create_dir_all(dir.join("data")))
        .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let secrets = secrets(&dir)?;
    // Fast path: prepared once, and answering.
    if ready_marker(local).is_file() && listening() {
        return Ok(settings(&secrets));
    }
    let bin = binary().ok_or_else(|| {
        "le stockage interne (Garage) est introuvable : il est livré dans l'AppImage (scripts/install-local.sh)".to_string()
    })?;
    // One process at a time starts and prepares it.
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join("ensure.lock"))
        .map_err(|e| e.to_string())?;
    lock.lock()
        .map_err(|e| format!("cannot lock {}: {e}", dir.display()))?;
    if ready_marker(local).is_file() && listening() {
        return Ok(settings(&secrets));
    }
    let _ = fs::remove_file(ready_marker(local));

    let toml = dir.join("garage.toml");
    fs::write(&toml, config_toml(&dir, &secrets.rpc_secret))
        .map_err(|e| format!("cannot write {}: {e}", toml.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&toml, fs::Permissions::from_mode(0o600));
    }

    if !listening() {
        let log = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("garage.log"))
            .map_err(|e| e.to_string())?;
        let mut node = Command::new(&bin);
        node.arg("-c").arg(&toml).arg("server").stdin(Stdio::null());
        node.stdout(log.try_clone().map_err(|e| e.to_string())?)
            .stderr(log);
        // Its own process group: it keeps running when the window closes or a terminal is interrupted.
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut node, 0);
        node.spawn()
            .map_err(|e| format!("cannot start {}: {e}", bin.display()))?;
    }
    let until = Instant::now() + Duration::from_secs(40);
    while cli(&bin, &toml, &["status"]).is_err() {
        if Instant::now() > until {
            return Err(format!(
                "Garage ne répond pas (voir {})",
                dir.join("garage.log").display()
            ));
        }
        std::thread::sleep(Duration::from_millis(300));
    }

    // Prepared once: the layout of its single node, the access key, the bucket. Every step looks before it acts.
    let status = cli(&bin, &toml, &["status"])?;
    if status.contains("NO ROLE ASSIGNED") {
        let id = cli(&bin, &toml, &["node", "id", "-q"])?;
        let id = id.trim().split('@').next().unwrap_or("").to_string();
        cli(
            &bin,
            &toml,
            &["layout", "assign", "-z", "dc1", "-c", "1G", &id],
        )?;
        cli(&bin, &toml, &["layout", "apply", "--version", "1"])?;
    }
    if cli(&bin, &toml, &["key", "info", &secrets.access_key]).is_err() {
        cli(
            &bin,
            &toml,
            &[
                "key",
                "import",
                "--yes",
                "-n",
                KEY_NAME,
                &secrets.access_key,
                &secrets.secret_key,
            ],
        )?;
    }
    if cli(&bin, &toml, &["bucket", "info", BUCKET]).is_err() {
        cli(&bin, &toml, &["bucket", "create", BUCKET])?;
    }
    cli(
        &bin,
        &toml,
        &[
            "bucket",
            "allow",
            "--read",
            "--write",
            "--owner",
            BUCKET,
            "--key",
            &secrets.access_key,
        ],
    )?;
    fs::write(ready_marker(local), b"ok\n").map_err(|e| e.to_string())?;
    Ok(settings(&secrets))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_node_is_configured_on_the_loopback_with_its_own_folders() {
        let toml = config_toml(Path::new("/data/garage"), "abc");
        assert!(toml.contains("metadata_dir = \"/data/garage/meta\""));
        assert!(toml.contains("api_bind_addr = \"127.0.0.1:39000\""));
        assert!(toml.contains("rpc_bind_addr = \"127.0.0.1:39001\""));
        assert!(!toml.contains("0.0.0.0") && !toml.contains("[::]"));
    }

    #[test]
    fn the_secrets_are_drawn_once_and_private() {
        let dir = std::env::temp_dir().join(format!("sb-garage-secrets-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let a = secrets(&dir).unwrap();
        let b = secrets(&dir).unwrap();
        assert_eq!(a.access_key, b.access_key);
        assert!(a.access_key.starts_with("GK") && a.access_key.len() == 26);
        assert_eq!((a.secret_key.len(), a.rpc_secret.len()), (64, 64));
        assert_ne!(a.secret_key, a.rpc_secret);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(dir.join("secrets.json"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o077, 0, "{mode:o}");
        }
        fs::remove_dir_all(dir).unwrap();
    }
}
