//! Runs the generated services in containers, without an export: the workspace is written into a folder of the
//! application, built into one image, and started with compose next to its infrastructure.
//!
//! The engine is a rootless Podman with a store of its own when Podman is installed: images, containers, volumes and
//! networks live in a folder of the application and never mix with the user's. Without Podman, Docker is used, and the
//! stack is kept apart by its compose project name.
//!
//! Everything here blocks: the callers (the Pods page, the CLI) run it on a thread.

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{RecvTimeoutError, channel};
use std::time::Duration;

pub use sb_codegen::GeneratedFile;
use sb_codegen::pods::{COMPOSE_FILE, DOCKERFILE};
pub use sb_codegen::pods::{Link, links, stack_name};
use sb_model::Project;
use serde::Deserialize;

/// Set by the caller to stop a command that is running (a build takes minutes).
pub type Cancel = Arc<AtomicBool>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineKind {
    Podman,
    Docker,
}

#[derive(Debug, Clone)]
pub struct Engine {
    pub kind: EngineKind,
    /// Added to the environment of every call: what points Podman at the private store.
    env: Vec<(String, String)>,
    /// The folder of the private store, when there is one.
    pub store: Option<PathBuf>,
}

/// `$XDG_DATA_HOME/service-builder`, or `~/.local/share/service-builder`.
fn data_dir() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var_os("HOME").unwrap_or_else(|| ".".into());
            PathBuf::from(home).join(".local").join("share")
        });
    base.join("service-builder")
}

fn answers(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

impl Engine {
    /// The engine the stack runs on: an isolated Podman if it is installed (with `podman-compose`), Docker otherwise.
    pub fn detect() -> Result<Self, String> {
        if answers("podman", &["--version"]) && answers("podman-compose", &["--version"]) {
            return Self::isolated_podman();
        }
        if answers("docker", &["compose", "version"]) {
            return Ok(Self {
                kind: EngineKind::Docker,
                env: vec![],
                store: None,
            });
        }
        Err(
            "Aucun moteur de conteneurs trouvé. Installez Podman (avec podman-compose) pour une exécution isolée, \
             ou Docker avec son extension compose."
                .into(),
        )
    }

    /// Podman with its store under the data folder of the application. The isolation is a matter of environment
    /// variables, so it also holds for the `podman` processes that `podman-compose` starts.
    fn isolated_podman() -> Result<Self, String> {
        let base = data_dir().join("engine");
        let (data, config) = (base.join("data"), base.join("config"));
        let containers = config.join("containers");
        for dir in [&data, &containers] {
            fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        }
        // Without it a rootless Podman does not resolve short image names (`postgres:16`).
        let registries = containers.join("registries.conf");
        fs::write(
            &registries,
            "unqualified-search-registries = [\"docker.io\"]\nshort-name-mode = \"permissive\"\n",
        )
        .map_err(|e| format!("cannot write {}: {e}", registries.display()))?;
        Ok(Self {
            kind: EngineKind::Podman,
            env: vec![
                ("XDG_DATA_HOME".into(), data.display().to_string()),
                ("XDG_CONFIG_HOME".into(), config.display().to_string()),
                (
                    "CONTAINERS_REGISTRIES_CONF".into(),
                    registries.display().to_string(),
                ),
            ],
            store: Some(base),
        })
    }

    pub fn label(&self) -> &'static str {
        match self.kind {
            EngineKind::Podman => "Podman (store isolé)",
            EngineKind::Docker => "Docker",
        }
    }

    fn program(&self) -> &'static str {
        match self.kind {
            EngineKind::Podman => "podman",
            EngineKind::Docker => "docker",
        }
    }

    /// `podman …` or `docker …`.
    fn command(&self) -> Command {
        let mut c = Command::new(self.program());
        c.envs(self.env.iter().cloned());
        // Plain lines instead of a redrawn progress: the output is shown in a log.
        c.env("BUILDKIT_PROGRESS", "plain");
        c.env("CARGO_TERM_COLOR", "never");
        c
    }

    /// The compose tool on the stack in `dir`. For Podman it is `podman-compose`, which drives `podman` itself:
    /// `podman compose` may hand over to the Docker plugin, and to the Docker daemon with it.
    fn compose(&self, stack: &str, dir: &Path) -> Command {
        let mut c = match self.kind {
            EngineKind::Podman => Command::new("podman-compose"),
            EngineKind::Docker => {
                let mut c = Command::new("docker");
                c.arg("compose");
                c
            }
        };
        c.envs(self.env.iter().cloned());
        c.args(["-p", stack, "-f", COMPOSE_FILE]).current_dir(dir);
        c
    }
}

/// Where the stack of a project is written: one folder per project name.
pub fn stack_dir(project: &Project) -> PathBuf {
    data_dir()
        .join("pods")
        .join(sb_codegen::pods::stack_name(project))
}

/// The files of the stack that are not part of an export: Dockerfile, compose, Traefik.
pub fn stack_files(project: &Project) -> Vec<GeneratedFile> {
    sb_codegen::pods::files(project)
}

/// Writes the workspace and the files of the stack into `dir`. A file whose content did not change is left alone:
/// its date is what tells cargo, inside the image, that it has nothing to compile again.
pub fn prepare(project: &Project, dir: &Path) -> Result<usize, String> {
    let analysis = crate::analyze(project);
    if analysis.files.is_empty() {
        let n = analysis
            .diagnostics
            .iter()
            .filter(|d| d.severity == sb_model::Severity::Error)
            .count();
        return Err(format!(
            "the project has {n} error(s), fix them before starting it"
        ));
    }
    let mut files = analysis.files;
    files.extend(stack_files(project));
    for f in &files {
        let target = dir.join(&f.path);
        if fs::read_to_string(&target).is_ok_and(|current| current == f.content) {
            continue;
        }
        let write = || {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&target, &f.content)
        };
        write().map_err(|e| format!("cannot write {}: {e}", target.display()))?;
    }
    Ok(files.len())
}

/// Runs the command to its end, handing each line it prints (both streams) to `log`. `cancel` kills it.
fn stream(mut command: Command, log: &dyn Fn(&str), cancel: &Cancel) -> Result<(), String> {
    let shown = format!("{command:?}");
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run {shown}: {e}"))?;
    let (tx, rx) = channel::<String>();
    let mut readers = vec![];
    let out = child
        .stdout
        .take()
        .map(|s| Box::new(s) as Box<dyn std::io::Read + Send>);
    let err = child
        .stderr
        .take()
        .map(|s| Box::new(s) as Box<dyn std::io::Read + Send>);
    for pipe in [out, err].into_iter().flatten() {
        let tx = tx.clone();
        readers.push(std::thread::spawn(move || {
            for line in BufReader::new(pipe).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        }));
    }
    drop(tx);
    let mut cancelled = false;
    loop {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(line) => log(&line),
            Err(RecvTimeoutError::Timeout) => {}
            // Both pipes are closed: the command is over.
            Err(RecvTimeoutError::Disconnected) => break,
        }
        if cancel.load(Ordering::Relaxed) && !cancelled {
            cancelled = true;
            let _ = child.kill();
        }
    }
    for r in readers {
        let _ = r.join();
    }
    let status = child.wait().map_err(|e| format!("{shown}: {e}"))?;
    if cancelled {
        return Err("interrompu".into());
    }
    if !status.success() {
        return Err(match status.code() {
            Some(code) => format!("la commande a échoué (code {code})"),
            None => "la commande a été interrompue".into(),
        });
    }
    Ok(())
}

/// Runs the command and gives back what it printed.
fn output(mut command: Command) -> Result<String, String> {
    let shown = format!("{command:?}");
    let out = command
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("cannot run {shown}: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// Writes the stack, compiles the services and starts everything. `log` receives the progress, line by line.
///
/// The services are compiled in a container of the builder image, with the registry of cargo and the build directory
/// in volumes: the second build only compiles what changed. A service whose binary changed is restarted.
pub fn up(
    engine: &Engine,
    project: &Project,
    dir: &Path,
    log: &dyn Fn(&str),
    cancel: &Cancel,
) -> Result<(), String> {
    use sb_codegen::pods as stack_gen;
    let stack = stack_gen::stack_name(project);
    let files = prepare(project, dir)?;
    log(&format!("▸ {files} fichiers dans {}", dir.display()));

    log("▸ Images de compilation et d'exécution…");
    for (target, image) in [
        ("builder", stack_gen::builder_image(project)),
        ("runtime", stack_gen::image(project)),
    ] {
        let mut build = engine.command();
        build
            .args([
                "build", "--target", target, "-t", &image, "-f", DOCKERFILE, ".",
            ])
            .current_dir(dir);
        stream(build, log, cancel)?;
    }

    // What runs now: a service that is already up keeps its old binary until it is restarted.
    let running: Vec<String> = ps(engine, &stack)
        .unwrap_or_default()
        .into_iter()
        .filter(|c| c.state == "running")
        .map(|c| c.service)
        .collect();

    log("▸ Compilation des services (plusieurs minutes la première fois)…");
    let builder = format!("{stack}-builder");
    // One left behind by an interrupted build would refuse the name.
    let remove = || {
        let mut rm = engine.command();
        rm.args(["rm", "-f", &builder]);
        let _ = output(rm);
    };
    remove();
    let mut build = engine.command();
    build.args(["run", "--rm", "--name", &builder, "-v"]);
    build.arg(format!("{}:/src:ro", dir.display()));
    for (volume, at) in stack_gen::build_volumes(project) {
        build.args(["-v", &format!("{volume}:{at}")]);
    }
    if !project.pods.cpuset.is_empty() {
        build.args(["--cpuset-cpus", &project.pods.cpuset]);
    }
    build.args([
        &stack_gen::builder_image(project),
        "sh",
        &format!("/src/{}", stack_gen::BUILD_SCRIPT),
    ]);
    let changed = std::sync::Mutex::new(Vec::<String>::new());
    let built = stream(
        build,
        &|line| match line.strip_prefix(stack_gen::CHANGED_MARK) {
            Some(name) => changed
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(name.trim().to_string()),
            None => log(line),
        },
        cancel,
    );
    if built.is_err() {
        // Killing the client does not stop the container.
        remove();
    }
    built?;

    log("▸ Démarrage des conteneurs…");
    let mut start = engine.compose(&stack, dir);
    // Containers of services that are no longer in the project go away with `--remove-orphans`.
    start.args(["up", "-d", "--remove-orphans"]);
    stream(start, log, cancel)?;

    let restart: Vec<String> = changed
        .into_inner()
        .unwrap_or_else(|e| e.into_inner())
        .into_iter()
        .filter(|name| running.contains(name))
        .collect();
    if !restart.is_empty() {
        log(&format!(
            "▸ Redémarrage de {} (nouveau binaire)…",
            restart.join(", ")
        ));
        let mut again = engine.compose(&stack, dir);
        again.arg("restart").args(&restart);
        stream(again, log, cancel)?;
    }
    log("▸ La stack est démarrée.");
    Ok(())
}

/// Stops the stack and removes its containers. `volumes` also deletes its data (the databases).
pub fn down(
    engine: &Engine,
    project: &Project,
    dir: &Path,
    volumes: bool,
    log: &dyn Fn(&str),
    cancel: &Cancel,
) -> Result<(), String> {
    if !dir.join(COMPOSE_FILE).exists() {
        return Ok(());
    }
    let mut stop = engine.compose(&sb_codegen::pods::stack_name(project), dir);
    stop.args(["down", "--remove-orphans"]);
    if volumes {
        stop.arg("-v");
    }
    log("▸ Arrêt de la stack…");
    stream(stop, log, cancel)?;
    log("▸ La stack est arrêtée.");
    Ok(())
}

/// A container of the stack, as the engine lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Container {
    pub id: String,
    /// The name of its service in the compose file (`orders`, `kafka`).
    pub service: String,
    pub name: String,
    pub image: String,
    /// `running`, `exited`, `created`…
    pub state: String,
    /// `Up 2 minutes (healthy)`.
    pub status: String,
}

/// One line of `ps --format json`. Docker prints one object per line with `Names` as a text, Podman an array of
/// objects with `Names` as a list: both are read.
#[derive(Deserialize)]
struct PsRow {
    #[serde(alias = "ID", alias = "Id", default)]
    id: String,
    #[serde(alias = "Names", default)]
    names: serde_json::Value,
    #[serde(alias = "Image", default)]
    image: String,
    #[serde(alias = "State", default)]
    state: String,
    #[serde(alias = "Status", default)]
    status: String,
    #[serde(alias = "Labels", default)]
    labels: serde_json::Value,
}

const SERVICE_LABEL: &str = "com.docker.compose.service";

fn parse_ps(text: &str) -> Vec<Container> {
    let rows: Vec<PsRow> = match serde_json::from_str::<Vec<PsRow>>(text) {
        Ok(rows) => rows,
        Err(_) => text
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect(),
    };
    let mut containers: Vec<Container> = rows
        .into_iter()
        .map(|r| {
            let name = match &r.names {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Array(a) => a
                    .first()
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
                _ => String::new(),
            };
            // Docker gives the labels as `a=b,c=d`, Podman as a map.
            let service = match &r.labels {
                serde_json::Value::Object(map) => map
                    .get(SERVICE_LABEL)
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                serde_json::Value::String(s) => s.split(',').find_map(|pair| {
                    pair.strip_prefix(SERVICE_LABEL)
                        .and_then(|rest| rest.strip_prefix('='))
                        .map(str::to_string)
                }),
                _ => None,
            };
            Container {
                service: service.unwrap_or_else(|| name.clone()),
                id: r.id,
                name,
                image: r.image,
                state: r.state.to_lowercase(),
                status: r.status,
            }
        })
        .collect();
    containers.sort_by(|a, b| a.service.cmp(&b.service));
    containers
}

/// The containers of the stack named `stack` ([`stack_name`]), running or not.
pub fn ps(engine: &Engine, stack: &str) -> Result<Vec<Container>, String> {
    let mut c = engine.command();
    c.args(["ps", "-a", "--no-trunc", "--format", "json", "--filter"])
        .arg(format!("label=com.docker.compose.project={stack}"));
    output(c).map(|text| parse_ps(&text))
}

/// The last lines a container printed, both streams in the order they were written.
pub fn logs(engine: &Engine, id: &str, tail: usize) -> Result<String, String> {
    let mut c = engine.command();
    c.args(["logs", "--tail", &tail.to_string(), id]);
    let out = c
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("cannot read the logs: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    // The services log on stderr, the infrastructure on stdout.
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    Ok(text)
}

/// `start`, `stop` or `restart` on one container.
pub fn container(engine: &Engine, verb: &str, id: &str) -> Result<(), String> {
    if !["start", "stop", "restart"].contains(&verb) {
        return Err(format!("unknown action {verb}"));
    }
    let mut c = engine.command();
    c.args([verb, id]);
    output(c).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sb_model::sample;

    #[test]
    fn both_engines_are_read() {
        let docker = r#"{"ID":"abc","Names":"sb-demo-orders-1","Image":"sb-demo:latest","State":"running","Status":"Up 2 minutes","Labels":"com.docker.compose.project=sb-demo,com.docker.compose.service=orders"}
{"ID":"def","Names":"sb-demo-kafka-1","Image":"apache/kafka:3.9.0","State":"exited","Status":"Exited (1)","Labels":"com.docker.compose.service=kafka"}"#;
        let list = parse_ps(docker);
        assert_eq!(list.len(), 2);
        assert_eq!(
            (list[0].service.as_str(), list[0].state.as_str()),
            ("kafka", "exited")
        );
        assert_eq!(
            (list[1].service.as_str(), list[1].id.as_str()),
            ("orders", "abc")
        );

        let podman = r#"[{"Id":"abc","Names":["sb-demo_orders_1"],"Image":"localhost/sb-demo:latest","State":"running","Status":"Up","Labels":{"com.docker.compose.service":"orders"}}]"#;
        let list = parse_ps(podman);
        assert_eq!(list[0].service, "orders");
        assert_eq!(list[0].name, "sb-demo_orders_1");
        assert!(parse_ps("").is_empty() && parse_ps("[]").is_empty());
    }

    #[test]
    fn the_stack_is_written_once_and_left_alone_when_nothing_changed() {
        let dir = std::env::temp_dir().join(format!("sb-pods-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut project = sample::orders();
        project.pods.traefik.enabled = true;
        let n = prepare(&project, &dir).unwrap();
        assert!(n > 10);
        for path in [
            "Cargo.toml",
            DOCKERFILE,
            sb_codegen::pods::BUILD_SCRIPT,
            COMPOSE_FILE,
            "traefik/traefik.yml",
            "traefik/dynamic/routers.yaml",
        ] {
            assert!(dir.join(path).exists(), "{path}");
        }
        // A second start does not touch what did not change: cargo would compile it again.
        let stamp = |path: &str| fs::metadata(dir.join(path)).unwrap().modified().unwrap();
        let before = stamp("services/orders/src/main.rs");
        std::thread::sleep(Duration::from_millis(20));
        project.pods.service_memory = "200m".into();
        prepare(&project, &dir).unwrap();
        assert_eq!(stamp("services/orders/src/main.rs"), before);
        assert!(
            fs::read_to_string(dir.join(COMPOSE_FILE))
                .unwrap()
                .contains("mem_limit: 200m")
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_project_with_errors_is_not_started() {
        let dir = std::env::temp_dir().join(format!("sb-pods-bad-{}", std::process::id()));
        let mut project = sample::orders();
        project.pods.service_memory = "lots".into();
        assert!(prepare(&project, &dir).unwrap_err().contains("error(s)"));
        assert!(!dir.exists());
    }

    #[test]
    fn a_command_is_streamed_and_can_be_cancelled() {
        let lines = std::sync::Mutex::new(vec![]);
        let mut c = Command::new("sh");
        c.args(["-c", "echo one; echo two >&2; exit 3"]);
        let err = stream(
            c,
            &|l| lines.lock().unwrap().push(l.to_string()),
            &Cancel::default(),
        );
        assert!(err.unwrap_err().contains("code 3"));
        let mut got = lines.into_inner().unwrap();
        got.sort();
        assert_eq!(got, ["one", "two"]);

        let cancel = Cancel::default();
        cancel.store(true, Ordering::Relaxed);
        let mut c = Command::new("sleep");
        c.arg("30");
        assert_eq!(stream(c, &|_| {}, &cancel).unwrap_err(), "interrompu");
    }
}
