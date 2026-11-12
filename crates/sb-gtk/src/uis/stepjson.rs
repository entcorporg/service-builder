//! The fields of a step that hold a structure (cookies, `WHERE` conditions, the values of an insert, the claims of a
//! token…): each is edited with its own widgets, as in the web version, never as JSON typed by hand. What a step
//! needs to be shaped is a schema of the project, chosen in a list.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use sb_app::scope::Var;
use sb_app::stepform::{self, Kind};
use sb_model::{Assignment, CmpOp, Cookie, Expr, Field, JwksSource, SameSite, SqlCondition, SqlMissing, Step, Table};
use serde_json::Value;

use super::assignments::expr_editor;
use super::inspector::{choice, commit_entry, link, number};
use super::ui::Shared;

type Commit = Rc<dyn Fn(String)>;
type Apply = Rc<dyn Fn(Option<Expr>)>;

const OPS: [(&str, CmpOp); 6] = [
    ("=", CmpOp::Eq),
    ("≠", CmpOp::Ne),
    (">", CmpOp::Gt),
    ("≥", CmpOp::Ge),
    ("<", CmpOp::Lt),
    ("≤", CmpOp::Le),
];

const STATUSES: [(u16, &str); 9] = [
    (400, "requête invalide"),
    (401, "non authentifié"),
    (403, "interdit"),
    (404, "introuvable"),
    (409, "conflit"),
    (422, "entité invalide"),
    (429, "trop de requêtes"),
    (500, "erreur interne"),
    (503, "indisponible"),
];

fn small(text: &str, class: &str) -> gtk::Label {
    let l = gtk::Label::builder().label(text).xalign(0.0).wrap(true).build();
    l.add_css_class(class);
    l
}

fn row(spacing: i32) -> gtk::Box {
    gtk::Box::new(gtk::Orientation::Horizontal, spacing)
}

fn stack() -> gtk::Box {
    gtk::Box::new(gtk::Orientation::Vertical, 6)
}

/// The editor of a field that holds a structure, or nothing when the field is an ordinary one.
/// `commit` gives the new text of the field, in the form the step understands.
pub fn structured(
    ui: &Shared,
    step: &Step,
    service: usize,
    spec: &stepform::Spec,
    text: &str,
    vars: &[Var],
    commit: Commit,
) -> Option<gtk::Widget> {
    let v = serde_json::to_value(step).ok()?;
    let kind = v["type"].as_str()?;
    match (kind, spec.key.as_str()) {
        ("respond", "cookies") => Some(cookies(text, vars, commit).upcast()),
        ("sql", "where") => {
            let table = table_of(ui, service, &v);
            Some(conditions(table.as_ref(), text, vars, commit).upcast())
        }
        ("sql", "missing") => Some(missing(text, commit).upcast()),
        ("sql", "values") => {
            let table = table_of(ui, service, &v)?;
            Some(column_values(&table, text, vars, commit).upcast())
        }
        ("verifyJwt", "source") => Some(jwks_source(text, commit).upcast()),
        ("verifyJwt", "schema") => Some(schema_pick(ui, text, false, commit).upcast()),
        ("sql", "into") => Some(schema_pick(ui, text, true, commit).upcast()),
        (_, _) if matches!(spec.kind, Kind::Assignments) => {
            let noun = match spec.key.as_str() {
                "claims" => "un claim",
                "data" => "une donnée",
                _ => "un champ",
            };
            Some(pairs(noun, text, vars, commit).upcast())
        }
        _ => None,
    }
}

fn table_of(ui: &Shared, service: usize, step: &Value) -> Option<Table> {
    let name = step["table"].as_str()?;
    let s = ui.state.borrow();
    s.project
        .services
        .get(service)?
        .tables
        .iter()
        .find(|t| t.name == name)
        .cloned()
}

fn parse<T: serde::de::DeserializeOwned + Default>(text: &str) -> T {
    serde_json::from_str(text).unwrap_or_default()
}

fn push<T: serde::Serialize>(commit: &Commit, value: &T) {
    commit(serde_json::to_string_pretty(value).unwrap_or_default());
}

fn expr_of(value: Option<Expr>) -> Expr {
    value.unwrap_or(Expr::Literal {
        value: Value::String(String::new()),
    })
}

// ---- schemas ----

/// A schema is picked in a list: the API schemas for a token, the proto and API ones for the rows of a table.
fn schema_pick(ui: &Shared, text: &str, optional: bool, commit: Commit) -> gtk::DropDown {
    let none = "aucun : une ligne typée par les colonnes".to_string();
    let mut options: Vec<String> = if optional { vec![none.clone()] } else { vec![] };
    options.extend(
        ui.state
            .borrow()
            .project
            .messages
            .iter()
            .filter(|m| {
                if optional {
                    m.kind != sb_model::MessageKind::Event
                } else {
                    m.kind == sb_model::MessageKind::Api
                }
            })
            .map(|m| m.name.clone()),
    );
    let current = if text.trim().is_empty() && optional {
        none.clone()
    } else {
        text.to_string()
    };
    let d = choice(&options, &current, move |v| {
        commit(if v == none { String::new() } else { v })
    });
    d.set_hexpand(true);
    d
}

// ---- assignments with no schema: names the flow chooses ----

fn assignments_of(text: &str) -> Vec<Assignment> {
    stepform::parse_assignments(text)
        .ok()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

fn assignments_text(list: &[Assignment]) -> String {
    stepform::assignments_to_text(&serde_json::to_value(list).unwrap_or_default())
}

/// A list of `name = value`, for what has no schema to say which names exist (the claims of a token, the data of a
/// real-time event).
fn pairs(noun: &str, text: &str, vars: &[Var], commit: Commit) -> gtk::Box {
    let host = stack();
    let state = Rc::new(RefCell::new(assignments_of(text)));
    let save: Rc<dyn Fn()> = {
        let (state, commit) = (state.clone(), commit.clone());
        Rc::new(move || commit(assignments_text(&state.borrow())))
    };
    for i in 0..state.borrow().len() {
        let a = state.borrow()[i].clone();
        let line = row(6);
        let name = commit_entry(&a.field, true, "nom", false, {
            let (state, save) = (state.clone(), save.clone());
            move |v| {
                state.borrow_mut()[i].field = v;
                save();
            }
        });
        name.set_width_chars(10);
        line.append(&name);
        let value: Apply = {
            let (state, save) = (state.clone(), save.clone());
            Rc::new(move |e| {
                if let Some(e) = e {
                    state.borrow_mut()[i].value = e;
                    save();
                }
            })
        };
        let editor = expr_editor(None, false, Some(a.value.clone()), vars, value);
        editor.set_hexpand(true);
        line.append(&editor);
        line.append(&super::inspector::icon_button(
            "trash",
            &format!("Retirer {}", a.field),
            true,
            {
                let (state, save) = (state.clone(), save.clone());
                move || {
                    state.borrow_mut().remove(i);
                    save();
                }
            },
        ));
        host.append(&line);
    }
    host.append(&link(&format!("+ Ajouter {noun}"), {
        let (state, save) = (state.clone(), save.clone());
        move || {
            let mut list = state.borrow_mut();
            let mut n = list.len() + 1;
            while list.iter().any(|a| a.field == format!("champ_{n}")) {
                n += 1;
            }
            list.push(Assignment {
                field: format!("champ_{n}"),
                value: expr_of(None),
            });
            drop(list);
            save();
        }
    }));
    host
}

/// The columns of the table, each with the value an insert or an update gives it (or nothing).
fn column_values(table: &Table, text: &str, vars: &[Var], commit: Commit) -> gtk::Box {
    let host = stack();
    let state = Rc::new(RefCell::new(assignments_of(text)));
    for c in &table.columns {
        let line = row(8);
        let label = gtk::Label::builder().label(&c.name).xalign(0.0).build();
        label.add_css_class("font-mono");
        label.set_width_request(110);
        let needed = !c.nullable && c.default.as_deref().is_none_or(|d| d.trim().is_empty());
        label.set_tooltip_text(Some(&if needed {
            format!("{} : obligatoire pour un ajout (ni nul ni valeur par défaut)", c.name)
        } else {
            c.name.clone()
        }));
        line.append(&label);
        let target: Field = Field::new(&c.name, c.ty.field_type());
        let current = state.borrow().iter().find(|a| a.field == c.name).map(|a| a.value.clone());
        let name = c.name.clone();
        let apply: Apply = {
            let (state, commit) = (state.clone(), commit.clone());
            Rc::new(move |e| {
                {
                    let mut list = state.borrow_mut();
                    list.retain(|a| a.field != name);
                    if let Some(value) = e {
                        list.push(Assignment {
                            field: name.clone(),
                            value,
                        });
                    }
                }
                commit(assignments_text(&state.borrow()));
            })
        };
        let editor = expr_editor(Some(&target), true, current, vars, apply);
        editor.set_hexpand(true);
        line.append(&editor);
        host.append(&line);
    }
    host
}

// ---- WHERE ----

fn conditions(table: Option<&Table>, text: &str, vars: &[Var], commit: Commit) -> gtk::Box {
    let host = stack();
    let state = Rc::new(RefCell::new(parse::<Vec<SqlCondition>>(text)));
    let save: Rc<dyn Fn()> = {
        let (state, commit) = (state.clone(), commit.clone());
        Rc::new(move || push(&commit, &*state.borrow()))
    };
    let columns: Vec<String> = table
        .map(|t| t.columns.iter().map(|c| c.name.clone()).collect())
        .unwrap_or_default();
    let names: Vec<String> = OPS.iter().map(|(n, _)| (*n).to_string()).collect();
    for i in 0..state.borrow().len() {
        let c = state.borrow()[i].clone();
        let column = table.and_then(|t| t.columns.iter().find(|x| x.name == c.column));
        let line = row(6);
        let pick = choice(&columns, &c.column, {
            let (state, save) = (state.clone(), save.clone());
            move |v| {
                state.borrow_mut()[i].column = v;
                save();
            }
        });
        line.append(&pick);
        let shown = OPS.iter().find(|(_, o)| *o == c.op).map_or("=", |(n, _)| *n);
        line.append(&choice(&names, shown, {
            let (state, save) = (state.clone(), save.clone());
            move |v| {
                if let Some((_, o)) = OPS.iter().find(|(n, _)| *n == v) {
                    state.borrow_mut()[i].op = *o;
                    save();
                }
            }
        }));
        let target = column.map(|x| Field::new(&x.name, x.ty.field_type()));
        let apply: Apply = {
            let (state, save) = (state.clone(), save.clone());
            Rc::new(move |e| {
                if let Some(e) = e {
                    state.borrow_mut()[i].value = e;
                    save();
                }
            })
        };
        let editor = expr_editor(target.as_ref(), false, Some(c.value.clone()), vars, apply);
        editor.set_hexpand(true);
        line.append(&editor);
        line.append(&super::inspector::icon_button("trash", "Retirer la condition", true, {
            let (state, save) = (state.clone(), save.clone());
            move || {
                state.borrow_mut().remove(i);
                save();
            }
        }));
        host.append(&line);
    }
    host.append(&link("+ Ajouter une condition", {
        let (state, save) = (state.clone(), save.clone());
        move || {
            let Some(first) = columns.first().cloned() else {
                return;
            };
            state.borrow_mut().push(SqlCondition {
                column: first,
                op: CmpOp::Eq,
                value: Expr::Raw { code: String::new() },
            });
            save();
        }
    }));
    host
}

/// One row expected and none found: answer something else than the 404 of the table.
fn missing(text: &str, commit: Commit) -> gtk::Box {
    let host = stack();
    let current: Option<SqlMissing> = serde_json::from_str(text).ok().flatten();
    let check = gtk::CheckButton::with_label("si aucune ligne ne correspond, répondre autre chose que 404");
    check.set_active(current.is_some());
    check.set_tooltip_text(Some(
        "Un jeton inconnu, expiré ou déjà utilisé, un compte qui n’existe pas…",
    ));
    check.connect_toggled({
        let commit = commit.clone();
        move |c| {
            if c.is_active() {
                push(
                    &commit,
                    &Some(SqlMissing {
                        status: 400,
                        message: "invalid request".into(),
                    }),
                );
            } else {
                commit("null".into());
            }
        }
    });
    host.append(&check);
    if let Some(m) = current {
        let line = row(6);
        let words: Vec<String> = STATUSES.iter().map(|(c, w)| format!("{c} · {w}")).collect();
        let now = STATUSES
            .iter()
            .find(|(c, _)| *c == m.status)
            .map_or(m.status.to_string(), |(c, w)| format!("{c} · {w}"));
        let message = m.message.clone();
        line.append(&choice(&words, &now, {
            let commit = commit.clone();
            move |v| {
                let status = v.split(' ').next().and_then(|s| s.parse().ok()).unwrap_or(400);
                push(
                    &commit,
                    &Some(SqlMissing {
                        status,
                        message: message.clone(),
                    }),
                );
            }
        }));
        let status = m.status;
        let entry = commit_entry(&m.message, false, "ce que le client reçoit", true, move |v| {
            push(&commit, &Some(SqlMissing { status, message: v }))
        });
        entry.set_hexpand(true);
        line.append(&entry);
        host.append(&line);
    }
    host
}

// ---- cookies ----

fn cookies(text: &str, vars: &[Var], commit: Commit) -> gtk::Box {
    let host = stack();
    let state = Rc::new(RefCell::new(parse::<Vec<Cookie>>(text)));
    let save: Rc<dyn Fn()> = {
        let (state, commit) = (state.clone(), commit.clone());
        Rc::new(move || push(&commit, &*state.borrow()))
    };
    let sites = ["lax".to_string(), "strict".to_string(), "none".to_string()];
    for i in 0..state.borrow().len() {
        let c = state.borrow()[i].clone();
        let card = stack();
        card.add_css_class("cookie-card");
        let head = row(6);
        let name = commit_entry(&c.name, true, "nom du cookie", false, {
            let (state, save) = (state.clone(), save.clone());
            move |v| {
                state.borrow_mut()[i].name = v;
                save();
            }
        });
        name.set_hexpand(true);
        head.append(&name);
        head.append(&super::inspector::icon_button(
            "trash",
            &format!("Supprimer le cookie {}", c.name),
            true,
            {
                let (state, save) = (state.clone(), save.clone());
                move || {
                    state.borrow_mut().remove(i);
                    save();
                }
            },
        ));
        card.append(&head);
        let value: Apply = {
            let (state, save) = (state.clone(), save.clone());
            Rc::new(move |e| {
                if let Some(e) = e {
                    state.borrow_mut()[i].value = e;
                    save();
                }
            })
        };
        let text_target = Field::new("", sb_model::FieldType::String);
        card.append(&expr_editor(Some(&text_target), false, Some(c.value.clone()), vars, value));

        let life = row(6);
        let expires = gtk::CheckButton::with_label("expire après");
        expires.set_active(c.max_age_secs.is_some());
        expires.connect_toggled({
            let (state, save) = (state.clone(), save.clone());
            move |b| {
                state.borrow_mut()[i].max_age_secs = b.is_active().then_some(900);
                save();
            }
        });
        life.append(&expires);
        if let Some(secs) = c.max_age_secs {
            life.append(&number(i64::from(secs), 0, 31_536_000, {
                let (state, save) = (state.clone(), save.clone());
                move |v| {
                    state.borrow_mut()[i].max_age_secs = Some(v as u32);
                    save();
                }
            }));
            life.append(&small("secondes", "row-hint"));
        }
        card.append(&life);

        let flags = row(10);
        for (label, on, set) in [
            ("HttpOnly", c.http_only, 0),
            ("Secure", c.secure, 1),
        ] {
            let b = gtk::CheckButton::with_label(label);
            b.set_active(on);
            b.connect_toggled({
                let (state, save) = (state.clone(), save.clone());
                move |b| {
                    let mut list = state.borrow_mut();
                    if set == 0 {
                        list[i].http_only = b.is_active();
                    } else {
                        list[i].secure = b.is_active();
                    }
                    drop(list);
                    save();
                }
            });
            flags.append(&b);
        }
        flags.append(&small("SameSite", "row-hint"));
        let shown = match c.same_site {
            SameSite::Lax => "lax",
            SameSite::Strict => "strict",
            SameSite::None => "none",
        };
        flags.append(&choice(&sites, shown, {
            let (state, save) = (state.clone(), save.clone());
            move |v| {
                state.borrow_mut()[i].same_site = match v.as_str() {
                    "strict" => SameSite::Strict,
                    "none" => SameSite::None,
                    _ => SameSite::Lax,
                };
                save();
            }
        }));
        card.append(&flags);
        host.append(&card);
    }
    host.append(&link("+ Ajouter un cookie", {
        let (state, save) = (state.clone(), save.clone());
        move || {
            state.borrow_mut().push(Cookie::new(
                "session",
                Expr::Literal {
                    value: Value::String(String::new()),
                },
            ));
            save();
        }
    }));
    host
}

// ---- where the keys of a token come from ----

fn jwks_source(text: &str, commit: Commit) -> gtk::Box {
    let host = stack();
    let current: JwksSource = serde_json::from_str(text).unwrap_or(JwksSource::Keys {
        key_envs: vec!["JWT_SIGNING_KEY".into()],
    });
    let kinds = [
        "les graines de ce service".to_string(),
        "le JWKS du service qui signe".to_string(),
    ];
    let shown = match current {
        JwksSource::Keys { .. } => &kinds[0],
        JwksSource::Url { .. } => &kinds[1],
    };
    host.append(&choice(&kinds, shown, {
        let (commit, first) = (commit.clone(), kinds[1].clone());
        move |v| {
            push(
                &commit,
                &if v == first {
                    JwksSource::Url {
                        url_env: "AUTH_JWKS_URL".into(),
                    }
                } else {
                    JwksSource::Keys {
                        key_envs: vec!["JWT_SIGNING_KEY".into()],
                    }
                },
            )
        }
    }));
    match current {
        JwksSource::Url { url_env } => {
            let line = row(6);
            line.append(&small("URL dans", "row-hint"));
            let e = commit_entry(&url_env, true, "", false, move |v| {
                push(&commit, &JwksSource::Url { url_env: v })
            });
            e.set_hexpand(true);
            line.append(&e);
            host.append(&line);
        }
        JwksSource::Keys { key_envs } => {
            let state = Rc::new(RefCell::new(key_envs));
            let save: Rc<dyn Fn()> = {
                let (state, commit) = (state.clone(), commit.clone());
                Rc::new(move || {
                    push(
                        &commit,
                        &JwksSource::Keys {
                            key_envs: state.borrow().clone(),
                        },
                    )
                })
            };
            for i in 0..state.borrow().len() {
                let line = row(6);
                let e = commit_entry(&state.borrow()[i].clone(), true, "", false, {
                    let (state, save) = (state.clone(), save.clone());
                    move |v| {
                        state.borrow_mut()[i] = v;
                        save();
                    }
                });
                e.set_hexpand(true);
                line.append(&e);
                if state.borrow().len() > 1 {
                    line.append(&super::inspector::icon_button("trash", "Retirer la graine", true, {
                        let (state, save) = (state.clone(), save.clone());
                        move || {
                            state.borrow_mut().remove(i);
                            save();
                        }
                    }));
                }
                host.append(&line);
            }
            host.append(&link("+ Ajouter une graine", {
                let (state, save) = (state.clone(), save.clone());
                move || {
                    let n = state.borrow().len() + 1;
                    state.borrow_mut().push(format!("JWT_SIGNING_KEY_{n}"));
                    save();
                }
            }));
        }
    }
    host
}
