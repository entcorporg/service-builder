//! Editing a value in place: where it comes from (a field, a literal, Rust code), and one row per field of a message.

use std::rc::Rc;

use gtk::prelude::*;
use sb_app::scope::Var;
use sb_model::{Assignment, Expr, Field, FieldType, Project};
use serde_json::Value;

use super::icons::{self, icon};
use super::inspector::{choice, commit_entry, icon_button, link};
use super::ui::Shared;

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

/// One value: a source (—, Champ, Valeur, Rust) and what goes with it.
pub fn expr_editor(
    target: Option<&Field>,
    allow_unset: bool,
    current: Option<Expr>,
    vars: &[Var],
    apply: Rc<dyn Fn(Option<Expr>)>,
) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.add_css_class("expr");
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
    let current = if current.is_none() && !allow_unset {
        Some(Expr::Path {
            path: String::new(),
        })
    } else {
        current
    };

    let modes: Vec<String> = MODES
        .iter()
        .filter(|m| allow_unset || **m != "—")
        .map(|s| (*s).to_string())
        .collect();
    let first_path = options.first().map(|v| v.path.clone()).unwrap_or_default();
    let mode = choice(&modes, mode_of(&current), {
        let apply = apply.clone();
        move |m| {
            apply(match m.as_str() {
                "Champ" => Some(Expr::Path {
                    path: first_path.clone(),
                }),
                "Valeur" => Some(Expr::Literal {
                    value: match kind {
                        Literal::List => Value::Array(vec![]),
                        Literal::Bool => Value::Bool(false),
                        Literal::Free => Value::String(String::new()),
                        Literal::Int | Literal::Float => Value::from(0),
                        _ => Value::String(String::new()),
                    },
                }),
                "Rust" => Some(Expr::Raw {
                    code: String::new(),
                }),
                _ => None,
            })
        }
    });
    mode.set_valign(gtk::Align::Center);
    row.append(&mode);

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
            let paths: Vec<String> = options.iter().map(|v| v.path.clone()).collect();
            let pick = choice(&names, &shown, {
                let apply = apply.clone();
                move |v| {
                    if let Some(p) = paths.iter().find(|p| v.starts_with(&format!("{p} · "))) {
                        apply(Some(Expr::Path { path: p.clone() }));
                    }
                }
            });
            pick.set_hexpand(true);
            row.append(&pick);
        }
        Some(Expr::Raw { code }) => {
            let e = commit_entry(code, true, "expression Rust", true, {
                let apply = apply.clone();
                move |v| apply(Some(Expr::Raw { code: v }))
            });
            e.set_hexpand(true);
            row.append(&e);
        }
        Some(Expr::Literal { value }) => row.append(&literal(kind, value, apply.clone())),
        Some(Expr::Now) => {
            let l = gtk::Label::new(Some("l’heure de la base (now())"));
            l.add_css_class("text-muted");
            row.append(&l);
        }
        None => {}
    }
    row
}

fn literal(kind: Literal, value: &Value, apply: Rc<dyn Fn(Option<Expr>)>) -> gtk::Widget {
    let set = move |v: Value| apply(Some(Expr::Literal { value: v }));
    match kind {
        Literal::None => {
            let l = gtk::Label::new(Some("Type non éditable ici, utilisez « Rust »"));
            l.add_css_class("text-muted");
            l.upcast()
        }
        Literal::Bool => {
            let d = choice(
                &["vrai".to_string(), "faux".to_string()],
                if *value == Value::Bool(true) {
                    "vrai"
                } else {
                    "faux"
                },
                move |v| set(Value::Bool(v == "vrai")),
            );
            d.set_hexpand(true);
            d.upcast()
        }
        Literal::Int | Literal::Float => {
            let s = gtk::SpinButton::with_range(-1e12, 1e12, 1.0);
            s.set_digits(if kind == Literal::Float { 4 } else { 0 });
            s.set_value(value.as_f64().unwrap_or(0.0));
            s.set_hexpand(true);
            s.connect_value_changed(move |s| {
                set(if kind == Literal::Int {
                    Value::from(s.value() as i64)
                } else {
                    Value::from(s.value())
                })
            });
            s.upcast()
        }
        Literal::List => {
            let empty = Value::Array(vec![]);
            let text = serde_json::to_string(if value.is_array() { value } else { &empty })
                .unwrap_or_default();
            let e = commit_entry(&text, true, "[\"a\", \"b\"]", true, move |v| {
                if let Ok(a @ Value::Array(_)) = serde_json::from_str::<Value>(&v) {
                    set(a);
                }
            });
            e.set_hexpand(true);
            e.upcast()
        }
        Literal::Free => {
            let text = match value {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            let e = commit_entry(
                &text,
                false,
                "texte, nombre ou true/false",
                true,
                move |v| {
                    set(match v.as_str() {
                        "true" => Value::Bool(true),
                        "false" => Value::Bool(false),
                        t if !t.trim().is_empty() && t.parse::<f64>().is_ok() => {
                            serde_json::from_str(t).unwrap_or(Value::String(v.clone()))
                        }
                        _ => Value::String(v),
                    })
                },
            );
            e.set_hexpand(true);
            e.upcast()
        }
        Literal::Text => {
            let text = match value {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            let e = commit_entry(&text, false, "texte", true, move |v| set(Value::String(v)));
            e.set_hexpand(true);
            e.upcast()
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

/// How a change reaches the list of assignments in the project.
pub type Update = Rc<dyn Fn(Box<dyn FnOnce(&mut Vec<Assignment>)>)>;

/// One row per field of `message`. `update` applies a change to the list of assignments in the project.
pub fn assignments(
    ui: &Shared,
    message: &str,
    list: &[Assignment],
    vars: &[Var],
    update: Update,
) -> gtk::Box {
    let host = gtk::Box::new(gtk::Orientation::Vertical, 7);
    let project = ui.state.borrow().project.clone();
    let Some(m) = project.messages.iter().find(|m| m.name == message) else {
        let l = gtk::Label::builder()
            .label("Choisissez d’abord un message valide.")
            .xalign(0.0)
            .build();
        l.add_css_class("empty");
        host.append(&l);
        return host;
    };
    for f in &m.fields {
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let name = gtk::Box::new(gtk::Orientation::Vertical, 0);
        name.set_width_request(112);
        name.set_valign(gtk::Align::Center);
        let n = gtk::Label::builder()
            .label(&f.name)
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        n.add_css_class("font-mono");
        name.append(&n);
        let t = gtk::Label::builder()
            .label(type_label(f))
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        t.add_css_class("assignment-type");
        name.append(&t);
        line.append(&name);
        let current = list
            .iter()
            .find(|a| a.field == f.name)
            .map(|a| a.value.clone());
        let field = f.name.clone();
        let apply: Rc<dyn Fn(Option<Expr>)> = {
            let update = update.clone();
            Rc::new(move |value| {
                let field = field.clone();
                update(Box::new(move |list| {
                    let i = list.iter().position(|a| a.field == field);
                    match (value, i) {
                        (None, Some(i)) => {
                            list.remove(i);
                        }
                        (Some(v), Some(i)) => list[i].value = v,
                        (Some(v), None) => list.push(Assignment { field, value: v }),
                        (None, None) => {}
                    }
                }))
            })
        };
        let editor = expr_editor(Some(f), true, current, vars, apply);
        editor.set_hexpand(true);
        line.append(&editor);
        host.append(&line);
    }
    // Assignments to a field the message no longer has.
    for a in list
        .iter()
        .filter(|a| !m.fields.iter().any(|f| f.name == a.field))
    {
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let l = gtk::Label::builder()
            .label(format!("{} · champ inconnu", a.field))
            .xalign(0.0)
            .hexpand(true)
            .build();
        l.add_css_class("font-mono");
        line.append(&l);
        let (update, field) = (update.clone(), a.field.clone());
        line.append(&icon_button(
            "trash",
            &format!("Retirer {}", a.field),
            true,
            move || {
                let field = field.clone();
                update(Box::new(move |list| list.retain(|x| x.field != field)));
            },
        ));
        host.append(&line);
    }
    if !m.fields.is_empty() {
        let (update, vars_owned, project, message) = (
            update.clone(),
            vars.to_vec(),
            project.clone(),
            message.to_string(),
        );
        let b = link(
            "Remplir automatiquement les champs de même nom",
            move || {
                let found = auto_map(&project, &message, &vars_owned);
                update(Box::new(move |list| {
                    for a in found {
                        if !list.iter().any(|x| x.field == a.field) {
                            list.push(a);
                        }
                    }
                }));
            },
        );
        let wrap = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        wrap.append(&icon("sparkles", 13, icons::SERVICE));
        wrap.append(&b);
        host.append(&wrap);
    }
    host
}
