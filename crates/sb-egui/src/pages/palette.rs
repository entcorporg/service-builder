//! The left column: tiles to add a service or a topic, then the structure tree of the project.
//! PORT TARGET: `sb-gtk/src/uis/palette.rs` (owner: canvas agent).

use eframe::egui::Ui;
use sb_model::ServiceKind;

use crate::app::App;

/// Adds a service of this kind with the next free ports, and selects it.
pub fn add_service(app: &mut App, kind: ServiceKind) {
    let _ = (app, kind);
}

/// Adds a topic and selects it.
pub fn add_topic(app: &mut App) {
    let _ = app;
}

/// The tiles that add a service or a topic.
pub fn tiles(app: &mut App, ui: &mut Ui) {
    let _ = app;
    super::common::empty(ui, "À porter : palette.rs (tiles)");
}

/// The structure of the project (services, their routes…); also the left column of the API page.
pub fn tree(app: &mut App, ui: &mut Ui) {
    let _ = app;
    super::common::empty(ui, "À porter : palette.rs (tree)");
}
