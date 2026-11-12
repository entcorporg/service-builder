//! The Architecture page: the palette on the left, services and topics as nodes in the middle, the inspector on
//! the right. PORT TARGET: `sb-gtk/src/uis/canvas.rs` (owner: canvas agent).

use eframe::egui::{self, Ui};

use crate::app::App;
use crate::theme::*;

#[derive(Clone, PartialEq, Debug)]
pub enum Sel {
    Service(String),
    Topic(String),
    /// The schemas (events, proto, API) in the inspector, the one of this name open.
    Messages(String),
}

#[derive(Default)]
pub struct CanvasState {
    pub selected: Option<Sel>,
}

/// Selects a service or a topic (or nothing), in the canvas, the tree and the inspector.
pub fn select(app: &mut App, sel: Option<Sel>) {
    app.canvas.selected = sel;
}

pub fn page(app: &mut App, ui: &mut Ui) {
    egui::Panel::left("palette")
        .exact_size(228.0)
        .resizable(false)
        .frame(
            egui::Frame::new()
                .fill(PANEL)
                .inner_margin(egui::Margin::same(12)),
        )
        .show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                super::palette::tiles(app, ui);
                super::palette::tree(app, ui);
            });
        });
    egui::Panel::right("inspector")
        .exact_size(410.0)
        .resizable(false)
        .frame(
            egui::Frame::new()
                .fill(PANEL)
                .inner_margin(egui::Margin::same(16)),
        )
        .show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| super::inspector::show(app, ui));
        });
    egui::CentralPanel::no_frame().show(ui, |ui| {
        super::common::empty(ui, "À porter : canvas.rs (le graphe)");
    });
}
