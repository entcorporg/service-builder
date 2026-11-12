//! The Flows page: every handler of every service, with its steps.
//! PORT TARGET: `sb-gtk/src/uis/flows.rs` (owner: steps agent).

use eframe::egui::Ui;

use crate::app::App;

#[derive(Default)]
pub struct FlowsState {}

pub fn page(app: &mut App, ui: &mut Ui) {
    let _ = app;
    super::common::empty(ui, "À porter : flows.rs");
}
