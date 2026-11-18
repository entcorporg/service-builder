//! `sb`: headless access to Service Builder, for CI and scripting.
//!
//! ```text
//! sb sample                          print the demo project as JSON
//! sb validate <project.json>         print diagnostics, exit 1 if there are errors
//! sb generate <project.json> <dir>   write the generated Cargo workspace into <dir>
//! sb infra <project.json> <dir>      write what runs the services in containers: docker-compose.yml, the override, Traefik, Garage
//! sb projects [query]                list the projects of the S3 store (and the ones opened on this machine)
//! sb pull <id> [version]             fetch a project of the store into its working file, print the path
//! sb push <project.json>             store the file as a new version (nothing if it did not change)
//! sb analyze                         project JSON on stdin -> analysis JSON on stdout
//! sb mcp [project.json]              MCP server on stdio, to let an AI client edit the project
//! ```

use std::io::Read;
use std::path::Path;
use std::process::ExitCode;

use sb_model::{Project, Severity, has_errors};

const USAGE: &str = "usage:
  sb sample
  sb validate <project.json>
  sb generate <project.json> <dir>
  sb infra <project.json> <dir>
  sb projects [query]
  sb pull <id> [version]
  sb push <project.json>
  sb analyze            (project JSON on stdin)
  sb mcp [project.json] (MCP server on stdio)";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args.iter().map(String::as_str).collect::<Vec<_>>()) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(2)
        }
    }
}

fn run(args: &[&str]) -> Result<ExitCode, String> {
    match args {
        ["sample"] => {
            let json = serde_json::to_string_pretty(&sb_model::sample::orders())
                .map_err(|e| e.to_string())?;
            println!("{json}");
            Ok(ExitCode::SUCCESS)
        }
        // Every feature at once: the project to try the generator on.
        ["sample", "showcase"] => {
            let json = serde_json::to_string_pretty(&sb_model::sample::showcase())
                .map_err(|e| e.to_string())?;
            println!("{json}");
            Ok(ExitCode::SUCCESS)
        }
        ["validate", file] => {
            let project = sb_app::load_project(Path::new(file))?;
            let analysis = sb_app::analyze(&project);
            print_diagnostics(&analysis.diagnostics);
            Ok(exit_code(&analysis.diagnostics))
        }
        ["generate", file, dir] => {
            let project = sb_app::load_project(Path::new(file))?;
            let diagnostics = sb_codegen::diagnose(&project);
            print_diagnostics(&diagnostics);
            let report = sb_app::export_project(&project, Path::new(dir))?;
            println!("wrote {} files to {}", report.files, report.dir);
            Ok(ExitCode::SUCCESS)
        }
        ["infra", file, dir] => {
            let project = sb_app::load_project(Path::new(file))?;
            print_diagnostics(&sb_codegen::diagnose(&project));
            let report = sb_app::infra::export_infra(&project, Path::new(dir))?;
            for f in &report.files {
                println!("{}/{f}", report.dir);
            }
            println!("start it from the folder of the code export with:");
            for command in sb_codegen::pods::manual_commands(&project, "docker") {
                println!("  {command}");
            }
            Ok(ExitCode::SUCCESS)
        }
        ["projects"] => projects(""),
        ["projects", query] => projects(query),
        ["pull", id] => pull(id, None),
        ["pull", id, version] => pull(
            id,
            Some(
                version
                    .parse()
                    .map_err(|_| format!("{version} is not a version number"))?,
            ),
        ),
        ["push", file] => {
            let pushed = sb_app::store::push_file(Path::new(file))?;
            if pushed.created {
                println!("version {} stored", pushed.version);
            } else {
                println!("unchanged: version {}", pushed.version);
            }
            Ok(ExitCode::SUCCESS)
        }
        ["analyze"] => {
            let mut input = String::new();
            std::io::stdin()
                .read_to_string(&mut input)
                .map_err(|e| e.to_string())?;
            let project: Project =
                serde_json::from_str(&input).map_err(|e| format!("invalid project: {e}"))?;
            let json =
                serde_json::to_string(&sb_app::analyze(&project)).map_err(|e| e.to_string())?;
            println!("{json}");
            Ok(ExitCode::SUCCESS)
        }
        ["mcp", rest @ ..] if rest.len() <= 1 => {
            // stdout belongs to the protocol: anything else goes to stderr.
            let project = rest.first().map(Path::new);
            match project {
                Some(file) => eprintln!("service-builder MCP server on {}", file.display()),
                None => eprintln!(
                    "service-builder MCP server, no project loaded: search_projects, then load_project"
                ),
            }
            sb_app::mcp::serve(project, std::io::stdin().lock(), std::io::stdout().lock())
                .map_err(|e| e.to_string())?;
            Ok(ExitCode::SUCCESS)
        }
        _ => Err(USAGE.to_string()),
    }
}

fn print_diagnostics(diagnostics: &[sb_model::Diagnostic]) {
    for d in diagnostics {
        let level = match d.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        eprintln!("{level}[{}] {}: {}", d.code, d.path, d.message);
    }
}

fn exit_code(diagnostics: &[sb_model::Diagnostic]) -> ExitCode {
    if has_errors(diagnostics) {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn projects(query: &str) -> Result<ExitCode, String> {
    for p in sb_app::store::search(query)? {
        println!(
            "{}\t{}\tv{}\t{}\t{}",
            p.id,
            p.name,
            p.version.map_or("-".into(), |v| v.to_string()),
            p.updated_at,
            p.source
        );
    }
    Ok(ExitCode::SUCCESS)
}

fn pull(id: &str, version: Option<u32>) -> Result<ExitCode, String> {
    let pulled = sb_app::store::pull(id, version, false)?;
    println!("{}", pulled.path.display());
    Ok(ExitCode::SUCCESS)
}
