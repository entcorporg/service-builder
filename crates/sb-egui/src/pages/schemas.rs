//! The messages (events, proto, API) in the inspector, with their fields.
//! PORT TARGET: `sb-gtk/src/uis/schemas.rs` (owner: inspector agent).

use eframe::egui::Ui;

use crate::app::App;

/// Selects the message of this name in the inspector (opening its card).
pub fn open_message(app: &mut App, name: &str) {
    app.open.insert(format!("msg:{name}"));
    super::canvas::select(app, Some(super::canvas::Sel::Messages(name.to_string())));
}

/// The schemas in the inspector: every message, its kind and its fields.
pub fn messages(app: &mut App, ui: &mut Ui) {
    let _ = app;
    super::common::empty(ui, "À porter : schemas.rs (messages)");
}
