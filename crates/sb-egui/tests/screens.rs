//! Development aid: renders the window headless (wgpu, software Vulkan is fine) and writes one PNG per page.
//!
//! ```sh
//! SB_SHOTS=/tmp/shots cargo test -p sb-egui --test screens -- --nocapture
//! ```
//!
//! * `SB_SHOTS`: the folder of the PNGs (default `target/shots`).
//! * `SB_PAGES`: the pages to render, comma separated, among `architecture,endpoint,flows,tables,mcp,code`
//!   (default: all).
//! * `SB_SAMPLE=showcase`: the complete sample instead of the orders one. `SB_PROJECT=<file>`: a project file.
//! * `SB_SELECT=<service>`, `<topic.name>` or `msg:<Message>`: selected on the canvas (inspector shown).
//! * `SB_ROUTE=<Service>/<route>`: the route open on the API page.
//! * `SB_OPEN_ALL=1`: every card open. `SB_SIZE=1480x920`: the window size.

use std::path::PathBuf;

use egui_kittest::Harness;
use sb_egui::app::{App, Page};
use sb_egui::pages::canvas::{Sel, select};

#[test]
fn screens() {
    // SAFETY: set before any thread of the app starts.
    unsafe { std::env::set_var("SB_NO_UPDATE", "1") };
    let dir = std::env::var("SB_SHOTS")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/shots"));
    std::fs::create_dir_all(&dir).unwrap();
    let (w, h) = std::env::var("SB_SIZE")
        .ok()
        .and_then(|s| {
            s.split_once('x')
                .map(|(a, b)| (a.parse().unwrap(), b.parse().unwrap()))
        })
        .unwrap_or((1480.0, 920.0));
    let pages =
        std::env::var("SB_PAGES").unwrap_or("architecture,endpoint,flows,tables,mcp,code".into());

    for name in pages.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let page = match name {
            "architecture" => Page::Architecture,
            "endpoint" => Page::Endpoint,
            "flows" => Page::Flows,
            "tables" => Page::Tables,
            "mcp" => Page::Mcp,
            "code" => Page::Code,
            other => panic!("unknown page {other}"),
        };
        let mut harness = Harness::builder()
            .with_size([w, h])
            .wgpu()
            .build_eframe(|cc| {
                let open = std::env::var("SB_PROJECT").ok().map(PathBuf::from);
                let mut app = App::new(cc, open);
                if std::env::var("SB_SAMPLE").as_deref() == Ok("showcase") {
                    app.load_sample("showcase");
                }
                if let Ok(name) = std::env::var("SB_SELECT") {
                    let sel = if let Some(m) = name.strip_prefix("msg:") {
                        app.open.insert(format!("msg:{m}"));
                        Sel::Messages(m.to_string())
                    } else if name.contains('.') {
                        Sel::Topic(name)
                    } else {
                        Sel::Service(name)
                    };
                    select(&mut app, Some(sel));
                }
                if let Ok(route) = std::env::var("SB_ROUTE")
                    && let Some((svc, r)) = route.split_once('/')
                {
                    sb_egui::pages::endpoint::open(&mut app, svc, r);
                }
                app.page = page;
                app
            });
        // Let the analysis come back and the icons load.
        for _ in 0..6 {
            harness.run_steps(2);
            std::thread::sleep(std::time::Duration::from_millis(150));
        }
        harness.state_mut().page = page;
        harness.run_steps(3);
        let image = harness.render().expect("render");
        let path = dir.join(format!("{name}.png"));
        image.save(&path).unwrap();
        eprintln!("wrote {}", path.display());
    }
}
