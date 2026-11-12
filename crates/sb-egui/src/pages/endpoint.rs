//! The API page: one route, configured end to end (request, steps, response).
//! PORT TARGET: `sb-gtk/src/uis/endpoint.rs` (owner: endpoint agent).

use eframe::egui::{self, Ui};

use crate::app::{App, Page};
use crate::theme::*;

#[derive(Default)]
pub struct EndpointState {
    /// The route open: (service, route).
    pub open: Option<(String, String)>,
}

/// Opens the configurator on a route.
pub fn open(app: &mut App, service: &str, route: &str) {
    app.endpoint.open = Some((service.to_string(), route.to_string()));
    app.page = Page::Endpoint;
}

pub fn page(app: &mut App, ui: &mut Ui) {
    egui::Panel::left("endpoint-tree")
        .exact_size(228.0)
        .resizable(false)
        .frame(
            egui::Frame::new()
                .fill(PANEL)
                .inner_margin(egui::Margin::same(12)),
        )
        .show(ui, |ui| {
            crate::widgets::section_title(ui, "Structure");
            egui::ScrollArea::vertical().show(ui, |ui| super::palette::tree(app, ui));
        });
    egui::CentralPanel::no_frame().show(ui, |ui| {
        super::common::empty(ui, "À porter : endpoint.rs");
    });
}
