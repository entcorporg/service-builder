//! Editing a value in place: where it comes from (a field, a literal, Rust code), and one row per field of a message.
//!
//! egui contract (replaces GTK's `apply` / `Update` callbacks): the editors return the change, the caller applies
//! it with `app.edit`, locating the list in the project itself.

use eframe::egui::{self, RichText, Ui};
use sb_app::scope::Var;
use sb_model::{Assignment, Expr, Field, FieldType, Project};
use serde_json::Value;

use super::common::{commit_entry, link};
use crate::app::App;
use crate::icons::icon;
use crate::theme::*;

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

/// A drop-down of `options` (`width` at most, never more than the room left): the text is truncated, so that a long
/// value never widens the column it is in. Returns the option picked when it differs from `current`.
pub fn pick(
    ui: &mut Ui,
    id: impl egui::AsIdSalt,
    current: &str,
    options: &[String],
    width: f32,
) -> Option<String> {
    let mut picked = None;
    egui::ComboBox::from_id_salt(id)
        .selected_text(if current.is_empty() { "—" } else { current })
        .width(width.min(ui.available_width()).max(30.0))
        .truncate()
        .show_ui(ui, |ui| {
            for o in options {
                if ui.selectable_label(o == current, o).clicked() && o != current {
                    picked = Some(o.clone());
                }
            }
        });
    picked
}

/// One value: a source (—, Champ, Valeur, Rust) and what goes with it. Returns `Some(new value)` when the user
/// changed it (`Some(None)`: unset, when `allow_unset`).
pub fn expr_editor(
    _app: &mut App,
    ui: &mut Ui,
    id: impl egui::AsIdSalt,
    target: Option<&Field>,
    allow_unset: bool,
    current: Option<&Expr>,
    vars: &[Var],
) -> Option<Option<Expr>> {
    let id = ui.make_persistent_id(id);
    let scalar = |t: &str| {
        let t = t.strip_prefix("optionnel ").unwrap_or(t);
        super::editors::SCALARS.iter().any(|(n, _)| *n == t)
    };
    let options: Vec<&Var> = match target {
        Some(t) => {
            let want = type_label(t);
            vars.iter().filter(|v| fits(&v.ty, &want)).collect()
        }
        None => vars.iter().filter(|v| scalar(&v.ty)).collect(),
    };
    let kind = target.map_or(Literal::Free, literal_kind);
    // Without "—", an empty value is a field still to choose.
    let current: Option<Expr> = match current {
        None if !allow_unset => Some(Expr::Path {
            path: String::new(),
        }),
        c => c.cloned(),
    };
    let modes: Vec<String> = MODES
        .iter()
        .filter(|m| allow_unset || **m != "—")
        .map(|s| (*s).to_string())
        .collect();
    let first_path = options.first().map(|v| v.path.clone()).unwrap_or_default();
    let mut out: Option<Option<Expr>> = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        if let Some(m) = pick(ui, id.with("mode"), mode_of(&current), &modes, 78.0) {
            out = Some(match m.as_str() {
                "Champ" => Some(Expr::Path {
                    path: first_path.clone(),
                }),
                "Valeur" => Some(Expr::Literal {
                    value: match kind {
                        Literal::List => Value::Array(vec![]),
                        Literal::Bool => Value::Bool(false),
                        Literal::Int | Literal::Float => Value::from(0),
                        _ => Value::String(String::new()),
                    },
                }),
                "Rust" => Some(Expr::Raw {
                    code: String::new(),
                }),
                _ => None,
            });
        }
        match &current {
            Some(Expr::Path { path }) => {
                let mut names: Vec<String> = options
                    .iter()
                    .map(|v| format!("{} · {}", v.path, v.ty))
                    .collect();
                let shown = options.iter().find(|v| v.path == *path).map_or_else(
                    || {
                        let s = if path.is_empty() {
                            "Choisir…".to_string()
                        } else {
                            format!("{path} (incompatible)")
                        };
                        names.insert(0, s.clone());
                        s
                    },
                    |v| format!("{} · {}", v.path, v.ty),
                );
                let width = ui.available_width();
                if let Some(v) = pick(ui, id.with("path"), &shown, &names, width)
                    && let Some(v) = options
                        .iter()
                        .find(|o| v.starts_with(&format!("{} · ", o.path)))
                {
                    out = Some(Some(Expr::Path {
                        path: v.path.clone(),
                    }));
                }
            }
            Some(Expr::Raw { code }) => {
                if let Some(v) =
                    commit_entry(ui, id.with("raw"), code, true, "expression Rust", true)
                {
                    out = Some(Some(Expr::Raw { code: v }));
                }
            }
            Some(Expr::Literal { value }) => {
                if let Some(v) = literal(ui, id, kind, value) {
                    out = Some(Some(Expr::Literal { value: v }));
                }
            }
            Some(Expr::Now) => {
                ui.label(RichText::new("l’heure de la base (now())").color(MUTED));
            }
            None => {}
        }
    });
    out
}

fn literal(ui: &mut Ui, id: egui::Id, kind: Literal, value: &Value) -> Option<Value> {
    let id = id.with("literal");
    let as_text = |value: &Value| match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    match kind {
        Literal::None => {
            ui.label(RichText::new("Type non éditable ici, utilisez « Rust »").color(MUTED));
            None
        }
        Literal::Bool => {
            let now = if *value == Value::Bool(true) {
                "vrai"
            } else {
                "faux"
            };
            let width = ui.available_width();
            pick(
                ui,
                id,
                now,
                &["vrai".to_string(), "faux".to_string()],
                width,
            )
            .map(|v| Value::Bool(v == "vrai"))
        }
        Literal::Int | Literal::Float => {
            let text = match value.as_f64() {
                Some(f) if kind == Literal::Int => (f as i64).to_string(),
                Some(f) => f.to_string(),
                None => "0".into(),
            };
            let v = commit_entry(ui, id, &text, true, "0", false)?;
            if kind == Literal::Int {
                v.parse::<i64>().ok().map(Value::from)
            } else {
                v.parse::<f64>()
                    .ok()
                    .filter(|f| f.is_finite())
                    .map(Value::from)
            }
        }
        Literal::List => {
            let empty = Value::Array(vec![]);
            let text = serde_json::to_string(if value.is_array() { value } else { &empty })
                .unwrap_or_default();
            let v = commit_entry(ui, id, &text, true, "[\"a\", \"b\"]", true)?;
            match serde_json::from_str::<Value>(&v) {
                Ok(a @ Value::Array(_)) => Some(a),
                _ => None,
            }
        }
        Literal::Free => {
            let v = commit_entry(
                ui,
                id,
                &as_text(value),
                false,
                "texte, nombre ou true/false",
                true,
            )?;
            Some(match v.as_str() {
                "true" => Value::Bool(true),
                "false" => Value::Bool(false),
                t if !t.trim().is_empty() && t.parse::<f64>().is_ok() => {
                    serde_json::from_str(t).unwrap_or(Value::String(v.clone()))
                }
                _ => Value::String(v),
            })
        }
        Literal::Text => {
            commit_entry(ui, id, &as_text(value), false, "texte", true).map(Value::String)
        }
    }
}

/// The paths of the scope that can fill each field of the message, matched by name: the "fill the rest" button.
fn auto_map(p: &Project, message: &str, vars: &[Var]) -> Vec<Assignment> {
    let Some(m) = p.messages.iter().find(|m| m.name == message) else {
        return vec![];
    };
    m.fields
        .iter()
        .filter_map(|f| {
            let want = type_label(f);
            let v = vars.iter().find(|v| {
                v.path.rsplit('.').next() == Some(f.name.as_str())
                    && v.path.contains('.')
                    && fits(&v.ty, &want)
            })?;
            Some(Assignment {
                field: f.name.clone(),
                value: Expr::Path {
                    path: v.path.clone(),
                },
            })
        })
        .collect()
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
    let id = ui.make_persistent_id(id);
    let project = app.project();
    let Some(m) = project.messages.iter().find(|m| m.name == message) else {
        super::common::empty(ui, "Choisissez d’abord un message valide.");
        return None;
    };
    let mut change: Option<Change> = None;
    ui.spacing_mut().item_spacing.y = 7.0;
    for f in &m.fields {
        let current = list.iter().find(|a| a.field == f.name).map(|a| &a.value);
        let mut picked = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.set_width(112.0);
                ui.add(egui::Label::new(RichText::new(&f.name).font(mono(SM))).truncate());
                ui.add(
                    egui::Label::new(RichText::new(type_label(f)).size(XS).color(FAINT)).truncate(),
                );
            });
            picked = expr_editor(app, ui, (id, &f.name), Some(f), true, current, vars);
        });
        if let Some(value) = picked {
            let field = f.name.clone();
            change = Some(Box::new(move |list: &mut Vec<Assignment>| {
                let i = list.iter().position(|a| a.field == field);
                match (value, i) {
                    (None, Some(i)) => {
                        list.remove(i);
                    }
                    (Some(v), Some(i)) => list[i].value = v,
                    (Some(v), None) => list.push(Assignment { field, value: v }),
                    (None, None) => {}
                }
            }));
        }
    }
    // Assignments to a field the message no longer has.
    for a in list
        .iter()
        .filter(|a| !m.fields.iter().any(|f| f.name == a.field))
    {
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{} · champ inconnu", a.field)).font(mono(SM)));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if super::common::icon_btn(ui, "trash", &format!("Retirer {}", a.field), true)
                    .clicked()
                {
                    let field = a.field.clone();
                    change = Some(Box::new(move |list: &mut Vec<Assignment>| {
                        list.retain(|x| x.field != field)
                    }));
                }
            });
        });
    }
    if !m.fields.is_empty() {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.add(icon("sparkles", 13.0, SERVICE));
            if link(ui, "Remplir automatiquement les champs de même nom").clicked() {
                let found = auto_map(&project, message, vars);
                change = Some(Box::new(move |list: &mut Vec<Assignment>| {
                    for a in found {
                        if !list.iter().any(|x| x.field == a.field) {
                            list.push(a);
                        }
                    }
                }));
            }
        });
    }
    change
}
