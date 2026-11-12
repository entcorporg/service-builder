use std::path::PathBuf;
use std::sync::OnceLock;

use gtk::glib;
use gtk::prelude::*;

use super::build::build;

const APP_ID: &str = "dev.servicebuilder.app";

/// The project to open at start, if one was given on the command line.
static OPEN: OnceLock<PathBuf> = OnceLock::new();

pub fn opening() -> Option<&'static PathBuf> {
    OPEN.get()
}

pub fn run(open: Option<PathBuf>) -> glib::ExitCode {
    if let Some(path) = open {
        let _ = OPEN.set(path);
    }
    // Screenshots are taken by a window of their own, even when the app is already open.
    let flags = if std::env::var_os("SB_SCREENSHOT").is_some() {
        gtk::gio::ApplicationFlags::NON_UNIQUE
    } else {
        gtk::gio::ApplicationFlags::empty()
    };
    let app = gtk::Application::builder()
        .application_id(APP_ID)
        .flags(flags)
        .build();
    app.connect_activate(build);
    // Arguments belong to the app (`mcp <project>`), not to GTK.
    app.run_with_args::<&str>(&[])
}
