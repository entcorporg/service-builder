//! Service Builder desktop app (egui).

fn main() -> std::process::ExitCode {
    // `service-builder mcp <project>` is the MCP server an AI client starts over stdio: no window.
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Without a project it works on the one the app opened last, or waits for `load_project`.
    if let Some((mode, rest)) = args.split_first()
        && mode == "mcp"
        && rest.len() <= 1
    {
        let stdio = (std::io::stdin().lock(), std::io::stdout().lock());
        let project = rest.first().map(std::path::Path::new);
        return match sb_app::mcp::serve(project, stdio.0, stdio.1) {
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
    let icon =
        eframe::icon_data::from_png_bytes(include_bytes!("../data/service-builder.png")).ok();
    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_title(sb_egui::app::TITLE)
        .with_app_id("dev.servicebuilder.app")
        .with_inner_size([1480.0, 920.0])
        .with_min_inner_size([960.0, 600.0]);
    if let Some(icon) = icon {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    match eframe::run_native(
        sb_egui::app::TITLE,
        options,
        Box::new(move |cc| Ok(Box::new(sb_egui::App::new(cc, open)))),
    ) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::ExitCode::from(1)
        }
    }
}
