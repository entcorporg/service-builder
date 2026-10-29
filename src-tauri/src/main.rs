// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // `service-builder mcp <project>` is the MCP server an AI client starts over stdio: no window.
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let [mode, project] = args.as_slice()
        && mode == "mcp"
    {
        let stdio = (std::io::stdin().lock(), std::io::stdout().lock());
        if let Err(e) = sb_app::mcp::serve(std::path::Path::new(project), stdio.0, stdio.1) {
            eprintln!("error: {e}");
            std::process::exit(2);
        }
        return;
    }
    service_builder_lib::run()
}
