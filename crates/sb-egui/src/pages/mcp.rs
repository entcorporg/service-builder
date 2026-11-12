//! The MCP page: how to plug an AI client into the project, and the feedback it left.
//! PORT TARGET: `sb-gtk/src/uis/mcp.rs`.

use eframe::egui::Ui;

use crate::app::App;

#[derive(Default)]
pub struct McpState {}

pub fn page(app: &mut App, ui: &mut Ui) {
    let _ = app;
    super::common::empty(ui, "À porter : mcp.rs");
}
