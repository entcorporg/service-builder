//! `sb`: headless access to Service Builder, for CI and scripting.
//!
//! ```text
//! sb sample                          print the demo project as JSON
//! sb validate <project.json>         print diagnostics, exit 1 if there are errors
//! sb generate <project.json> <dir>   write the generated Cargo workspace into <dir>
//! sb pods <project.json> <dir>       write the workspace with what runs it in containers (Dockerfile, compose, Traefik)
//! sb up <project.json>             compile the services and start them in containers, with their infrastructure
//! sb down <project.json> [--volumes] stop that stack (and delete its data)
//! sb analyze                         project JSON on stdin -> analysis JSON on stdout
//! sb mcp <project.json>              MCP server on stdio, to let an AI client edit the project
//! ```

use std::io::Read;
use std::path::Path;
use std::process::ExitCode;

use sb_model::{Project, Severity, has_errors};

const USAGE: &str = "usage:
  sb sample
  sb validate <project.json>
  sb generate <project.json> <dir>
  sb pods <project.json> <dir>
  sb up <project.json>
  sb down <project.json> [--volumes]
  sb analyze            (project JSON on stdin)
  sb mcp <project.json> (MCP server on stdio)";

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
        ["pods", file, dir] => {
            let project = sb_app::load_project(Path::new(file))?;
            print_diagnostics(&sb_codegen::diagnose(&project));
            let files = sb_app::pods::prepare(&project, Path::new(dir))?;
            println!("wrote {files} files to {dir}");
            println!("start it from that folder with:");
            for command in sb_codegen::pods::manual_commands(&project, "docker") {
                println!("  {command}");
            }
            Ok(ExitCode::SUCCESS)
        }
        ["up", file] => {
            let project = sb_app::load_project(Path::new(file))?;
            let engine = sb_app::pods::Engine::detect()?;
            println!("engine: {}", engine.label());
            let dir = sb_app::pods::stack_dir(&project);
            sb_app::pods::up(
                &engine,
                &project,
                &dir,
                &|l| println!("{l}"),
                &Default::default(),
            )?;
            for link in sb_codegen::pods::links(&project) {
                println!("{}: {}", link.label, link.url);
            }
            Ok(ExitCode::SUCCESS)
        }
        ["down", file, rest @ ..] if matches!(rest, [] | ["--volumes"]) => {
            let project = sb_app::load_project(Path::new(file))?;
            let engine = sb_app::pods::Engine::detect()?;
            let dir = sb_app::pods::stack_dir(&project);
            let log = |l: &str| println!("{l}");
            sb_app::pods::down(
                &engine,
                &project,
                &dir,
                !rest.is_empty(),
                &log,
                &Default::default(),
            )?;
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
        ["mcp", file] => {
            // stdout belongs to the protocol: anything else goes to stderr.
            eprintln!("service-builder MCP server on {file}");
            sb_app::mcp::serve(
                Path::new(file),
                std::io::stdin().lock(),
                std::io::stdout().lock(),
            )
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
