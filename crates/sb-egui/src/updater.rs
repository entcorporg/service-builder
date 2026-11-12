//! Self-update from the GitHub releases of the project.
//!
//! The latest release carries the AppImage and its minisign signature (`<name>.AppImage.minisig`). An update is
//! installed only if that signature verifies against the public key below, so a release that was not signed with
//! our key is refused. Only an AppImage can replace itself; any other install is told where to download.

use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use minisign_verify::{PublicKey, Signature};
use serde::Deserialize;

const REPO: &str = "entcorporg/service-builder";
/// Same key as the one that signed the releases of the earlier builds (second line of the minisign `.pub` file).
const PUBLIC_KEY: &str = "RWQiT9vVFFIol7mB70PrAQTHCd1KMEg/4lpYy7o+u29zpg4NiUeU0zq8";
/// An AppImage is far below this; the cap keeps a wrong or hostile response from filling the disk.
const MAX_DOWNLOAD: u64 = 512 * 1024 * 1024;

pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, PartialEq)]
pub struct Update {
    pub version: String,
    pub notes: Option<String>,
    pub release_page: String,
    /// Absent when the release has no AppImage (or no signature for it).
    pub download: Option<Download>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Download {
    pub url: String,
    pub signature_url: String,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    body: Option<String>,
    html_url: String,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .user_agent(format!("service-builder/{CURRENT_VERSION}"))
        .timeout_global(Some(std::time::Duration::from_secs(600)))
        .build()
        .into()
}

/// `1.2.3` or `v1.2.3`; anything else (pre-releases included) is not a version we publish.
fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let mut parts = text.trim().trim_start_matches('v').split('.');
    let v = (
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    );
    parts.next().is_none().then_some(v)
}

fn into_update(release: Release, current: &str) -> Option<Update> {
    let latest = parse_version(&release.tag_name)?;
    if latest <= parse_version(current)? {
        return None;
    }
    let appimage = release
        .assets
        .iter()
        .find(|a| a.name.ends_with(".AppImage"));
    let download = appimage.and_then(|image| {
        let signature = release
            .assets
            .iter()
            .find(|a| a.name == format!("{}.minisig", image.name))?;
        Some(Download {
            url: image.browser_download_url.clone(),
            signature_url: signature.browser_download_url.clone(),
        })
    });
    Some(Update {
        version: release.tag_name.trim_start_matches('v').to_string(),
        notes: release.body.filter(|b| !b.trim().is_empty()),
        release_page: release.html_url,
        download,
    })
}

/// Asks GitHub whether a release newer than this build exists. Blocking: call it off the UI thread.
pub fn check() -> Result<Option<Update>, String> {
    let release: Release = agent()
        .get(&format!(
            "https://api.github.com/repos/{REPO}/releases/latest"
        ))
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| format!("cannot reach GitHub: {e}"))?
        .body_mut()
        .read_json()
        .map_err(|e| format!("unexpected answer from GitHub: {e}"))?;
    Ok(into_update(release, CURRENT_VERSION))
}

/// The AppImage this process runs from; `None` for any other install (`.deb`, `cargo run`).
pub fn appimage_path() -> Option<PathBuf> {
    std::env::var_os("APPIMAGE")
        .map(PathBuf::from)
        .filter(|p| p.is_file())
}

fn verify(content: &[u8], signature: &str) -> Result<(), String> {
    let key = PublicKey::from_base64(PUBLIC_KEY).map_err(|e| format!("bad public key: {e}"))?;
    let signature = Signature::decode(signature).map_err(|e| format!("bad signature: {e}"))?;
    key.verify(content, &signature, false)
        .map_err(|_| "the signature does not match: update refused".to_string())
}

/// Downloads, verifies and swaps the running AppImage for the new one; the caller then restarts.
/// `progress` receives a percentage, or `None` while the size is unknown. Blocking.
pub fn install(
    download: &Download,
    target: &Path,
    progress: impl Fn(Option<u8>),
) -> Result<(), String> {
    let agent = agent();
    let signature = agent
        .get(&download.signature_url)
        .call()
        .map_err(|e| format!("cannot download the signature: {e}"))?
        .body_mut()
        .read_to_string()
        .map_err(|e| format!("cannot read the signature: {e}"))?;

    let mut response = agent
        .get(&download.url)
        .call()
        .map_err(|e| format!("cannot download the update: {e}"))?;
    let total = response.body().content_length();
    let mut reader = response.body_mut().as_reader();
    let mut content = Vec::new();
    let mut chunk = [0u8; 64 * 1024];
    loop {
        let n = reader
            .read(&mut chunk)
            .map_err(|e| format!("download interrupted: {e}"))?;
        if n == 0 {
            break;
        }
        content.extend_from_slice(&chunk[..n]);
        if content.len() as u64 > MAX_DOWNLOAD {
            return Err("the update is larger than expected: refused".into());
        }
        progress(total.map(|t| ((content.len() as u64 * 100) / t.max(1)).min(100) as u8));
    }
    verify(&content, &signature)?;
    replace(target, &content)
}

/// Writes next to the target (same filesystem) then renames over it: atomic, and a running AppImage can be replaced.
fn replace(target: &Path, content: &[u8]) -> Result<(), String> {
    let staged = target.with_extension("update");
    let write = || -> std::io::Result<()> {
        let mut file = fs::File::create(&staged)?;
        file.write_all(content)?;
        file.sync_all()?;
        fs::set_permissions(&staged, fs::Permissions::from_mode(0o755))?;
        fs::rename(&staged, target)
    };
    write().map_err(|e| {
        let _ = fs::remove_file(&staged);
        format!("cannot replace {}: {e}", target.display())
    })
}

/// Starts the new AppImage and returns; the caller quits.
pub fn relaunch(target: &Path) -> Result<(), String> {
    std::process::Command::new(target)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("cannot restart {}: {e}", target.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str, assets: &[&str]) -> Release {
        Release {
            tag_name: tag.into(),
            body: Some("notes".into()),
            html_url: "https://example.test/r".into(),
            assets: assets
                .iter()
                .map(|n| Asset {
                    name: (*n).into(),
                    browser_download_url: format!("https://example.test/{n}"),
                })
                .collect(),
        }
    }

    #[test]
    fn versions_compare_numerically() {
        assert_eq!(parse_version("v2.10.0"), Some((2, 10, 0)));
        assert!(parse_version("2.10.0") > parse_version("2.9.9"));
        assert_eq!(parse_version("2.0.0-rc.1"), None);
        assert_eq!(parse_version("2.0"), None);
    }

    #[test]
    fn only_a_newer_release_is_an_update() {
        assert!(into_update(release("v2.0.3", &[]), "2.0.3").is_none());
        assert!(into_update(release("v2.0.2", &[]), "2.0.3").is_none());
        assert!(into_update(release("nightly", &[]), "2.0.3").is_none());
        let update = into_update(release("v2.1.0", &[]), "2.0.3").unwrap();
        assert_eq!(update.version, "2.1.0");
        assert!(update.download.is_none());
    }

    #[test]
    fn an_appimage_needs_its_signature() {
        let image = "Service.Builder_2.1.0_amd64.AppImage";
        let unsigned = into_update(release("v2.1.0", &[image]), "2.0.3").unwrap();
        assert!(unsigned.download.is_none(), "no signature, no install");

        let signed = into_update(
            release("v2.1.0", &[image, &format!("{image}.minisig")]),
            "2.0.3",
        )
        .unwrap();
        let download = signed.download.unwrap();
        assert!(download.url.ends_with(image));
        assert!(download.signature_url.ends_with(".minisig"));
    }

    #[test]
    fn a_wrong_signature_is_refused() {
        assert!(verify(b"payload", "not a signature").is_err());
    }

    #[test]
    fn replacing_swaps_the_file_and_makes_it_executable() {
        let dir = std::env::temp_dir().join(format!("sb-update-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let target = dir.join("app.AppImage");
        fs::write(&target, "old").unwrap();
        replace(&target, b"new").unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"new");
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o111,
            0o111
        );
        assert!(!target.with_extension("update").exists());
        fs::remove_dir_all(dir).unwrap();
    }
}
