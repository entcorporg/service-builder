//! The Tables page: the tables of each service, their columns and indexes.
//! PORT TARGET: `sb-gtk/src/uis/tablesview.rs`.

use eframe::egui::Ui;

use crate::app::App;

#[derive(Default)]
pub struct TablesState {}

pub fn page(app: &mut App, ui: &mut Ui) {
    let _ = app;
    super::common::empty(ui, "À porter : tablesview.rs");
}
