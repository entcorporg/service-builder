//! The right column of the Architecture page: the project, a service or a topic, edited in place.
//! PORT TARGET: `sb-gtk/src/uis/inspector.rs` (owner: inspector agent). Its building blocks are in `common.rs`.

use eframe::egui::Ui;

use crate::app::App;

#[derive(Default)]
pub struct InspectorState {}

/// The inspector of the current selection (`app.canvas.selected`), drawn in the column it is given (410 px).
pub fn show(app: &mut App, ui: &mut Ui) {
    let _ = app;
    super::common::empty(ui, "À porter : inspector.rs");
}
