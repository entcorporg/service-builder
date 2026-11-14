//! The fields of a step that hold a structure (cookies, `WHERE` conditions, the values of an insert, the claims of a
//! token…): each is edited with its own widgets, as in the web version, never as JSON typed by hand. What a step
//! needs to be shaped is a schema of the project, chosen in a list.
//! (Used by `stepcards`.) Every editor returns the new text of the field, in the form the step understands, when the
//! user changed something.

use eframe::egui::{self, Align, Frame, Layout, Margin, RichText, Stroke, Ui, Vec2};
use sb_app::scope::Var;
use sb_app::stepform::{self, Kind};
use sb_model::{
    Assignment, CmpOp, Column, Cookie, Expr, Field, JwksSource, ResponseHeader, SameSite,
    SqlCondition, SqlJoin, SqlJoinKind, SqlJoinOn, SqlMissing, SqlSort, SqlTest, Step, Table,
};
use serde_json::Value;

use super::assignments::{expr_editor, sql_expr_editor};
use super::common::{commit_entry, icon_btn, link};
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
        | ("respond", "headers")
        | ("sql", "where")
        | ("sql", "missing")
        | ("verifyJwt", "source")
        | ("verifyJwt", "schema")
        | ("sql", "into") => true,
        ("httpCall", "auth" | "response" | "onError" | "bodyFormat") => true,
        ("sql", "values") => table.is_some(),
        ("sql", "table") => true,
        ("sql", "columns" | "orderBy" | "joins" | "thenBy" | "cursor") => table.is_some(),
        _ => matches!(spec.kind, Kind::Assignments),
    };
    if !known {
        return None;
    }
    ui.label(RichText::new(name).size(11.0).color(FAINT));
    let query = Query::of(app, service, &v);
    let out = match (kind, spec.key.as_str()) {
        ("respond", "cookies") => cookies(app, ui, id, text, vars),
        ("respond", "headers") => response_headers(app, ui, id, text, vars),
        ("sql", "table") => table_pick(app, ui, id, service, text),
        ("sql", "columns") => columns_ticks(ui, &query?, text),
        ("sql", "orderBy") => order_pick(ui, id, &query?, text),
        ("sql", "joins") => joins(app, ui, id, service, &query?, text),
        ("sql", "thenBy") => then_by(ui, id, &query?, text),
        ("sql", "cursor") => cursor(app, ui, id, &query?, &v, text, vars),
        ("sql", "where") => conditions(app, ui, id, query.as_ref(), text, vars),
        ("sql", "missing") => missing(ui, id, text),
        ("sql", "values") => column_values(app, ui, id, &table?, text, vars),
        ("verifyJwt", "source") => jwks_source(ui, id, text),
        ("verifyJwt", "schema") => schema_pick(app, ui, id, text, false),
        ("sql", "into") => schema_pick(app, ui, id, text, true),
        ("httpCall", "auth") => labelled(ui, id, text, &HTTP_AUTH),
        ("httpCall", "onError") => labelled(ui, id, text, &HTTP_ON_ERROR),
        ("httpCall", "bodyFormat") => labelled(ui, id, text, &HTTP_BODY_FORMAT),
        ("httpCall", "response") => http_response(app, ui, id, text),
        _ => {
            let noun = match spec.key.as_str() {
                "claims" => "un claim",
                "data" => "une donnée",
                "headers" => "un en-tête",
                "query" => "un paramètre",
                "body" => "un champ du corps",
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

/// What a condition tests, as it is read in the list: the comparison operators, then the other tests.
const TESTS: [(&str, SqlTest); 5] = [
    ("est dans la liste", SqlTest::In),
    ("contient", SqlTest::Contains),
    ("commence par", SqlTest::StartsWith),
    ("est nul", SqlTest::IsNull),
    ("n’est pas nul", SqlTest::IsNotNull),
];

fn test_names() -> Vec<String> {
    let mut v = op_names();
    v.extend(TESTS.iter().map(|(n, _)| (*n).to_string()));
    v
}

fn test_name(c: &SqlCondition) -> String {
    match TESTS.iter().find(|(_, t)| *t == c.test) {
        Some((n, _)) => (*n).to_string(),
        None => op_name(c.op).to_string(),
    }
}

fn set_test(c: &mut SqlCondition, word: &str) {
    if let Some((_, t)) = TESTS.iter().find(|(n, _)| *n == word) {
        c.test = *t;
    } else if let Some((_, o)) = OPS.iter().find(|(n, _)| *n == word) {
        c.test = SqlTest::Compare;
        c.op = *o;
    }
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
    super::assignments::pick(ui, id.with("schema"), &current, &options, f32::MAX)
        .map(|v| if v == none { String::new() } else { v })
}

// ---- outbound HTTP calls ----

/// How the secret of a call is sent (the word of the model, then what the person reads).
const HTTP_AUTH: [(&str, &str); 4] = [
    ("", "aucune"),
    ("bearer", "jeton Bearer"),
    ("basic", "Basic : utilisateur et mot de passe"),
    ("apiKey", "clé d’API dans un en-tête"),
];

const HTTP_ON_ERROR: [(&str, &str); 2] = [
    (
        "fail",
        "arrêter le flux : la transaction est annulée, la route répond une erreur",
    ),
    (
        "continue",
        "continuer : le flux lit le statut et décide (0 : personne n’a répondu)",
    ),
];

const HTTP_BODY_FORMAT: [(&str, &str); 2] = [
    (
        "form",
        "formulaire (x-www-form-urlencoded) : Stripe, Twilio",
    ),
    ("json", "JSON"),
];

/// A choice between words of the model, shown with a sentence each.
fn labelled(ui: &mut Ui, id: egui::Id, text: &str, options: &[(&str, &str)]) -> Option<String> {
    let labels: Vec<String> = options.iter().map(|(_, l)| (*l).to_string()).collect();
    let current = options
        .iter()
        .find(|(w, _)| *w == text.trim())
        .map_or(text, |(_, l)| *l);
    super::assignments::pick(ui, id.with("labelled"), current, &labels, f32::MAX).and_then(|l| {
        options
            .iter()
            .find(|(_, label)| *label == l)
            .map(|(w, _)| (*w).to_string())
    })
}

/// The API schema the JSON answer of a call is read into, or none (the status is all that is used).
fn http_response(app: &mut App, ui: &mut Ui, id: egui::Id, text: &str) -> Option<String> {
    let none = "aucun : seul le statut est lu".to_string();
    let mut options = vec![none.clone()];
    options.extend(
        app.project()
            .messages
            .iter()
            .filter(|m| m.kind == sb_model::MessageKind::Api)
            .map(|m| m.name.clone()),
    );
    let current = if text.trim().is_empty() {
        none.clone()
    } else {
        text.to_string()
    };
    super::assignments::pick(ui, id.with("response"), &current, &options, f32::MAX)
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
    for (i, a) in list.clone().into_iter().enumerate() {
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
            let edited = sql_expr_editor(
                app,
                ui,
                (id, "col", &c.name),
                Some(&target),
                true,
                current.as_ref(),
                vars,
                c.ty == sb_model::ColumnType::Timestamp,
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
    query: Option<&Query>,
    text: &str,
    vars: &[Var],
) -> Option<String> {
    let mut list = parse::<Vec<SqlCondition>>(text);
    let mut changed = false;
    let mut remove = None;
    let columns: Vec<String> = query.map(Query::choices).unwrap_or_default();
    let names = test_names();
    ui.spacing_mut().item_spacing.y = 6.0;
    for (i, c) in list.clone().into_iter().enumerate() {
        let column = query.and_then(|q| q.column(&c.column));
        // In a narrow column the value goes under the column and the operator.
        let narrow = ui.available_width() < 440.0;
        let target = column.map(|x| {
            let mut f = Field::new(&x.name, x.ty.field_type());
            match c.test {
                SqlTest::In => f.repeated = true,
                SqlTest::Contains | SqlTest::StartsWith => f.ty = sb_model::FieldType::String,
                _ => {}
            }
            f
        });
        let mut value = |ui: &mut Ui, reserve: f32| -> Option<Expr> {
            let edited = fill(ui, reserve, |ui| {
                sql_expr_editor(
                    app,
                    ui,
                    (id, "value", i),
                    target.as_ref(),
                    false,
                    Some(&c.value),
                    vars,
                    column.is_some_and(|x| x.ty == sb_model::ColumnType::Timestamp),
                )
            });
            edited.flatten()
        };
        let mut new_value = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if let Some(v) =
                super::assignments::pick(ui, (id, "column", i), &c.column, &columns, 120.0)
            {
                list[i].column = v;
                changed = true;
            }
            if let Some(v) =
                super::assignments::pick(ui, (id, "op", i), &test_name(&c), &names, 110.0)
            {
                set_test(&mut list[i], &v);
                changed = true;
            }
            if !narrow && c.test.has_value() {
                new_value = value(ui, 32.0);
            }
            if icon_btn(ui, "trash", "Retirer la condition", true).clicked() {
                remove = Some(i);
            }
        });
        if narrow && c.test.has_value() {
            new_value = value(ui, 0.0);
        }
        if let Some(e) = new_value {
            list[i].value = e;
            changed = true;
        }
    }
    if let Some(i) = remove {
        list.remove(i);
        changed = true;
    }
    if link(ui, "+ Ajouter une condition").clicked()
        && let Some(first) = columns.first().cloned()
    {
        list.push(SqlCondition::new(
            first,
            CmpOp::Eq,
            Expr::Raw {
                code: String::new(),
            },
        ));
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
            if let Some(v) = super::assignments::pick(ui, id.with("status"), &now, &words, 170.0) {
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
    for (i, c) in list.clone().into_iter().enumerate() {
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
                ui.horizontal_wrapped(|ui| {
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
                    if let Some(v) =
                        super::assignments::pick(ui, (id, "site", i), shown, &sites, 90.0)
                    {
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

// ---- response headers ----

/// Free headers of the answer of a route: a name and a value read from the flow (a field, a literal...).
fn response_headers(
    app: &mut App,
    ui: &mut Ui,
    id: egui::Id,
    text: &str,
    vars: &[Var],
) -> Option<String> {
    let mut list = parse::<Vec<ResponseHeader>>(text);
    let mut changed = false;
    let mut remove = None;
    ui.spacing_mut().item_spacing.y = 6.0;
    for (i, h) in list.clone().into_iter().enumerate() {
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
                        if let Some(v) = commit_entry(
                            ui,
                            (id, "hname", i),
                            &h.name,
                            true,
                            "nom de l'en-tête (Cache-Control…)",
                            false,
                        ) {
                            list[i].name = v;
                            changed = true;
                        }
                    });
                    if icon_btn(
                        ui,
                        "trash",
                        &format!("Supprimer l'en-tête {}", h.name),
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
                    (id, "hvalue", i),
                    Some(&text_target),
                    false,
                    Some(&h.value),
                    vars,
                ) {
                    list[i].value = e;
                    changed = true;
                }
            });
    }
    if let Some(i) = remove {
        list.remove(i);
        changed = true;
    }
    if link(ui, "+ Ajouter un en-tête").clicked() {
        list.push(ResponseHeader {
            name: "Cache-Control".into(),
            value: Expr::Literal {
                value: Value::String("no-store".into()),
            },
        });
        changed = true;
    }
    small(
        ui,
        "Les cookies, Content-Type, Retry-After et les en-têtes CORS sont posés ailleurs : ils sont refusés ici.",
    );
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
    if let Some(v) = super::assignments::pick(ui, id.with("kind"), shown, &kinds, f32::MAX) {
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
            for (i, value) in list.clone().into_iter().enumerate() {
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

// ---- tables, columns and joins of a read ----

/// The tables a select reads (the step's, then the joined ones), to pick columns by name.
struct Query {
    base: String,
    tables: Vec<Table>,
}

impl Query {
    fn of(app: &App, service: usize, step: &Value) -> Option<Query> {
        let base = step["table"].as_str()?.to_string();
        let joins: Vec<SqlJoin> = serde_json::from_value(step["joins"].clone()).unwrap_or_default();
        let p = app.project();
        let svc = p.services.get(service)?;
        let mut tables = vec![svc.table(&base)?.clone()];
        for j in &joins {
            if let Some(t) = svc.table(&j.table) {
                tables.push(t.clone());
            }
        }
        Some(Query { base, tables })
    }

    fn column(&self, name: &str) -> Option<&Column> {
        let (t, c) = name.split_once('.').unwrap_or((&self.base, name));
        self.tables.iter().find(|x| x.name == t)?.column(c)
    }

    /// Every column, as the step writes it: bare for the table of the step, `table.column` for a joined one.
    fn choices(&self) -> Vec<String> {
        self.choices_of(self.tables.len())
    }

    /// The columns of the first `n` tables.
    fn choices_of(&self, n: usize) -> Vec<String> {
        self.tables
            .iter()
            .take(n)
            .flat_map(|t| {
                t.columns.iter().map(|c| {
                    if t.name == self.base {
                        c.name.clone()
                    } else {
                        format!("{}.{}", t.name, c.name)
                    }
                })
            })
            .collect()
    }
}

fn table_pick(
    app: &mut App,
    ui: &mut Ui,
    id: egui::Id,
    service: usize,
    text: &str,
) -> Option<String> {
    let names: Vec<String> = app
        .project()
        .services
        .get(service)
        .map(|s| s.tables.iter().map(|t| t.name.clone()).collect())
        .unwrap_or_default();
    if names.is_empty() {
        small(
            ui,
            "Ce service n’a pas encore de table : créez-en une dans la vue Tables.",
        );
    }
    super::assignments::pick(ui, id.with("table"), text, &names, f32::MAX)
}

/// The columns to read, ticked table by table (the joined ones are prefixed by their table).
fn columns_ticks(ui: &mut Ui, q: &Query, text: &str) -> Option<String> {
    let now: Vec<String> = text
        .split(',')
        .map(|c| c.trim().to_string())
        .filter(|c| !c.is_empty())
        .collect();
    let mut out = None;
    for (n, t) in q.tables.iter().enumerate() {
        if q.tables.len() > 1 {
            small(ui, &t.name);
        }
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            for c in &t.columns {
                let key = if n == 0 {
                    c.name.clone()
                } else {
                    format!("{}.{}", t.name, c.name)
                };
                if let Some(on) = widgets::check(ui, now.contains(&key), &c.name) {
                    // Kept in the order of the tables, so that the row does not shuffle as boxes are ticked.
                    let mut next = now.clone();
                    next.retain(|x| *x != key);
                    if on {
                        next.push(key);
                    }
                    let order = q.choices();
                    next.sort_by_key(|x| order.iter().position(|o| o == x).unwrap_or(usize::MAX));
                    out = Some(next.join(", "));
                }
            }
        });
    }
    out
}

fn order_pick(ui: &mut Ui, id: egui::Id, q: &Query, text: &str) -> Option<String> {
    let none = "aucun tri".to_string();
    let mut options = vec![none.clone()];
    options.extend(q.choices());
    let current = if text.trim().is_empty() {
        none.clone()
    } else {
        text.to_string()
    };
    super::assignments::pick(ui, id.with("order"), &current, &options, f32::MAX)
        .map(|v| if v == none { String::new() } else { v })
}

/// The foreign keys that link a table to the ones before it, as join conditions: a key of the joined table that points at
/// one of them, or a key of one of them that points at the joined table.
fn foreign_key_pairs(q: &Query, before: usize, joined: &Table) -> Vec<(String, Vec<SqlJoinOn>)> {
    let name_of = |t: &Table, c: &str| {
        if t.name == q.base {
            c.to_string()
        } else {
            format!("{}.{c}", t.name)
        }
    };
    let mut out = vec![];
    for earlier in q.tables.iter().take(before) {
        for fk in &joined.foreign_keys {
            if fk.ref_table == earlier.name {
                out.push((
                    format!(
                        "{}({}) → {}({})",
                        joined.name,
                        fk.columns.join(", "),
                        earlier.name,
                        fk.ref_columns.join(", ")
                    ),
                    fk.columns
                        .iter()
                        .zip(&fk.ref_columns)
                        .map(|(own, theirs)| SqlJoinOn {
                            column: name_of(earlier, theirs),
                            ref_column: own.clone(),
                        })
                        .collect(),
                ));
            }
        }
        for fk in &earlier.foreign_keys {
            if fk.ref_table == joined.name {
                out.push((
                    format!(
                        "{}({}) → {}({})",
                        earlier.name,
                        fk.columns.join(", "),
                        joined.name,
                        fk.ref_columns.join(", ")
                    ),
                    fk.columns
                        .iter()
                        .zip(&fk.ref_columns)
                        .map(|(own, theirs)| SqlJoinOn {
                            column: name_of(earlier, own),
                            ref_column: theirs.clone(),
                        })
                        .collect(),
                ));
            }
        }
    }
    out
}

/// The tables joined to the one of the step, each matched along a foreign key (picked in a list) or by pairs of columns.
fn joins(
    app: &mut App,
    ui: &mut Ui,
    id: egui::Id,
    service: usize,
    q: &Query,
    text: &str,
) -> Option<String> {
    let mut list = parse::<Vec<SqlJoin>>(text);
    let all: Vec<Table> = app
        .project()
        .services
        .get(service)
        .map(|s| s.tables.clone())
        .unwrap_or_default();
    let kinds = ["interne".to_string(), "à gauche".to_string()];
    let mut changed = false;
    let mut remove = None;
    ui.spacing_mut().item_spacing.y = 6.0;
    for (i, j) in list.clone().into_iter().enumerate() {
        Frame::new()
            .fill(PANEL2)
            .stroke(Stroke::new(1.0, BORDER))
            .corner_radius(10)
            .inner_margin(Margin::same(8))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let now = if j.kind == SqlJoinKind::Left {
                        &kinds[1]
                    } else {
                        &kinds[0]
                    };
                    if let Some(v) =
                        super::assignments::pick(ui, (id, "kind", i), now, &kinds, 90.0)
                    {
                        list[i].kind = if v == kinds[1] {
                            SqlJoinKind::Left
                        } else {
                            SqlJoinKind::Inner
                        };
                        changed = true;
                    }
                    let others: Vec<String> = all
                        .iter()
                        .filter(|t| {
                            t.name != q.base
                                && (t.name == j.table
                                    || !list.iter().any(|other| other.table == t.name))
                        })
                        .map(|t| t.name.clone())
                        .collect();
                    if let Some(v) =
                        super::assignments::pick(ui, (id, "table", i), &j.table, &others, 160.0)
                    {
                        // A new table: the first foreign key that links it gives the conditions.
                        list[i].on = all
                            .iter()
                            .find(|t| t.name == v)
                            .and_then(|t| foreign_key_pairs(q, i + 1, t).into_iter().next())
                            .map(|(_, on)| on)
                            .unwrap_or_default();
                        list[i].table = v;
                        changed = true;
                    }
                    if icon_btn(ui, "trash", "Retirer la jointure", true).clicked() {
                        remove = Some(i);
                    }
                });
                let jt = all.iter().find(|t| t.name == j.table);
                let left_choices = q.choices_of(i + 1);
                let right_choices: Vec<String> = jt
                    .map(|t| t.columns.iter().map(|c| c.name.clone()).collect())
                    .unwrap_or_default();
                let mut drop_pair = None;
                for (k, on) in j.on.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        if let Some(v) = super::assignments::pick(
                            ui,
                            (id, "on-l", i, k),
                            &on.column,
                            &left_choices,
                            140.0,
                        ) {
                            list[i].on[k].column = v;
                            changed = true;
                        }
                        ui.label("=");
                        if let Some(v) = super::assignments::pick(
                            ui,
                            (id, "on-r", i, k),
                            &format!("{}.{}", j.table, on.ref_column),
                            &right_choices
                                .iter()
                                .map(|c| format!("{}.{c}", j.table))
                                .collect::<Vec<_>>(),
                            140.0,
                        ) {
                            list[i].on[k].ref_column =
                                v.split_once('.').map(|(_, c)| c.to_string()).unwrap_or(v);
                            changed = true;
                        }
                        if icon_btn(ui, "trash", "Retirer cette paire", true).clicked() {
                            drop_pair = Some(k);
                        }
                    });
                }
                if let Some(k) = drop_pair {
                    list[i].on.remove(k);
                    changed = true;
                }
                let fks = jt
                    .map(|t| foreign_key_pairs(q, i + 1, t))
                    .unwrap_or_default();
                if !fks.is_empty() {
                    let labels: Vec<String> = fks.iter().map(|(l, _)| l.clone()).collect();
                    ui.horizontal(|ui| {
                        small(ui, "d’après la clé étrangère");
                        if let Some(v) =
                            super::assignments::pick(ui, (id, "fk", i), "choisir…", &labels, 260.0)
                            && let Some((_, on)) = fks.iter().find(|(l, _)| *l == v)
                        {
                            list[i].on = on.clone();
                            changed = true;
                        }
                    });
                }
                if link(ui, "+ Ajouter une paire de colonnes").clicked()
                    && let (Some(l), Some(r)) = (left_choices.first(), right_choices.first())
                {
                    list[i].on.push(SqlJoinOn {
                        column: l.clone(),
                        ref_column: r.clone(),
                    });
                    changed = true;
                }
            });
    }
    if let Some(i) = remove {
        list.remove(i);
        changed = true;
    }
    if link(ui, "+ Joindre une table").clicked()
        && let Some(t) = all
            .iter()
            .find(|t| t.name != q.base && !list.iter().any(|j| j.table == t.name))
    {
        let on = foreign_key_pairs(q, q.tables.len(), t)
            .into_iter()
            .next()
            .map(|(_, on)| on)
            .unwrap_or_default();
        list.push(SqlJoin {
            kind: SqlJoinKind::Inner,
            table: t.name.clone(),
            alias: None,
            on,
        });
        changed = true;
    }
    changed.then(|| push(&list))
}

/// The sort keys after the first one.
fn then_by(ui: &mut Ui, id: egui::Id, q: &Query, text: &str) -> Option<String> {
    let mut list = parse::<Vec<SqlSort>>(text);
    let choices = q.choices();
    let mut changed = false;
    let mut remove = None;
    for (i, k) in list.clone().into_iter().enumerate() {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if let Some(v) =
                super::assignments::pick(ui, (id, "col", i), &k.column, &choices, 160.0)
            {
                list[i].column = v;
                changed = true;
            }
            if let Some(on) = widgets::check(ui, k.descending, "décroissant") {
                list[i].descending = on;
                changed = true;
            }
            if icon_btn(ui, "trash", "Retirer ce tri", true).clicked() {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        list.remove(i);
        changed = true;
    }
    if link(ui, "+ Ajouter un tri").clicked()
        && let Some(first) = choices.first()
    {
        list.push(SqlSort {
            column: first.clone(),
            descending: false,
        });
        changed = true;
    }
    changed.then(|| push(&list))
}

/// The cursor: the value of each sort key for the row after which the page starts.
fn cursor(
    app: &mut App,
    ui: &mut Ui,
    id: egui::Id,
    q: &Query,
    step: &Value,
    text: &str,
    vars: &[Var],
) -> Option<String> {
    let mut keys: Vec<String> = step["orderBy"]
        .as_str()
        .map(String::from)
        .into_iter()
        .collect();
    if let Ok(more) = serde_json::from_value::<Vec<SqlSort>>(step["thenBy"].clone()) {
        keys.extend(more.into_iter().map(|k| k.column));
    }
    if keys.is_empty() {
        small(
            ui,
            "Choisissez d’abord un tri : le curseur a une valeur par clé de tri.",
        );
        return None;
    }
    let mut list = parse::<Vec<Expr>>(text);
    list.resize(keys.len(), Expr::Literal { value: Value::Null });
    let mut changed = false;
    for (i, key) in keys.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.add_sized(
                [110.0, 22.0],
                egui::Label::new(RichText::new(key).font(mono(SM))).truncate(),
            );
            let mut target = q
                .column(key)
                .map(|x| Field::new(&x.name, x.ty.field_type()));
            if let Some(t) = target.as_mut() {
                t.optional = true;
            }
            let edited = fill(ui, 0.0, |ui| {
                sql_expr_editor(
                    app,
                    ui,
                    (id, "cursor", i),
                    target.as_ref(),
                    false,
                    Some(&list[i]),
                    vars,
                    false,
                )
            });
            if let Some(e) = edited.flatten() {
                list[i] = e;
                changed = true;
            }
        });
    }
    small(
        ui,
        "Un champ de la requête absent (optionnel) donne la première page.",
    );
    changed.then(|| push(&list))
}
