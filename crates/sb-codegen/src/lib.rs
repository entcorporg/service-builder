//! Turns a validated [`sb_model::Project`] into a Cargo workspace of Rust services:
//! protobuf contracts, one tonic gRPC server + rdkafka consumers per service, and a
//! docker-compose for Kafka.

mod expr;
mod proto;
pub mod rawcheck;
mod service;
mod workspace;
mod writer;

use std::fs;
use std::io;
use std::path::Path;

use sb_model::{Diagnostic, Project, has_errors};
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GeneratedFile {
    /// Path relative to the workspace root, always `/`-separated.
    pub path: String,
    pub content: String,
}

impl GeneratedFile {
    fn new(path: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            content: content.into(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CodegenError {
    #[error("the project has {} error(s)", .0.iter().filter(|d| d.severity == sb_model::Severity::Error).count())]
    Invalid(Vec<Diagnostic>),
    #[error("internal generator error: {0}")]
    Internal(String),
}

/// Full diagnostics of a project: model validation plus Rust syntax of raw blocks.
pub fn diagnose(project: &Project) -> Vec<Diagnostic> {
    let mut d = sb_model::validate(project);
    d.extend(rawcheck::check(project));
    d
}

/// Generates the whole workspace. Fails with the diagnostics if the project has errors.
pub fn generate(project: &Project) -> Result<Vec<GeneratedFile>, CodegenError> {
    let diagnostics = diagnose(project);
    if has_errors(&diagnostics) {
        return Err(CodegenError::Invalid(diagnostics));
    }
    let mut files = workspace::files(project);
    for svc in &project.services {
        files.extend(service::generate(project, svc)?);
    }
    // Safety net: a generator bug must surface here, not as a cryptic compiler error later.
    for f in files.iter().filter(|f| f.path.ends_with(".rs")) {
        syn::parse_file(&f.content)
            .map_err(|e| CodegenError::Internal(format!("{} is not valid Rust: {e}", f.path)))?;
    }
    Ok(files)
}

/// Formats the Rust files with `rustfmt` when it is installed (best effort: files are left
/// untouched otherwise). Kept out of [`generate`] so generation stays pure and deterministic.
/// Files are formatted in parallel since this runs after every edit in the editor.
pub fn rustfmt(files: &mut [GeneratedFile]) {
    std::thread::scope(|scope| {
        for f in files.iter_mut().filter(|f| f.path.ends_with(".rs")) {
            scope.spawn(move || {
                if let Some(formatted) = rustfmt_one(&f.content) {
                    f.content = formatted;
                }
            });
        }
    });
}

fn rustfmt_one(source: &str) -> Option<String> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let mut child = Command::new("rustfmt")
        .args(["--edition", "2024"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    // Feed stdin from another thread so a large file cannot deadlock against the stdout pipe.
    let mut stdin = child.stdin.take()?;
    let input = source.to_owned();
    let writer = std::thread::spawn(move || stdin.write_all(input.as_bytes()));
    let output = child.wait_with_output().ok()?;
    let _ = writer.join();
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

/// Writes the files under `dir`, creating directories as needed. Existing files with the
/// same path are overwritten; other files are left alone.
pub fn write_to_dir(files: &[GeneratedFile], dir: &Path) -> io::Result<()> {
    for f in files {
        let target = dir.join(&f.path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(target, &f.content)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sb_model::sample;

    fn file<'a>(files: &'a [GeneratedFile], path: &str) -> &'a str {
        &files
            .iter()
            .find(|f| f.path == path)
            .unwrap_or_else(|| panic!("missing {path}"))
            .content
    }

    #[test]
    fn generates_expected_layout() {
        let files = generate(&sample::orders()).unwrap();
        let mut paths: Vec<_> = files.iter().map(|f| f.path.as_str()).collect();
        paths.sort();
        assert_eq!(
            paths,
            vec![
                ".gitignore",
                "Cargo.toml",
                "README.md",
                "contracts/Cargo.toml",
                "contracts/build.rs",
                "contracts/proto/orders_demo.proto",
                "contracts/src/lib.rs",
                "docker-compose.yml",
                "runtime/Cargo.toml",
                "runtime/src/lib.rs",
                "services/notifications/Cargo.toml",
                "services/notifications/src/ctx.rs",
                "services/notifications/src/flows.rs",
                "services/notifications/src/main.rs",
                "services/orders/Cargo.toml",
                "services/orders/src/ctx.rs",
                "services/orders/src/flows.rs",
                "services/orders/src/main.rs",
                "services/orders/src/server.rs",
                "services/payments/Cargo.toml",
                "services/payments/src/ctx.rs",
                "services/payments/src/flows.rs",
                "services/payments/src/main.rs",
            ]
        );
    }

    #[test]
    fn rpc_flow_snapshot() {
        let files = generate(&sample::orders()).unwrap();
        let flows = file(&files, "services/orders/src/flows.rs");
        assert!(
            flows.contains("pub async fn create_order(ctx: &Ctx, input: pb::CreateOrderRequest)")
        );
        assert!(flows.contains("let order_id: String = eda_runtime::new_id();"));
        assert!(flows.contains("ctx.producer"));
        assert!(flows.contains("topics::ORDERS_CREATED"));
        assert!(flows.contains("return Ok(pb::CreateOrderResponse {"));
        assert!(flows.contains("// publish to orders.created"));
    }

    #[test]
    fn consumer_flow_uses_branch_and_rpc_client() {
        let files = generate(&sample::orders()).unwrap();
        let payments = file(&files, "services/payments/src/flows.rs");
        assert!(payments.contains("if (approved.clone()) == (true) {"));
        assert!(payments.contains("} else {"));
        let notifications = file(&files, "services/notifications/src/flows.rs");
        assert!(notifications.contains("ctx.orders.clone().cancel_order("));
        let ctx = file(&files, "services/notifications/src/ctx.rs");
        assert!(
            ctx.contains("pub orders: pb::orders_client::OrdersClient<tonic::transport::Channel>")
        );
        assert!(ctx.contains("ORDERS_URL"));
    }

    #[test]
    fn main_wires_consumers_and_server() {
        let files = generate(&sample::orders()).unwrap();
        let orders = file(&files, "services/orders/src/main.rs");
        assert!(orders.contains("OrdersServer::new"));
        assert!(orders.contains("ORDERS_PORT"));
        let payments = file(&files, "services/payments/src/main.rs");
        assert!(payments.contains("eda_runtime::consume::<pb::OrderCreated"));
        assert!(payments.contains("\"payments-on_order_created\""));
        assert!(payments.contains("ctrl_c"));
        assert!(!payments.contains("mod server"));
    }

    #[test]
    fn refuses_invalid_projects() {
        let mut p = sample::orders();
        p.topics[0].message = "Nope".into();
        match generate(&p) {
            Err(CodegenError::Invalid(d)) => assert!(has_errors(&d)),
            other => panic!("expected Invalid, got {other:?}"),
        }
    }

    #[test]
    fn refuses_bad_rust_blocks() {
        let mut p = sample::orders();
        p.services[1].consumers[0].steps.push(sb_model::Step::Code {
            id: "x".into(),
            code: "let = ;".into(),
        });
        assert!(matches!(generate(&p), Err(CodegenError::Invalid(_))));
    }

    #[test]
    fn generation_is_deterministic() {
        assert_eq!(
            generate(&sample::orders()).unwrap(),
            generate(&sample::orders()).unwrap()
        );
    }

    #[test]
    fn rustfmt_keeps_code_valid() {
        let mut files = generate(&sample::orders()).unwrap();
        rustfmt(&mut files);
        for f in files.iter().filter(|f| f.path.ends_with(".rs")) {
            syn::parse_file(&f.content).unwrap();
        }
        // Whether or not rustfmt is installed, comments survive.
        assert!(
            file(&files, "services/orders/src/flows.rs").contains("// publish to orders.created")
        );
    }

    #[test]
    fn writes_files_to_disk() {
        let dir = std::env::temp_dir().join(format!("sb-codegen-test-{}", std::process::id()));
        let files = generate(&sample::orders()).unwrap();
        write_to_dir(&files, &dir).unwrap();
        assert!(dir.join("services/orders/src/main.rs").exists());
        assert!(dir.join("contracts/proto/orders_demo.proto").exists());
        fs::remove_dir_all(dir).unwrap();
    }
}
