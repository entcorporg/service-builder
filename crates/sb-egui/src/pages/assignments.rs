//! Editing a value in place: where it comes from (a field, a literal, Rust code), and one row per field of a message.
//! PORT TARGET: `sb-gtk/src/uis/assignments.rs` (owner: steps agent).
//!
//! egui contract (replaces GTK's `apply` / `Update` callbacks): the editors return the change, the caller applies
//! it with `app.edit`, locating the list in the project itself.

use eframe::egui::{self, Ui};
use sb_app::scope::Var;
use sb_model::{Assignment, Expr, Field, FieldType};

use crate::app::App;

/// A change to a list of assignments, returned by [`assignments`]; apply it inside `app.edit`.
pub type Change = Box<dyn FnOnce(&mut Vec<Assignment>)>;

/// The type of a field as the scope writes it (`string`, `liste de Item`, `optionnel int64`).
pub fn type_label(f: &Field) -> String {
    let base = match &f.ty {
        FieldType::Message { name } => name.clone(),
        other => super::editors::type_name(other),
    };
    format!(
        "{}{}{base}",
        if f.repeated { "liste de " } else { "" },
        if f.optional && !f.repeated && !f.ty.is_message() {
            "optionnel "
        } else {
            ""
        }
    )
}

#[derive(Clone, Copy, PartialEq)]
enum Literal {
    /// No type to follow: numbers and booleans are recognised, the rest is text.
    Free,
    Text,
    Bool,
    Int,
    Float,
    List,
    None,
}

fn literal_kind(f: &Field) -> Literal {
    if f.ty.is_message() {
        return Literal::None;
    }
    if f.repeated {
        return if matches!(f.ty, FieldType::Bytes | FieldType::Timestamp) {
            Literal::None
        } else {
            Literal::List
        };
    }
    match f.ty {
        FieldType::String | FieldType::Uuid => Literal::Text,
        FieldType::Bool => Literal::Bool,
        FieldType::Int32 | FieldType::Int64 => Literal::Int,
        FieldType::Float | FieldType::Double => Literal::Float,
        _ => Literal::None,
    }
}

/// Whether a value of type `have` can fill a field of type `want`: the same, text for text, or a plain value for an optional one.
fn fits(have: &str, want: &str) -> bool {
    let norm = |s: &str| s.replace("uuid", "string");
    let (h, w) = (norm(have), norm(want));
    h == w || w.strip_prefix("optionnel ") == Some(h.as_str())
}

const MODES: [&str; 4] = ["—", "Champ", "Valeur", "Rust"];

fn mode_of(e: &Option<Expr>) -> &'static str {
    match e {
        None => MODES[0],
        Some(Expr::Path { .. }) => MODES[1],
        Some(Expr::Literal { .. }) => MODES[2],
        Some(Expr::Raw { .. }) => MODES[3],
        Some(Expr::Now) => MODES[2],
    }
}

/// One value: a source (—, Champ, Valeur, Rust) and what goes with it. Returns `Some(new value)` when the user
/// changed it (`Some(None)`: unset, when `allow_unset`).
pub fn expr_editor(
    app: &mut App,
    ui: &mut Ui,
    id: impl egui::AsIdSalt,
    target: Option<&Field>,
    allow_unset: bool,
    current: Option<&Expr>,
    vars: &[Var],
) -> Option<Option<Expr>> {
    let _ = (
        app,
        ui.make_persistent_id(id),
        target,
        allow_unset,
        current,
        vars,
    );
    super::common::empty(ui, "À porter : assignments.rs (expr_editor)");
    None
}

/// One row per field of `message`. Returns the change to apply to the list of assignments, if any.
pub fn assignments(
    app: &mut App,
    ui: &mut Ui,
    id: impl egui::AsIdSalt,
    message: &str,
    list: &[Assignment],
    vars: &[Var],
) -> Option<Change> {
    let _ = (app, ui.make_persistent_id(id), message, list, vars);
    super::common::empty(ui, "À porter : assignments.rs (assignments)");
    None
}
