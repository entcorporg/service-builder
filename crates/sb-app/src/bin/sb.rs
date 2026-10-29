//! `sb`: headless access to Service Builder, for CI and scripting.
//!
//! ```text
//! sb sample                          print the demo project as JSON
//! sb validate <project.json>         print diagnostics, exit 1 if there are errors
//! sb generate <project.json> <dir>   write the generated Cargo workspace into <dir>
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
