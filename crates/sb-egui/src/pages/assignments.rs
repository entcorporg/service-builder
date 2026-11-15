//! Editing a value in place: where it comes from (a field, a literal, Rust code), and one row per field of a message.
//!
//! egui contract (replaces GTK's `apply` / `Update` callbacks): the editors return the change, the caller applies
//! it with `app.edit`, locating the list in the project itself.

use eframe::egui::{self, RichText, Ui};
use sb_app::scope::Var;
use sb_model::{Assignment, Expr, Field, FieldType, Func, Project};
use serde_json::Value;

use super::common::{commit_entry, icon_btn, link};
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
    // `liste de *`: any list.
    if want == "liste de *" {
        return have.starts_with("liste de ");
    }
    let norm = |s: &str| s.replace("uuid", "string");
    let (h, w) = (norm(have), norm(want));
    h == w || w.strip_prefix("optionnel ") == Some(h.as_str())
}

const MODES: [&str; 6] = ["—", "Champ", "Valeur", "Texte", "Opération", "Rust"];
/// One more source, for a timestamp column of an SQL step only: the clock of the database (`now()`).
const CLOCK: &str = "Maintenant";

fn mode_of(e: &Option<Expr>) -> &'static str {
    match e {
        None => MODES[0],
        Some(Expr::Path { .. }) => MODES[1],
        Some(Expr::Literal { .. }) => MODES[2],
        Some(Expr::Raw { .. }) => MODES[5],
        Some(Expr::Template { .. }) => MODES[3],
        Some(Expr::Call { .. }) => MODES[4],
        Some(Expr::Now) => CLOCK,
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
    app: &mut App,
    ui: &mut Ui,
    id: impl egui::AsIdSalt,
    target: Option<&Field>,
    allow_unset: bool,
    current: Option<&Expr>,
    vars: &[Var],
) -> Option<Option<Expr>> {
    expr_editor_with(app, ui, id, target, allow_unset, current, vars, false)
}

/// The value written to, or compared with, a column in an SQL step: a timestamp column (`clock`) can also take the
/// clock of the database, which exists nowhere else.
#[allow(clippy::too_many_arguments)]
pub fn sql_expr_editor(
    app: &mut App,
    ui: &mut Ui,
    id: impl egui::AsIdSalt,
    target: Option<&Field>,
    allow_unset: bool,
    current: Option<&Expr>,
    vars: &[Var],
    clock: bool,
) -> Option<Option<Expr>> {
    expr_editor_with(app, ui, id, target, allow_unset, current, vars, clock)
}

#[allow(clippy::too_many_arguments)]
fn expr_editor_with(
    _app: &mut App,
    ui: &mut Ui,
    id: impl egui::AsIdSalt,
    target: Option<&Field>,
    allow_unset: bool,
    current: Option<&Expr>,
    vars: &[Var],
    clock: bool,
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
    // A clock already chosen stays in the list, even where it is refused: the user sees what is there.
    let clock = clock || matches!(current, Some(Expr::Now));
    let modes: Vec<String> = MODES
        .iter()
        .filter(|m| allow_unset || **m != "—")
        .chain(clock.then_some(&CLOCK))
        .map(|s| (*s).to_string())
        .collect();
    let first_path = options.first().map(|v| v.path.clone()).unwrap_or_default();
    let mut out: Option<Option<Expr>> = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        if let Some(m) = pick(ui, id.with("mode"), mode_of(&current), &modes, 90.0) {
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
                "Texte" => Some(Expr::Template {
                    template: String::new(),
                }),
                "Opération" => Some(match kind {
                    Literal::Int | Literal::Float => Expr::Call {
                        func: Func::Add,
                        args: vec![
                            Expr::Path {
                                path: String::new(),
                            },
                            Expr::Literal {
                                value: Value::from(1),
                            },
                        ],
                    },
                    _ => Expr::Call {
                        func: Func::Concat,
                        args: vec![
                            Expr::Literal {
                                value: Value::String(String::new()),
                            },
                            Expr::Path {
                                path: String::new(),
                            },
                        ],
                    },
                }),
                CLOCK => Some(Expr::Now),
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
            Some(Expr::Template { template }) => {
                if let Some(v) = commit_entry(
                    ui,
                    id.with("template"),
                    template,
                    false,
                    "Commande {input.id} reçue (les valeurs entre accolades)",
                    true,
                ) {
                    out = Some(Some(Expr::Template { template: v }));
                }
            }
            Some(Expr::Call { func, args }) => {
                // The arguments are laid out under the picker, one editor each: the same editor, recursively.
                ui.vertical(|ui| {
                    if let Some(e) = call_editor(_app, ui, id, *func, args, vars) {
                        out = Some(Some(e));
                    }
                });
            }
            None => {}
        }
    });
    out
}

/// The operations, as a person reads them.
fn func_label(f: Func) -> &'static str {
    match f {
        Func::Concat => "Coller des textes",
        Func::Lower => "Minuscules",
        Func::Upper => "Majuscules",
        Func::Trim => "Retirer les espaces autour",
        Func::SanitizeHtml => "Nettoyer un HTML (retirer scripts et attributs dangereux)",
        Func::Length => "Nombre de caractères",
        Func::Contains => "Contient (oui/non)",
        Func::StartsWith => "Commence par (oui/non)",
        Func::EndsWith => "Finit par (oui/non)",
        Func::Replace => "Remplacer dans un texte",
        Func::Split => "Découper un texte en liste",
        Func::Join => "Assembler une liste en texte",
        Func::Add => "Addition",
        Func::Sub => "Soustraction",
        Func::Mul => "Multiplication",
        Func::Div => "Division",
        Func::Rem => "Reste de la division",
        Func::Min => "Le plus petit",
        Func::Max => "Le plus grand",
        Func::Round => "Arrondir",
        Func::Floor => "Arrondir vers le bas",
        Func::Ceil => "Arrondir vers le haut",
        Func::ToInt32 => "En entier 32 bits",
        Func::ToInt64 => "En entier 64 bits",
        Func::ToDouble => "En nombre décimal",
        Func::ToText => "En texte",
        Func::ParseInt64 => "Lire un entier dans un texte",
        Func::ParseDouble => "Lire un décimal dans un texte",
        Func::Default => "Valeur par défaut si absente",
        Func::NewUuid => "Nouvel identifiant (UUID)",
        Func::NewUuidV7 => "Nouvel identifiant triable (UUID v7)",
        Func::ShortId => "Nombre en identifiant court (UkLWZg)",
        Func::FromShortId => "Identifiant court en nombre",
        Func::ListLength => "Nombre d’éléments d’une liste",
    }
}

/// The name of argument `i`, and the field that types what can fill it (`None`: any plain value).
fn arg_spec(f: Func, i: usize) -> (String, Option<Field>) {
    let text = || Some(Field::new("", FieldType::String));
    let list = |ty: FieldType| {
        let mut f = Field::new("", ty);
        f.repeated = true;
        Some(f)
    };
    match f {
        Func::Concat => (format!("Texte {}", i + 1), text()),
        Func::Lower
        | Func::Upper
        | Func::Trim
        | Func::SanitizeHtml
        | Func::Length
        | Func::ParseInt64
        | Func::ParseDouble => ("Texte".into(), text()),
        Func::Contains => (["Texte", "Contient"][i.min(1)].into(), text()),
        Func::StartsWith => (["Texte", "Commence par"][i.min(1)].into(), text()),
        Func::EndsWith => (["Texte", "Finit par"][i.min(1)].into(), text()),
        Func::Replace => (["Texte", "Remplacer", "Par"][i.min(2)].into(), text()),
        Func::Split => (["Texte", "Séparateur"][i.min(1)].into(), text()),
        Func::Join => match i {
            0 => ("Liste de textes".into(), list(FieldType::String)),
            _ => ("Séparateur".into(), text()),
        },
        Func::Div | Func::Rem => (["Dividende", "Diviseur"][i.min(1)].into(), None),
        Func::Add | Func::Sub | Func::Mul | Func::Min | Func::Max => {
            (["Premier nombre", "Second nombre"][i.min(1)].into(), None)
        }
        Func::Round | Func::Floor | Func::Ceil | Func::ToInt32 | Func::ToInt64 | Func::ToDouble => {
            ("Nombre".into(), None)
        }
        Func::ToText => ("Valeur".into(), None),
        Func::Default => (
            ["Valeur (peut être absente)", "Sinon"][i.min(1)].into(),
            None,
        ),
        Func::NewUuid | Func::NewUuidV7 => (String::new(), None),
        Func::ShortId => (
            "Nombre (entier 64 bits, 0 ou plus)".into(),
            Some(Field::new("", FieldType::Int64)),
        ),
        Func::FromShortId => ("Identifiant court".into(), text()),
        Func::ListLength => (
            "Liste".into(),
            list(FieldType::Message { name: "*".into() }),
        ),
    }
}

/// An operation: which one, then one value per argument. Returns the new expression when something changed.
fn call_editor(
    app: &mut App,
    ui: &mut Ui,
    id: egui::Id,
    func: Func,
    args: &[Expr],
    vars: &[Var],
) -> Option<Expr> {
    let mut out: Option<Expr> = None;
    let names: Vec<String> = Func::ALL
        .iter()
        .map(|f| func_label(*f).to_string())
        .collect();
    let width = ui.available_width();
    if let Some(v) = pick(ui, id.with("func"), func_label(func), &names, width)
        && let Some(f) = Func::ALL.iter().find(|f| func_label(**f) == v)
    {
        // Keep the values already given where the new operation still has a place for them.
        let (min, max) = f.arity();
        let mut next: Vec<Expr> = args
            .iter()
            .take(max.unwrap_or(args.len()))
            .cloned()
            .collect();
        while next.len() < min {
            next.push(Expr::Path {
                path: String::new(),
            });
        }
        return Some(Expr::Call {
            func: *f,
            args: next,
        });
    }
    let (min, max) = func.arity();
    for (i, a) in args.iter().enumerate() {
        let (name, target) = arg_spec(func, i);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if max.is_none() && args.len() > min && icon_btn(ui, "trash", "Retirer", true).clicked()
            {
                let mut list = args.to_vec();
                list.remove(i);
                out = Some(Expr::Call { func, args: list });
            }
            ui.label(RichText::new(&name).size(11.0).color(FAINT));
        });
        if let Some(Some(e)) = expr_editor_with(
            app,
            ui,
            id.with(("arg", i)),
            target.as_ref(),
            false,
            Some(a),
            vars,
            false,
        ) {
            let mut list = args.to_vec();
            list[i] = e;
            out = Some(Expr::Call { func, args: list });
        }
    }
    if max.is_none() && link(ui, "+ Ajouter un texte").clicked() {
        let mut list = args.to_vec();
        list.push(Expr::Literal {
            value: Value::String(String::new()),
        });
        out = Some(Expr::Call { func, args: list });
    }
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
