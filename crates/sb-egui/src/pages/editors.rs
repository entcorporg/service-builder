//! The forms of the services and topics (and the messages), and the list pages.
//! PORT TARGET: `sb-gtk/src/uis/editors.rs` (owner: inspector agent). `SCALARS`, `type_name`, `delete_*` are ported.

use sb_model::FieldType;

use crate::app::App;

pub const SCALARS: [(&str, FieldType); 9] = [
    ("string", FieldType::String),
    ("bool", FieldType::Bool),
    ("int32", FieldType::Int32),
    ("int64", FieldType::Int64),
    ("float", FieldType::Float),
    ("double", FieldType::Double),
    ("bytes", FieldType::Bytes),
    ("timestamp", FieldType::Timestamp),
    ("uuid", FieldType::Uuid),
];

pub fn type_name(ty: &FieldType) -> String {
    match ty {
        FieldType::Message { name } => name.clone(),
        other => SCALARS
            .iter()
            .find(|(_, t)| t == other)
            .map_or_else(|| format!("{other:?}"), |(n, _)| (*n).to_string()),
    }
}

/// Removes the service, and its name from the producers of the topics.
pub fn delete_service(app: &mut App, index: usize) {
    app.edit(move |p| {
        if index >= p.services.len() {
            return;
        }
        let name = p.services.remove(index).name;
        for t in &mut p.topics {
            t.producers.retain(|s| *s != name);
        }
    })
}

pub fn delete_topic(app: &mut App, index: usize) {
    app.edit(move |p| {
        if index < p.topics.len() {
            p.topics.remove(index);
        }
    })
}

/// The modal form of a service (new when `index` is `None`).
pub fn service_form(app: &mut App, index: Option<usize>) {
    let _ = (app, index);
}

/// The modal form of a topic (new when `index` is `None`).
pub fn topic_form(app: &mut App, index: Option<usize>) {
    let _ = (app, index);
}
