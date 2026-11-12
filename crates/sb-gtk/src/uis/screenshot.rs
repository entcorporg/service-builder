//! Development aid: `SB_SCREENSHOT=<dir> service-builder` writes one PNG per page of the window, then quits.
//! The window is painted by GTK itself, so no screenshot tool is needed (and none is asked of the desktop).

use std::path::PathBuf;
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;

/// Dumps the window as it is now.
fn dump(window: &gtk::ApplicationWindow, path: &std::path::Path) -> Result<(), String> {
    let (w, h) = (window.width(), window.height());
    let paintable = gtk::WidgetPaintable::new(Some(window));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, f64::from(w), f64::from(h));
    let node = snapshot.to_node().ok_or("nothing to paint")?;
    let renderer = window
        .native()
        .and_then(|n| n.renderer())
        .ok_or("the window has no renderer yet")?;
    renderer
        .render_texture(&node, None)
        .save_to_png(path)
        .map_err(|e| e.to_string())
}

/// With `SB_DUMP=<depth>`: prints the widget tree (type, css classes, size) so a layout can be measured instead of guessed.
fn tree(w: &gtk::Widget, depth: usize, max: usize) {
    if depth > max {
        return;
    }
    if w.is_mapped() {
        eprintln!("{}{} [{}] {}x{}", "  ".repeat(depth), w.type_().name(), w.css_classes().join(" "), w.width(), w.height());
    }
    let mut child = w.first_child();
    while let Some(c) = child {
        tree(&c, depth + 1, max);
        child = c.next_sibling();
    }
}

/// Called once the window is up; does nothing unless `SB_SCREENSHOT` is set.
pub fn schedule(window: &gtk::ApplicationWindow, stack: &gtk::Stack) {
    let Some(dir) = std::env::var_os("SB_SCREENSHOT").map(PathBuf::from) else {
        return;
    };
    let (window, stack) = (window.clone(), stack.clone());
    glib::spawn_future_local(async move {
        let _ = std::fs::create_dir_all(&dir);
        // `SB_SIZE=1480x920`: compare at a given size even when the window manager maximizes the window.
        if let Some((w, h)) = std::env::var("SB_SIZE").ok().and_then(|s| {
            let (w, h) = s.split_once('x')?;
            Some((w.parse::<i32>().ok()?, h.parse::<i32>().ok()?))
        }) {
            window.unmaximize();
            window.set_default_size(w, h);
            window.set_resizable(false);
            glib::timeout_future(Duration::from_millis(500)).await;
        }
        // The analysis of the sample runs in the background: give it time before the first page.
        glib::timeout_future(Duration::from_millis(2500)).await;
        let pages: Vec<String> = stack
            .pages()
            .iter::<gtk::StackPage>()
            .filter_map(Result::ok)
            .filter_map(|p| p.name().map(|n| n.to_string()))
            .collect();
        if let Some(max) = std::env::var("SB_DUMP").ok().and_then(|v| v.parse().ok()) {
            tree(window.upcast_ref(), 0, max);
        }
        for (i, name) in pages.iter().enumerate() {
            stack.set_visible_child_name(name);
            glib::timeout_future(Duration::from_millis(700)).await;
            let path = dir.join(format!("new-{}-{name}.png", i + 1));
            match dump(&window, &path) {
                Ok(()) => eprintln!("wrote {}", path.display()),
                Err(e) => eprintln!("cannot write {}: {e}", path.display()),
            }
        }
        if let Some(app) = window.application() {
            app.quit();
        }
    });
}
