//! Where the projects are kept: an S3 bucket (Garage), with every save a new version, and on this machine the
//! working copy of each project plus the list of the last ones opened.
//!
//! ```text
//! bucket                                   this machine ($XDG_DATA_HOME/service-builder)
//! <id>/meta.json     name, version, sha    state.json         the last projects opened (the app opens the last one at start)
//! <id>/versions/000001.json …              projects/<id>.sb.json   the working copy of a project made in the app
//! ```
//!
//! The working copy is what the editor, the MCP server and the `sb` commands read and write; the bucket is where it
//! is kept and where its history lives. A project is identified by `<id>`: the name it had when it was first seen
//! (renaming it later does not change it). Nothing here needs the network to open a project: the bucket is only
//! reached to keep a version, to list the projects and to fetch one that is not on this machine.
//!
//! The bucket is the application's own Garage (`garage.rs`): started and prepared when it is needed, with keys nobody
//! sees. There is no setting, no address to give, nothing to create.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::s3::{self, Client, S3Config, sha256_hex};

/// The projects remembered: more is useless, the bucket lists them all.
pub const MAX_RECENT: usize = 12;

// ---- where things are on this machine ---------------------------------------------------

/// The folder of the application.
#[derive(Debug, Clone)]
pub struct Local {
    pub data: PathBuf,
}

fn xdg(var: &str, fallback: &[&str]) -> PathBuf {
    std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let mut home = PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| ".".into()));
            home.extend(fallback);
            home
        })
        .join("service-builder")
}

impl Local {
    /// `~/.local/share/service-builder`.
    pub fn system() -> Self {
        Self {
            data: xdg("XDG_DATA_HOME", &[".local", "share"]),
        }
    }

    pub fn state_file(&self) -> PathBuf {
        self.data.join("state.json")
    }

    /// The working copies of the projects made in the app.
    pub fn projects_dir(&self) -> PathBuf {
        self.data.join("projects")
    }

    pub fn working_copy(&self, id: &str) -> PathBuf {
        self.projects_dir().join(format!("{id}.sb.json"))
    }

    // ---- the state ----

    pub fn state(&self) -> State {
        fs::read_to_string(self.state_file())
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save_state(&self, state: &State) -> Result<(), String> {
        write_private(&self.state_file(), &(pretty(state) + "\n"))
    }

    /// Reads the state, changes it and writes it back.
    pub fn update_state<R>(&self, change: impl FnOnce(&mut State) -> R) -> Result<R, String> {
        let mut state = self.state();
        let out = change(&mut state);
        self.save_state(&state)?;
        Ok(out)
    }
}

fn pretty(value: &impl Serialize) -> String {
    serde_json::to_string_pretty(value).unwrap_or_default()
}

/// Atomic, and private: a crash leaves the old file, and nobody else reads it.
fn write_private(path: &Path, text: &str) -> Result<(), String> {
    let fail = |e: std::io::Error| format!("cannot write {}: {e}", path.display());
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(fail)?;
    }
    let tmp = path.with_extension("tmp");
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    std::io::Write::write_all(&mut options.open(&tmp).map_err(fail)?, text.as_bytes())
        .map_err(fail)?;
    fs::rename(&tmp, path).map_err(fail)
}

/// A project opened on this machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Recent {
    pub id: String,
    pub name: String,
    /// The working copy.
    pub path: String,
    /// Unix seconds.
    pub opened_at: u64,
    /// The hash of the file as it was when it was last stored, and the version it became: what tells that the
    /// file holds changes the bucket does not have.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub synced_sha: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub synced_version: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    /// The id of the project open in the app: the next start opens it again, and an MCP server started without a
    /// project works on it.
    #[serde(default)]
    pub last: Option<String>,
    /// The last ones opened, the most recent first.
    #[serde(default)]
    pub recent: Vec<Recent>,
}

impl State {
    pub fn find(&self, id: &str) -> Option<&Recent> {
        self.recent.iter().find(|r| r.id == id)
    }

    pub fn find_path(&self, path: &Path) -> Option<&Recent> {
        let wanted = path.display().to_string();
        self.recent.iter().find(|r| r.path == wanted)
    }

    /// The project that was open last, if its file is still there.
    pub fn last_open(&self) -> Option<&Recent> {
        self.last
            .as_deref()
            .and_then(|id| self.find(id))
            .filter(|r| Path::new(&r.path).is_file())
    }

    /// Puts the project first, and makes it the last one. What it knows about the bucket is kept unless given.
    pub fn touch(&mut self, id: &str, name: &str, path: &Path) {
        let before = self.find(id).cloned();
        self.recent.retain(|r| r.id != id);
        self.recent.insert(
            0,
            Recent {
                id: id.to_string(),
                name: name.to_string(),
                path: path.display().to_string(),
                opened_at: s3::now(),
                synced_sha: before.as_ref().and_then(|r| r.synced_sha.clone()),
                synced_version: before.and_then(|r| r.synced_version),
            },
        );
        self.recent.truncate(MAX_RECENT);
        self.last = Some(id.to_string());
    }

    pub fn synced(&mut self, id: &str, sha: Option<String>, version: Option<u32>) {
        if let Some(r) = self.recent.iter_mut().find(|r| r.id == id) {
            r.synced_sha = sha;
            r.synced_version = version;
        }
    }
}

/// The identity of a project: its name in lower case letters, digits and dashes.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-');
    if out.is_empty() {
        "project".into()
    } else {
        out.chars().take(80).collect()
    }
}

/// An id is a folder name of the bucket and a file name here: nothing else gets through.
fn check_id(id: &str) -> Result<(), String> {
    if !id.is_empty()
        && id.len() <= 80
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        Ok(())
    } else {
        Err(format!(
            "{id} is not a project id (lower case letters, digits and dashes)"
        ))
    }
}

// ---- the bucket -------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Meta {
    name: String,
    version: u32,
    sha256: String,
    saved_at: String,
}

/// One saved state of a project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Version {
    pub number: u32,
    /// `2026-11-18T12:46:03.000Z`.
    pub saved_at: String,
    pub size: u64,
}

/// A project of the bucket.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteProject {
    pub id: String,
    pub name: String,
    pub version: u32,
    pub saved_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pushed {
    pub version: u32,
    /// `false`: the bucket already had exactly this.
    pub created: bool,
}

pub struct Remote {
    client: Client,
}

fn meta_key(id: &str) -> String {
    format!("{id}/meta.json")
}

fn version_key(id: &str, number: u32) -> String {
    format!("{id}/versions/{number:06}.json")
}

impl Remote {
    pub fn connect(config: &S3Config) -> Result<Self, String> {
        Ok(Self {
            client: Client::new(config)?,
        })
    }

    pub fn check(&self) -> Result<(), String> {
        self.client.check()
    }

    fn meta(&self, id: &str) -> Result<Option<Meta>, String> {
        let Some(bytes) = self.client.get(&meta_key(id))? else {
            return Ok(None);
        };
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| format!("{}: not valid: {e}", meta_key(id)))
    }

    /// The versions of a project, the newest first.
    pub fn versions(&self, id: &str) -> Result<Vec<Version>, String> {
        check_id(id)?;
        let prefix = format!("{id}/versions/");
        let mut versions: Vec<Version> = self
            .client
            .list(&prefix, None)?
            .objects
            .into_iter()
            .filter_map(|o| {
                let number = o
                    .key
                    .strip_prefix(&prefix)?
                    .strip_suffix(".json")?
                    .parse()
                    .ok()?;
                Some(Version {
                    number,
                    saved_at: o.modified,
                    size: o.size,
                })
            })
            .collect();
        versions.sort_by_key(|v| std::cmp::Reverse(v.number));
        Ok(versions)
    }

    pub fn projects(&self) -> Result<Vec<RemoteProject>, String> {
        let mut out = vec![];
        for prefix in self.client.list("", Some("/"))?.prefixes {
            let id = prefix.trim_end_matches('/');
            if check_id(id).is_err() {
                continue;
            }
            if let Some(meta) = self.meta(id)? {
                out.push(RemoteProject {
                    id: id.to_string(),
                    name: meta.name,
                    version: meta.version,
                    saved_at: meta.saved_at,
                });
            }
        }
        out.sort_by_key(|p| p.name.to_lowercase());
        Ok(out)
    }

    /// A version of a project (the latest without a number): its number and its text.
    pub fn read(&self, id: &str, version: Option<u32>) -> Result<(u32, Vec<u8>), String> {
        check_id(id)?;
        let number = match version {
            Some(n) => n,
            None => {
                self.meta(id)?
                    .ok_or_else(|| format!("no project {id} in the bucket"))?
                    .version
            }
        };
        let bytes = self
            .client
            .get(&version_key(id, number))?
            .ok_or_else(|| format!("{id} has no version {number}"))?;
        Ok((number, bytes))
    }

    /// Keeps `bytes` as the next version of the project, unless the latest one is exactly that.
    pub fn push(&self, id: &str, name: &str, bytes: &[u8]) -> Result<Pushed, String> {
        check_id(id)?;
        let sha = sha256_hex(bytes);
        let meta = self.meta(id)?;
        if let Some(m) = &meta
            && m.sha256 == sha
        {
            return Ok(Pushed {
                version: m.version,
                created: false,
            });
        }
        // The numbers only go up, even if a `meta.json` is missing or behind.
        let latest = self.versions(id)?.first().map_or(0, |v| v.number);
        let version = latest.max(meta.map_or(0, |m| m.version)) + 1;
        self.client.put(&version_key(id, version), bytes)?;
        let meta = Meta {
            name: name.to_string(),
            version,
            sha256: sha,
            saved_at: s3::iso8601(s3::now()),
        };
        self.client.put(&meta_key(id), pretty(&meta).as_bytes())?;
        Ok(Pushed {
            version,
            created: true,
        })
    }
}

// ---- local and remote together ---------------------------------------------------------

/// Where a project of a search is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Source {
    /// In the bucket and on this machine.
    Both,
    Bucket,
    /// Only on this machine: never stored.
    Local,
}

impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Source::Both => "bucket+local",
            Source::Bucket => "bucket",
            Source::Local => "local",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectInfo {
    pub id: String,
    pub name: String,
    /// The latest version in the bucket.
    pub version: Option<u32>,
    pub updated_at: String,
    pub source: Source,
}

#[derive(Debug, Default)]
pub struct Search {
    pub projects: Vec<ProjectInfo>,
    /// Why the bucket was not read: the projects of this machine are listed all the same.
    pub bucket_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pulled {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
    /// `None` when the project was not fetched: it was only on this machine.
    pub version: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct Store {
    pub local: Local,
    /// Tests reach a server of their own instead of the Garage of the application.
    #[cfg(test)]
    fixed: Option<S3Config>,
}

impl Store {
    pub fn system() -> Self {
        Self::at(Local::system())
    }

    pub fn at(local: Local) -> Self {
        #[cfg(test)]
        {
            // `test-s3.json` in the folder of the application stands for the Garage.
            let fixed = fs::read_to_string(local.data.join("test-s3.json"))
                .ok()
                .and_then(|t| serde_json::from_str(&t).ok());
            Self { local, fixed }
        }
        #[cfg(not(test))]
        Self { local }
    }

    /// Whether there is a store to use: Garage is there to be started (or was prepared before). Cheap: nothing is
    /// started and nothing goes over the network.
    pub fn configured(&self) -> bool {
        #[cfg(test)]
        if self.fixed.is_some() {
            return true;
        }
        crate::garage::available(&self.local)
    }

    /// Reaches the store, starting and preparing Garage first if need be (blocking: a few seconds the first time).
    pub fn remote(&self) -> Result<Remote, String> {
        #[cfg(test)]
        if let Some(config) = &self.fixed {
            return Remote::connect(config);
        }
        Remote::connect(&crate::garage::ensure(&self.local)?)
    }

    /// The id a project file is stored under: the one it already has on this machine, else its name.
    pub fn id_of(&self, path: &Path, name: &str) -> String {
        self.local
            .state()
            .find_path(path)
            .map_or_else(|| slug(name), |r| r.id.clone())
    }

    /// Notes that the app opened this file.
    pub fn opened(&self, path: &Path, name: &str) -> Result<String, String> {
        let id = self.id_of(path, name);
        self.local.update_state(|s| s.touch(&id, name, path))?;
        Ok(id)
    }

    /// Whether the file holds something the bucket does not have: it was never stored, or changed since.
    pub fn unsynced(&self, path: &Path) -> bool {
        let Ok(bytes) = fs::read(path) else {
            return false;
        };
        self.local
            .state()
            .find_path(path)
            .is_none_or(|r| r.synced_sha.as_deref() != Some(&sha256_hex(&bytes)))
    }

    /// Keeps the file as a new version in the bucket (nothing if it did not change since the last one).
    pub fn push_file(&self, path: &Path) -> Result<Pushed, String> {
        let bytes = fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let project: sb_model::Project = serde_json::from_slice(&bytes)
            .map_err(|e| format!("{} is not a valid project: {e}", path.display()))?;
        let id = self.opened(path, &project.name)?;
        let pushed = self.remote()?.push(&id, &project.name, &bytes)?;
        self.local.update_state(|s| {
            s.synced(&id, Some(sha256_hex(&bytes)), Some(pushed.version));
        })?;
        Ok(pushed)
    }

    /// The projects of the bucket and the ones opened here, whose id or name holds `query`.
    pub fn search(&self, query: &str) -> Search {
        let query = query.trim().to_lowercase();
        let local = self.local.state();
        let mut out = Search::default();
        let mut found: Vec<ProjectInfo> = vec![];
        match self.remote().and_then(|r| r.projects()) {
            Ok(projects) => {
                for p in projects {
                    let here = local.find(&p.id).is_some();
                    found.push(ProjectInfo {
                        id: p.id,
                        name: p.name,
                        version: Some(p.version),
                        updated_at: p.saved_at,
                        source: if here { Source::Both } else { Source::Bucket },
                    });
                }
            }
            Err(e) => out.bucket_error = Some(e),
        }
        for r in &local.recent {
            if !found.iter().any(|p| p.id == r.id) {
                found.push(ProjectInfo {
                    id: r.id.clone(),
                    name: r.name.clone(),
                    version: None,
                    updated_at: s3::iso8601(r.opened_at),
                    source: Source::Local,
                });
            }
        }
        out.projects = found
            .into_iter()
            .filter(|p| {
                query.is_empty() || p.id.contains(&query) || p.name.to_lowercase().contains(&query)
            })
            .collect();
        out
    }

    /// Makes the working copy of a project from the bucket (a version, or the latest) and makes it the last one
    /// opened. A copy that holds changes the bucket does not have is kept unless `discard_local`.
    ///
    /// A project that is only on this machine, or a bucket that cannot be reached, leaves the copy as it is: that
    /// is the project still opened.
    pub fn pull(
        &self,
        id: &str,
        version: Option<u32>,
        discard_local: bool,
    ) -> Result<Pulled, String> {
        check_id(id)?;
        let state = self.local.state();
        let path = state
            .find(id)
            .map(|r| PathBuf::from(&r.path))
            .unwrap_or_else(|| self.local.working_copy(id));
        let remote = self.remote();
        let known = state.find(id).filter(|_| path.is_file());
        let remote = match (remote, known) {
            (Ok(remote), _) => remote,
            (Err(_), Some(r)) if version.is_none() => {
                // Nothing to fetch it from, but it is here.
                self.local.update_state(|s| s.touch(id, &r.name, &path))?;
                return Ok(Pulled {
                    id: id.into(),
                    name: r.name.clone(),
                    path,
                    version: None,
                });
            }
            (Err(e), _) => return Err(e),
        };
        let (number, bytes) = match remote.read(id, version) {
            Ok(found) => found,
            Err(_) if version.is_none() && known.is_some() => {
                // Never stored: the copy of this machine is the project.
                let r = known.expect("checked");
                self.local.update_state(|s| s.touch(id, &r.name, &path))?;
                return Ok(Pulled {
                    id: id.into(),
                    name: r.name.clone(),
                    path,
                    version: None,
                });
            }
            Err(e) => return Err(e),
        };
        let project: sb_model::Project = serde_json::from_slice(&bytes)
            .map_err(|e| format!("{id} version {number} is not a valid project: {e}"))?;
        let sha = sha256_hex(&bytes);
        if path.is_file() && !discard_local {
            let local_sha = sha256_hex(&fs::read(&path).unwrap_or_default());
            let unsynced = state
                .find(id)
                .is_none_or(|r| r.synced_sha.as_deref() != Some(&local_sha));
            if unsynced && local_sha != sha {
                return Err(format!(
                    "the copy of {id} on this machine ({}) has changes that are not stored: save_version first, or load again with discard_local",
                    path.display()
                ));
            }
        }
        write_private_copy(&path, &bytes)?;
        // An old version is not what the bucket holds as its latest: the copy is a change to store.
        let is_latest = remote.meta(id)?.is_none_or(|m| m.version == number);
        self.local.update_state(|s| {
            s.touch(id, &project.name, &path);
            s.synced(id, is_latest.then_some(sha), is_latest.then_some(number));
        })?;
        Ok(Pulled {
            id: id.into(),
            name: project.name,
            path,
            version: Some(number),
        })
    }
}

/// A working copy is a project file: written whole, in a folder made if need be.
fn write_private_copy(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let fail = |e: std::io::Error| format!("cannot write {}: {e}", path.display());
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(fail)?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes).map_err(fail)?;
    fs::rename(&tmp, path).map_err(fail)
}

// ---- with the folders of the system ----------------------------------------------------

pub fn push_file(path: &Path) -> Result<Pushed, String> {
    Store::system().push_file(path)
}

pub fn search(query: &str) -> Result<Vec<ProjectInfo>, String> {
    let found = Store::system().search(query);
    match found.bucket_error {
        Some(e) if found.projects.is_empty() => Err(e),
        Some(e) => {
            eprintln!("warning: {e}");
            Ok(found.projects)
        }
        None => Ok(found.projects),
    }
}

pub fn pull(id: &str, version: Option<u32>, discard_local: bool) -> Result<Pulled, String> {
    Store::system().pull(id, version, discard_local)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::s3::testing::fake_server;
    use sb_model::Project;

    fn store(tag: &str) -> Store {
        let dir = std::env::temp_dir().join(format!("sb-store-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let (config, _) = fake_server();
        let local = Local {
            data: dir.join("data"),
        };
        fs::create_dir_all(&local.data).unwrap();
        fs::write(
            local.data.join("test-s3.json"),
            serde_json::to_string(&config).unwrap(),
        )
        .unwrap();
        Store::at(local)
    }

    fn write(store: &Store, id: &str, project: &Project) -> PathBuf {
        let path = store.local.working_copy(id);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        crate::save_project(&path, project).unwrap();
        path
    }

    #[test]
    fn a_project_is_identified_by_the_name_it_had() {
        assert_eq!(slug("Orders Demo"), "orders-demo");
        assert_eq!(slug("  --Clairtyx v1!"), "clairtyx-v1");
        assert_eq!(slug("???"), "project");
        assert!(check_id("orders-demo").is_ok());
        for bad in ["", "../x", "A", "a/b", "a b"] {
            assert!(check_id(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn every_change_is_a_version_and_an_unchanged_file_is_not() {
        let store = store("versions");
        let mut project = sb_model::sample::orders();
        let path = write(&store, "orders", &project);
        let first = store.push_file(&path).unwrap();
        assert_eq!(
            first,
            Pushed {
                version: 1,
                created: true
            }
        );
        assert!(!store.unsynced(&path));
        // Same bytes: nothing new.
        assert_eq!(
            store.push_file(&path).unwrap(),
            Pushed {
                version: 1,
                created: false
            }
        );

        project.name = "Orders Renamed".into();
        crate::save_project(&path, &project).unwrap();
        assert!(store.unsynced(&path));
        assert_eq!(
            store.push_file(&path).unwrap(),
            Pushed {
                version: 2,
                created: true
            }
        );
        // Renaming does not move the project: same id, same history.
        let remote = store.remote().unwrap();
        let listed: Vec<_> = remote
            .projects()
            .unwrap()
            .into_iter()
            .map(|p| (p.id, p.name, p.version))
            .collect();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].2, 2);
        assert_eq!(listed[0].1, "Orders Renamed");
        let versions = remote.versions(&listed[0].0).unwrap();
        assert_eq!(
            versions.iter().map(|v| v.number).collect::<Vec<_>>(),
            [2, 1]
        );
        let (n, old) = remote.read(&listed[0].0, Some(1)).unwrap();
        assert_eq!(n, 1);
        assert!(
            String::from_utf8(old)
                .unwrap()
                .contains("\"name\": \"Orders\"")
        );
        assert!(
            remote
                .read(&listed[0].0, Some(9))
                .unwrap_err()
                .contains("no version 9")
        );
    }

    #[test]
    fn the_last_projects_opened_are_remembered_most_recent_first() {
        let store = store("recent");
        for name in ["One", "Two", "Three"] {
            let path = write(&store, &slug(name), &Project::new(name));
            store.opened(&path, name).unwrap();
        }
        let state = store.local.state();
        assert_eq!(
            state
                .recent
                .iter()
                .map(|r| r.id.as_str())
                .collect::<Vec<_>>(),
            ["three", "two", "one"]
        );
        assert_eq!(state.last.as_deref(), Some("three"));
        // Opening one again brings it to the front without a duplicate.
        let one = store.local.working_copy("one");
        store.opened(&one, "One").unwrap();
        let state = store.local.state();
        assert_eq!(state.recent.len(), 3);
        assert_eq!(state.last_open().unwrap().id, "one");
        // A file that is gone is not opened again.
        fs::remove_file(&one).unwrap();
        assert!(store.local.state().last_open().is_none());
        // The list stays short.
        for i in 0..20 {
            let name = format!("p{i}");
            let path = write(&store, &name, &Project::new(&name));
            store.opened(&path, &name).unwrap();
        }
        assert_eq!(store.local.state().recent.len(), MAX_RECENT);
    }

    #[test]
    fn a_file_keeps_its_id_when_the_project_is_renamed() {
        let store = store("rename");
        let path = write(&store, "orders", &Project::new("Orders"));
        assert_eq!(store.opened(&path, "Orders").unwrap(), "orders");
        assert_eq!(store.id_of(&path, "Something Else"), "orders");
        assert_eq!(
            store.id_of(&PathBuf::from("/elsewhere/x.sb.json"), "Something Else"),
            "something-else"
        );
    }

    #[test]
    fn search_finds_the_bucket_and_this_machine() {
        let store = store("search");
        let path = write(&store, "orders", &Project::new("Orders"));
        store.push_file(&path).unwrap();
        let only_here = write(&store, "draft", &Project::new("Draft"));
        store.opened(&only_here, "Draft").unwrap();

        let all = store.search("").projects;
        assert_eq!(all.len(), 2);
        let orders = all.iter().find(|p| p.id == "orders").unwrap();
        assert_eq!((orders.version, orders.source), (Some(1), Source::Both));
        let draft = all.iter().find(|p| p.id == "draft").unwrap();
        assert_eq!((draft.version, draft.source), (None, Source::Local));
        assert_eq!(store.search("ORD").projects.len(), 1);
        assert!(store.search("zzz").projects.is_empty());

        // The bucket does not answer: what is here is still found, and the reason is given.
        let down = Store::at(Local {
            data: store.local.data.join("elsewhere"),
        });
        fs::create_dir_all(&down.local.data).unwrap();
        fs::copy(store.local.state_file(), down.local.state_file()).unwrap();
        let found = down.search("");
        assert_eq!(found.projects.len(), 2);
        assert!(found.bucket_error.unwrap().contains("introuvable"));
    }

    #[test]
    fn pulling_makes_the_working_copy_and_protects_unsaved_changes() {
        let store = store("pull");
        let path = write(&store, "orders", &Project::new("Orders"));
        store.push_file(&path).unwrap();
        // Another machine: the copy is not there.
        fs::remove_file(&path).unwrap();
        let pulled = store.pull("orders", None, false).unwrap();
        assert_eq!((pulled.version, pulled.name.as_str()), (Some(1), "Orders"));
        assert_eq!(pulled.path, path);
        assert!(!store.unsynced(&path));
        assert_eq!(store.local.state().last.as_deref(), Some("orders"));

        // Changes made here and not stored are not thrown away.
        let mut edited = Project::new("Orders");
        edited.topics.clear();
        edited.name = "Orders edited".into();
        crate::save_project(&path, &edited).unwrap();
        let refused = store.pull("orders", None, false).unwrap_err();
        assert!(refused.contains("not stored"), "{refused}");
        assert!(fs::read_to_string(&path).unwrap().contains("Orders edited"));
        // Unless asked for.
        store.pull("orders", None, true).unwrap();
        assert!(fs::read_to_string(&path).unwrap().contains("\"Orders\""));

        // An old version is a change to store: the copy is not in sync with the latest.
        crate::save_project(&path, &edited).unwrap();
        store.push_file(&path).unwrap();
        let old = store.pull("orders", Some(1), false).unwrap();
        assert_eq!(old.version, Some(1));
        assert!(store.unsynced(&path));
        assert!(
            store
                .pull("orders", Some(7), true)
                .unwrap_err()
                .contains("no version 7")
        );
        assert!(
            store
                .pull("nope", None, false)
                .unwrap_err()
                .contains("no project nope")
        );
        assert!(store.pull("../etc", None, false).is_err());
    }
}
