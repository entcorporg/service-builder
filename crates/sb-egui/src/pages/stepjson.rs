//! The fields of a step that hold a structure (cookies, `WHERE` conditions, the values of an insert, the claims of a
//! token…): each is edited with its own widgets, as in the web version, never as JSON typed by hand. What a step
//! needs to be shaped is a schema of the project, chosen in a list.
//! (Used by `stepcards`.) Every editor returns the new text of the field, in the form the step understands, when the
//! user changed something.

use eframe::egui::{self, Align, Frame, Layout, Margin, RichText, Stroke, Ui, Vec2};
use sb_app::scope::Var;
use sb_app::stepform::{self, Kind};
use sb_model::{
    Assignment, CmpOp, Cookie, Expr, Field, JwksSource, SameSite, SqlCondition, SqlMissing, Step,
    Table,
};
use serde_json::Value;

use super::assignments::expr_editor;
use super::common::{choice, commit_entry, icon_btn, link};
use crate::app::App;
use crate::theme::*;
use crate::widgets;

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

fn small(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(11.0).color(FAINT));
}

/// Gives the control the width left, minus `reserve` for what comes after it on the line.
fn fill<R>(ui: &mut Ui, reserve: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    let w = (ui.available_width() - reserve).max(60.0);
    let h = ui.spacing().interact_size.y;
    ui.allocate_ui_with_layout(Vec2::new(w, h), Layout::left_to_right(Align::Center), add)
        .inner
}

/// The editor of a field that holds a structure, or nothing when the field is an ordinary one (`None`, nothing is
/// drawn). Otherwise it draws `name` and the editor, and returns the new text of the field when it changed.
#[allow(clippy::too_many_arguments)]
pub fn structured(
    app: &mut App,
    ui: &mut Ui,
    id: egui::Id,
    name: &str,
    step: &Step,
    service: usize,
    spec: &stepform::Spec,
    text: &str,
    vars: &[Var],
) -> Option<Option<String>> {
    let v = serde_json::to_value(step).ok()?;
    let kind = v["type"].as_str()?;
    let table = table_of(app, service, &v);
    // What is drawn depends on the field: decide first whether there is an editor, then draw the name above it.
    let known = match (kind, spec.key.as_str()) {
        ("respond", "cookies")
        | ("sql", "where")
        | ("sql", "missing")
        | ("verifyJwt", "source")
        | ("verifyJwt", "schema")
        | ("sql", "into") => true,
        ("sql", "values") => table.is_some(),
        _ => matches!(spec.kind, Kind::Assignments),
    };
    if !known {
        return None;
    }
    ui.label(RichText::new(name).size(11.0).color(FAINT));
    let out = match (kind, spec.key.as_str()) {
        ("respond", "cookies") => cookies(app, ui, id, text, vars),
        ("sql", "where") => conditions(app, ui, id, table.as_ref(), text, vars),
        ("sql", "missing") => missing(ui, id, text),
        ("sql", "values") => column_values(app, ui, id, &table?, text, vars),
        ("verifyJwt", "source") => jwks_source(ui, id, text),
        ("verifyJwt", "schema") => schema_pick(app, ui, id, text, false),
        ("sql", "into") => schema_pick(app, ui, id, text, true),
        _ => {
            let noun = match spec.key.as_str() {
                "claims" => "un claim",
                "data" => "une donnée",
                _ => "un champ",
            };
            pairs(app, ui, id, noun, text, vars)
        }
    };
    Some(out)
}

fn table_of(app: &App, service: usize, step: &Value) -> Option<Table> {
    let name = step["table"].as_str()?;
    app.project()
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

fn push<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).unwrap_or_default()
}

fn expr_of(value: Option<Expr>) -> Expr {
    value.unwrap_or(Expr::Literal {
        value: Value::String(String::new()),
    })
}

fn op_name(op: CmpOp) -> &'static str {
    OPS.iter().find(|(_, o)| *o == op).map_or("=", |(n, _)| *n)
}

fn op_names() -> Vec<String> {
    OPS.iter().map(|(n, _)| (*n).to_string()).collect()
}

fn status_words() -> Vec<String> {
    STATUSES.iter().map(|(c, w)| format!("{c} · {w}")).collect()
}

// ---- schemas ----

/// A schema is picked in a list: the API schemas for a token, the proto and API ones for the rows of a table.
fn schema_pick(
    app: &mut App,
    ui: &mut Ui,
    id: egui::Id,
    text: &str,
    optional: bool,
) -> Option<String> {
    let none = "aucun : une ligne typée par les colonnes".to_string();
    let mut options: Vec<String> = if optional { vec![none.clone()] } else { vec![] };
    options.extend(
        app.project()
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
    choice(ui, id.with("schema"), &options, &current)
        .map(|v| if v == none { String::new() } else { v })
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
fn pairs(
    app: &mut App,
    ui: &mut Ui,
    id: egui::Id,
    noun: &str,
    text: &str,
    vars: &[Var],
) -> Option<String> {
    let mut list = assignments_of(text);
    let mut changed = false;
    let mut remove = None;
    ui.spacing_mut().item_spacing.y = 6.0;
    for i in 0..list.len() {
        let a = list[i].clone();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if let Some(v) = widgets::text_field_hint(ui, (id, "name", i), &a.field, 110.0, "nom")
                && !v.is_empty()
            {
                list[i].field = v;
                changed = true;
            }
            let edited = fill(ui, 32.0, |ui| {
                expr_editor(app, ui, (id, "value", i), None, false, Some(&a.value), vars)
            });
            if let Some(Some(e)) = edited {
                list[i].value = e;
                changed = true;
            }
            if icon_btn(ui, "trash", &format!("Retirer {}", a.field), true).clicked() {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        list.remove(i);
        changed = true;
    }
    if link(ui, &format!("+ Ajouter {noun}")).clicked() {
        let mut n = list.len() + 1;
        while list.iter().any(|a| a.field == format!("champ_{n}")) {
            n += 1;
        }
        list.push(Assignment {
            field: format!("champ_{n}"),
            value: expr_of(None),
        });
        changed = true;
    }
    changed.then(|| assignments_text(&list))
}

/// The columns of the table, each with the value an insert or an update gives it (or nothing).
fn column_values(
    app: &mut App,
    ui: &mut Ui,
    id: egui::Id,
    table: &Table,
    text: &str,
    vars: &[Var],
) -> Option<String> {
    let mut list = assignments_of(text);
    let mut changed = false;
    ui.spacing_mut().item_spacing.y = 6.0;
    for c in &table.columns {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let needed = !c.nullable && c.default.as_deref().is_none_or(|d| d.trim().is_empty());
            ui.add_sized(
                [110.0, 22.0],
                egui::Label::new(RichText::new(&c.name).font(mono(SM))).truncate(),
            )
            .on_hover_text(if needed {
                format!(
                    "{} : obligatoire pour un ajout (ni nul ni valeur par défaut)",
                    c.name
                )
            } else {
                c.name.clone()
            });
            let target = Field::new(&c.name, c.ty.field_type());
            let current = list
                .iter()
                .find(|a| a.field == c.name)
                .map(|a| a.value.clone());
            let edited = expr_editor(
                app,
                ui,
                (id, "col", &c.name),
                Some(&target),
                true,
                current.as_ref(),
                vars,
            );
            if let Some(e) = edited {
                list.retain(|a| a.field != c.name);
                if let Some(value) = e {
                    list.push(Assignment {
                        field: c.name.clone(),
                        value,
                    });
                }
                changed = true;
            }
        });
    }
    changed.then(|| assignments_text(&list))
}

// ---- WHERE ----

fn conditions(
    app: &mut App,
    ui: &mut Ui,
    id: egui::Id,
    table: Option<&Table>,
    text: &str,
    vars: &[Var],
) -> Option<String> {
    let mut list = parse::<Vec<SqlCondition>>(text);
    let mut changed = false;
    let mut remove = None;
    let columns: Vec<String> = table
        .map(|t| t.columns.iter().map(|c| c.name.clone()).collect())
        .unwrap_or_default();
    let names = op_names();
    ui.spacing_mut().item_spacing.y = 6.0;
    for i in 0..list.len() {
        let c = list[i].clone();
        let column = table.and_then(|t| t.columns.iter().find(|x| x.name == c.column));
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if let Some(v) = widgets::choice(ui, (id, "column", i), &c.column, &columns, 120.0) {
                list[i].column = v;
                changed = true;
            }
            if let Some(v) = widgets::choice(ui, (id, "op", i), op_name(c.op), &names, 52.0)
                && let Some((_, o)) = OPS.iter().find(|(n, _)| *n == v)
            {
                list[i].op = *o;
                changed = true;
            }
            let target = column.map(|x| Field::new(&x.name, x.ty.field_type()));
            let edited = fill(ui, 32.0, |ui| {
                expr_editor(
                    app,
                    ui,
                    (id, "value", i),
                    target.as_ref(),
                    false,
                    Some(&c.value),
                    vars,
                )
            });
            if let Some(Some(e)) = edited {
                list[i].value = e;
                changed = true;
            }
            if icon_btn(ui, "trash", "Retirer la condition", true).clicked() {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        list.remove(i);
        changed = true;
    }
    if link(ui, "+ Ajouter une condition").clicked()
        && let Some(first) = columns.first().cloned()
    {
        list.push(SqlCondition {
            column: first,
            op: CmpOp::Eq,
            value: Expr::Raw {
                code: String::new(),
            },
        });
        changed = true;
    }
    changed.then(|| push(&list))
}

/// One row expected and none found: answer something else than the 404 of the table.
fn missing(ui: &mut Ui, id: egui::Id, text: &str) -> Option<String> {
    let current: Option<SqlMissing> = serde_json::from_str(text).ok().flatten();
    let mut out = None;
    let mut on = current.is_some();
    let r = ui.checkbox(
        &mut on,
        "si aucune ligne ne correspond, répondre autre chose que 404",
    );
    r.clone()
        .on_hover_text("Un jeton inconnu, expiré ou déjà utilisé, un compte qui n’existe pas…");
    if r.changed() {
        out = Some(if on {
            push(&Some(SqlMissing {
                status: 400,
                message: "invalid request".into(),
            }))
        } else {
            "null".into()
        });
    }
    if let Some(m) = current {
        let words = status_words();
        let now = STATUSES
            .iter()
            .find(|(c, _)| *c == m.status)
            .map_or(m.status.to_string(), |(c, w)| format!("{c} · {w}"));
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if let Some(v) = widgets::choice(ui, id.with("status"), &now, &words, 170.0) {
                let status = v
                    .split(' ')
                    .next()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(400);
                out = Some(push(&Some(SqlMissing {
                    status,
                    message: m.message.clone(),
                })));
            }
            if let Some(v) = commit_entry(
                ui,
                id.with("message"),
                &m.message,
                false,
                "ce que le client reçoit",
                true,
            ) {
                out = Some(push(&Some(SqlMissing {
                    status: m.status,
                    message: v,
                })));
            }
        });
    }
    out
}

// ---- cookies ----

fn cookies(app: &mut App, ui: &mut Ui, id: egui::Id, text: &str, vars: &[Var]) -> Option<String> {
    let mut list = parse::<Vec<Cookie>>(text);
    let mut changed = false;
    let mut remove = None;
    let sites = ["lax".to_string(), "strict".to_string(), "none".to_string()];
    ui.spacing_mut().item_spacing.y = 6.0;
    for i in 0..list.len() {
        let c = list[i].clone();
        Frame::new()
            .fill(PANEL2)
            .stroke(Stroke::new(1.0, BORDER))
            .corner_radius(10)
            .inner_margin(Margin::symmetric(10, 8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 6.0;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    fill(ui, 32.0, |ui| {
                        if let Some(v) =
                            commit_entry(ui, (id, "name", i), &c.name, true, "nom du cookie", false)
                        {
                            list[i].name = v;
                            changed = true;
                        }
                    });
                    if icon_btn(
                        ui,
                        "trash",
                        &format!("Supprimer le cookie {}", c.name),
                        true,
                    )
                    .clicked()
                    {
                        remove = Some(i);
                    }
                });
                let text_target = Field::new("", sb_model::FieldType::String);
                if let Some(Some(e)) = expr_editor(
                    app,
                    ui,
                    (id, "value", i),
                    Some(&text_target),
                    false,
                    Some(&c.value),
                    vars,
                ) {
                    list[i].value = e;
                    changed = true;
                }
                ui.horizontal(|ui| {
                    if let Some(on) = widgets::check(ui, c.max_age_secs.is_some(), "expire après")
                    {
                        list[i].max_age_secs = on.then_some(900);
                        changed = true;
                    }
                    if let Some(secs) = c.max_age_secs {
                        if let Some(v) = widgets::number_field(
                            ui,
                            (id, "age", i),
                            i64::from(secs),
                            0,
                            31_536_000,
                        ) {
                            list[i].max_age_secs = Some(v as u32);
                            changed = true;
                        }
                        small(ui, "secondes");
                    }
                });
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    if let Some(on) = widgets::check(ui, c.http_only, "HttpOnly") {
                        list[i].http_only = on;
                        changed = true;
                    }
                    if let Some(on) = widgets::check(ui, c.secure, "Secure") {
                        list[i].secure = on;
                        changed = true;
                    }
                    small(ui, "SameSite");
                    let shown = match c.same_site {
                        SameSite::Lax => "lax",
                        SameSite::Strict => "strict",
                        SameSite::None => "none",
                    };
                    if let Some(v) = widgets::choice(ui, (id, "site", i), shown, &sites, 90.0) {
                        list[i].same_site = match v.as_str() {
                            "strict" => SameSite::Strict,
                            "none" => SameSite::None,
                            _ => SameSite::Lax,
                        };
                        changed = true;
                    }
                });
            });
    }
    if let Some(i) = remove {
        list.remove(i);
        changed = true;
    }
    if link(ui, "+ Ajouter un cookie").clicked() {
        list.push(Cookie::new(
            "session",
            Expr::Literal {
                value: Value::String(String::new()),
            },
        ));
        changed = true;
    }
    changed.then(|| push(&list))
}

// ---- where the keys of a token come from ----

fn jwks_source(ui: &mut Ui, id: egui::Id, text: &str) -> Option<String> {
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
    let mut out = None;
    if let Some(v) = choice(ui, id.with("kind"), &kinds, shown) {
        out = Some(push(&if v == kinds[1] {
            JwksSource::Url {
                url_env: "AUTH_JWKS_URL".into(),
            }
        } else {
            JwksSource::Keys {
                key_envs: vec!["JWT_SIGNING_KEY".into()],
            }
        }));
    }
    match current {
        JwksSource::Url { url_env } => {
            ui.horizontal(|ui| {
                small(ui, "URL dans");
                if let Some(v) = commit_entry(ui, id.with("url"), &url_env, true, "", false) {
                    out = Some(push(&JwksSource::Url { url_env: v }));
                }
            });
        }
        JwksSource::Keys { key_envs } => {
            let mut list = key_envs;
            let mut changed = false;
            let mut remove = None;
            ui.spacing_mut().item_spacing.y = 6.0;
            for i in 0..list.len() {
                let value = list[i].clone();
                ui.horizontal(|ui| {
                    let reserve = if list.len() > 1 { 32.0 } else { 0.0 };
                    fill(ui, reserve, |ui| {
                        if let Some(v) = commit_entry(ui, (id, "seed", i), &value, true, "", false)
                        {
                            list[i] = v;
                            changed = true;
                        }
                    });
                    if list.len() > 1 && icon_btn(ui, "trash", "Retirer la graine", true).clicked()
                    {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                list.remove(i);
                changed = true;
            }
            if link(ui, "+ Ajouter une graine").clicked() {
                let n = list.len() + 1;
                list.push(format!("JWT_SIGNING_KEY_{n}"));
                changed = true;
            }
            if changed {
                out = Some(push(&JwksSource::Keys { key_envs: list }));
            }
        }
    }
    out
}
