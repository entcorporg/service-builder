//! The files that run the services in containers (compose, Traefik, Garage, the image), written by the
//! "Exporter l'infra" button. Service Builder does not start anything itself: it hands the files over.
//!
//! They are meant to sit in the folder of the code export, next to `Cargo.toml`: the image is built from it.

use std::fs;
use std::path::Path;

use sb_codegen::GeneratedFile;
use sb_model::{Project, Severity};

pub use sb_codegen::pods::{Link, links, stack_name};

/// The files of the stack, for the project as it is.
pub fn stack_files(project: &Project) -> Vec<GeneratedFile> {
    sb_codegen::pods::files_with(project, &env_file_mounts(project))
}

/// The files of the host that the environment file names by an absolute path (`DKIM_PRIVATE_KEY_PATH=/…/dkim.pem`).
/// A container does not see them unless they are mounted: the stack mounts each one at the same path.
pub fn env_file_mounts(project: &Project) -> Vec<String> {
    let Ok(text) = fs::read_to_string(project.pods.env_file.trim()) else {
        return vec![];
    };
    let mut paths: Vec<String> = text
        .lines()
        .filter_map(env_value)
        // A colon would be read as the separator of the mount.
        .filter(|value| value.starts_with('/') && !value.contains(':'))
        .filter(|value| Path::new(value).is_file())
        .collect();
    paths.sort();
    paths.dedup();
    paths
}

/// The value of a `NAME=value` line, as compose reads it: quotes removed, and without the comment that follows a
/// value that has none.
fn env_value(line: &str) -> Option<String> {
    let line = line.trim();
    let line = line.strip_prefix("export ").unwrap_or(line);
    if line.starts_with('#') {
        return None;
    }
    let value = line.split_once('=')?.1.trim();
    for quote in ['"', '\''] {
        if let Some(rest) = value.strip_prefix(quote) {
            return rest.split_once(quote).map(|(inside, _)| inside.to_string());
        }
    }
    let value = value.split_once(" #").map_or(value, |(before, _)| before);
    Some(value.trim_end().to_string())
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InfraReport {
    pub dir: String,
    /// Every file of the stack, by its path in `dir`.
    pub files: Vec<String>,
}

/// Writes the files of the stack into `dir`. A file whose content did not change is left alone (its date is what
/// tells cargo, in the build image, that there is nothing to compile again).
pub fn export_infra(project: &Project, dir: &Path) -> Result<InfraReport, String> {
    let errors = sb_codegen::diagnose(project)
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .count();
    if errors > 0 {
        return Err(format!(
            "the project has {errors} error(s), fix them before exporting the infrastructure"
        ));
    }
    let files = stack_files(project);
    if project.uses_garage() {
        garage_secrets(dir)?;
    }
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
    Ok(InfraReport {
        dir: dir.display().to_string(),
        files: files.into_iter().map(|f| f.path).collect(),
    })
}

/// `bytes` random bytes in hexadecimal, from the generator of the system.
fn random_hex(bytes: usize) -> Result<String, String> {
    use std::io::Read;
    let mut buffer = vec![0u8; bytes];
    fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut buffer))
        .map_err(|e| format!("cannot read /dev/urandom: {e}"))?;
    Ok(buffer.iter().map(|b| format!("{b:02x}")).collect())
}

/// The secrets of Garage, drawn once per stack and kept next to it, out of the project: the secret of the node, and
/// the access key the services reach it with (an identifier and a secret in the form Garage imports). A file that is
/// there is never written again: the key it holds is the one the store knows.
fn garage_secrets(dir: &Path) -> Result<(), String> {
    use sb_codegen::pods::{GARAGE_RPC_ENV, GARAGE_S3_ENV};
    let draw = |path: &str, content: &dyn Fn() -> Result<String, String>| {
        let target = dir.join(path);
        if target.exists() {
            return Ok(());
        }
        let text = content()?;
        let write = || {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
            std::io::Write::write_all(&mut options.open(&target)?, text.as_bytes())
        };
        write().map_err(|e| format!("cannot write {}: {e}", target.display()))
    };
    draw(GARAGE_RPC_ENV, &|| {
        Ok(format!("GARAGE_RPC_SECRET={}\n", random_hex(32)?))
    })?;
    draw(GARAGE_S3_ENV, &|| {
        Ok(format!(
            "AWS_ACCESS_KEY_ID=GK{}\nAWS_SECRET_ACCESS_KEY={}\n",
            random_hex(12)?,
            random_hex(32)?
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sb_codegen::pods::{BASE_COMPOSE_FILE, COMPOSE_FILE, DOCKERFILE};
    use sb_model::sample;
    use std::time::Duration;

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("sb-infra-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn the_files_the_environment_file_names_are_mounted() {
        let dir = temp("env");
        fs::create_dir_all(&dir).unwrap();
        let key = dir.join("dkim.pem").display().to_string();
        fs::write(&key, "key").unwrap();
        let env = dir.join(".env");
        fs::write(
            &env,
            format!(
                "# keys\nSMTP_HOST=mailpit   # dev relay\nDKIM_PRIVATE_KEY_PATH={key}    # dev key\n\
                 export COPY=\"{key}\"\nMISSING=/nowhere/key.pem\nFOLDER={}\n",
                dir.display()
            ),
        )
        .unwrap();
        let mut project = sample::orders();
        project.pods.env_file = env.display().to_string();
        assert_eq!(env_file_mounts(&project), vec![key.clone()]);
        let compose = stack_files(&project)
            .into_iter()
            .find(|f| f.path == COMPOSE_FILE)
            .unwrap()
            .content;
        assert!(
            compose.contains(&format!("- \"{key}:{key}:ro\"")),
            "{compose}"
        );
        // No environment file, or one that cannot be read: nothing is mounted.
        project.pods.env_file = dir.join("absent").display().to_string();
        assert!(env_file_mounts(&project).is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_infrastructure_is_exported_without_the_code_and_left_alone_when_nothing_changed() {
        let dir = temp("export");
        let mut project = sample::orders();
        project.pods.traefik.enabled = true;
        let report = export_infra(&project, &dir).unwrap();
        for path in [
            BASE_COMPOSE_FILE,
            COMPOSE_FILE,
            DOCKERFILE,
            sb_codegen::pods::BUILD_SCRIPT,
            "traefik/traefik.yml",
            "traefik/dynamic/routers.yaml",
        ] {
            assert!(dir.join(path).exists(), "{path}");
            assert!(report.files.iter().any(|f| f == path), "{path}");
        }
        // Not the code: that is the other button.
        assert!(!dir.join("Cargo.toml").exists());

        let stamp = |path: &str| fs::metadata(dir.join(path)).unwrap().modified().unwrap();
        let before = stamp(BASE_COMPOSE_FILE);
        std::thread::sleep(Duration::from_millis(20));
        project.pods.service_memory = "200m".into();
        export_infra(&project, &dir).unwrap();
        assert_eq!(stamp(BASE_COMPOSE_FILE), before);
        assert!(
            fs::read_to_string(dir.join(COMPOSE_FILE))
                .unwrap()
                .contains("mem_limit: 200m")
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_secrets_of_garage_are_drawn_once() {
        use sb_codegen::pods::{GARAGE_INIT, GARAGE_RPC_ENV, GARAGE_S3_ENV};
        let dir = temp("garage");
        // Without an object step, nothing of Garage is written.
        export_infra(&sample::orders(), &dir).unwrap();
        assert!(!dir.join("garage").exists());

        let mut project = sample::orders();
        sample::add_jwt(&mut project);
        sample::add_avatars(&mut project);
        export_infra(&project, &dir).unwrap();
        assert!(dir.join(GARAGE_INIT).exists() && dir.join("garage/garage.toml").exists());
        let read = |path: &str| fs::read_to_string(dir.join(path)).unwrap();
        let rpc = read(GARAGE_RPC_ENV);
        let secret = rpc.trim().strip_prefix("GARAGE_RPC_SECRET=").unwrap();
        assert!(secret.len() == 64 && secret.bytes().all(|b| b.is_ascii_hexdigit()));
        let s3 = read(GARAGE_S3_ENV);
        let mut lines = s3.lines();
        let id = lines
            .next()
            .unwrap()
            .strip_prefix("AWS_ACCESS_KEY_ID=GK")
            .unwrap();
        let key = lines
            .next()
            .unwrap()
            .strip_prefix("AWS_SECRET_ACCESS_KEY=")
            .unwrap();
        assert_eq!((id.len(), key.len()), (24, 64));
        assert_ne!(secret, key);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(dir.join(GARAGE_S3_ENV))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o077, 0, "{mode:o}");
        }
        // The next export keeps them: the store knows this key.
        export_infra(&project, &dir).unwrap();
        assert_eq!((read(GARAGE_RPC_ENV), read(GARAGE_S3_ENV)), (rpc, s3));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_project_with_errors_exports_nothing() {
        let dir = temp("bad");
        let mut project = sample::orders();
        project.pods.service_memory = "lots".into();
        assert!(
            export_infra(&project, &dir)
                .unwrap_err()
                .contains("error(s)")
        );
        assert!(!dir.exists());
    }
}
