//! Service Builder desktop app: GTK 4 window over `sb-app`.

mod uis;
mod updater;

fn main() -> std::process::ExitCode {
    // `service-builder mcp <project>` is the MCP server an AI client starts over stdio: no window.
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let [mode, project] = args.as_slice()
        && mode == "mcp"
    {
        let stdio = (std::io::stdin().lock(), std::io::stdout().lock());
        return match sb_app::mcp::serve(std::path::Path::new(project), stdio.0, stdio.1) {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                std::process::ExitCode::from(2)
            }
        };
    }
    // `service-builder <project>` opens that project at start.
    let open = match args.as_slice() {
        [path] => Some(std::path::PathBuf::from(path)),
        _ => None,
    };
    uis::run::run(open).into()
}
