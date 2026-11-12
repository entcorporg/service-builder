//! The schemas (events, proto, API) in the inspector of the Architecture page, as in the web version: three
//! blocks, one card per schema, and its fields edited in place.

use eframe::egui::{self, Frame, Margin, RichText, Stroke, Ui, Vec2};
use sb_app::rename;
use sb_model::names::{is_pascal_ident, is_snake_ident};
use sb_model::{Field, FieldType, Message, MessageKind, Project, Rule};

use super::canvas::{Sel, select};
use super::common::{
    add_message, card, choice, commit_entry, empty, field, icon_btn, link, number, section,
    separator,
};
use super::editors::{SCALARS, type_name};
use super::inspector::add_button;
use crate::app::App;
use crate::theme::*;
use crate::widgets;

const KINDS: [MessageKind; 3] = [MessageKind::Event, MessageKind::Proto, MessageKind::Api];

fn title(kind: MessageKind) -> &'static str {
    match kind {
        MessageKind::Event => "Événements",
        MessageKind::Proto => "Proto (gRPC)",
        MessageKind::Api => "API",
        MessageKind::Row => "Lignes de table",
    }
}

fn one(kind: MessageKind) -> &'static str {
    match kind {
        MessageKind::Event => "Événement",
        MessageKind::Proto => "Proto",
        MessageKind::Api => "API",
        MessageKind::Row => "Ligne",
    }
}

fn hint(kind: MessageKind) -> &'static str {
    match kind {
        MessageKind::Event => "Charge utile d’un topic Kafka",
        MessageKind::Proto => "Requêtes, réponses gRPC et types partagés",
        MessageKind::Api => "Corps, paramètres et réponses des routes HTTP",
        MessageKind::Row => "Ligne d’une table",
    }
}

fn base_name(kind: MessageKind) -> &'static str {
    match kind {
        MessageKind::Event => "NewEvent",
        MessageKind::Proto => "NewMessage",
        MessageKind::Api => "NewApiSchema",
        MessageKind::Row => "NewRow",
    }
}

fn card_id(name: &str) -> String {
    format!("msg:{name}")
}

/// Selects the message of this name in the inspector (opening its card).
pub fn open_message(app: &mut App, name: &str) {
    app.open.insert(card_id(name));
    select(app, Some(Sel::Messages(name.to_string())));
}

fn message_mut<'a>(p: &'a mut Project, name: &str) -> Option<&'a mut Message> {
    p.messages.iter_mut().find(|m| m.name == name)
}

/// The types a field of a schema of this kind can have: API schemas travel as JSON, which has no bytes or timestamp.
fn type_options(p: &Project, kind: MessageKind) -> Vec<String> {
    let api = kind == MessageKind::Api;
    let mut options: Vec<String> = SCALARS
        .iter()
        .map(|(n, _)| (*n).to_string())
        .filter(|n| !api || (n != "bytes" && n != "timestamp"))
        .collect();
    options.extend(
        p.messages
            .iter()
            .filter(|m| (m.kind == MessageKind::Api) == api)
            .map(|m| m.name.clone()),
    );
    options
}

fn parse_type(name: &str) -> FieldType {
    SCALARS.iter().find(|(n, _)| *n == name).map_or_else(
        || FieldType::Message {
            name: name.to_string(),
        },
        |(_, t)| t.clone(),
    )
}

fn next_tag(m: &Message) -> u32 {
    m.fields
        .iter()
        .enumerate()
        .map(|(i, f)| f.tag.unwrap_or(i as u32 + 1))
        .max()
        .unwrap_or(0)
        + 1
}

/// A change of the rules of a field.
type RulesChange = Box<dyn FnOnce(&mut Vec<Rule>)>;

fn is_text(ty: &FieldType) -> bool {
    matches!(ty, FieldType::String | FieldType::Uuid)
}

/// The schemas in the inspector: a block per kind, a card per schema.
pub fn messages(app: &mut App, ui: &mut Ui) {
    let project = app.project();
    for (n, kind) in KINDS.into_iter().enumerate() {
        if n > 0 {
            separator(ui);
        }
        let mut add = false;
        section(
            ui,
            title(kind),
            "schema",
            |ui| add = add_button(ui, &format!("Ajouter un schéma {}", one(kind)), true),
            |ui| {
                ui.add(egui::Label::new(RichText::new(hint(kind)).size(SM).color(MUTED)).wrap());
                let entries: Vec<&Message> =
                    project.messages.iter().filter(|m| m.kind == kind).collect();
                if entries.is_empty() {
                    empty(ui, "Aucun schéma de ce type.");
                }
                for m in entries {
                    message_card(app, ui, &project, m);
                }
            },
        );
        if add {
            let mut added = String::new();
            app.edit(|p| added = add_message(p, base_name(kind), kind));
            open_message(app, &added);
        }
    }
}

fn message_card(app: &mut App, ui: &mut Ui, project: &Project, m: &Message) {
    let count = m.fields.len();
    card(
        app,
        ui,
        &card_id(&m.name),
        None,
        &m.name,
        &format!("{count} champ{}", if count > 1 { "s" } else { "" }),
        |app, ui| {
            if icon_btn(ui, "trash", &format!("Supprimer {}", m.name), true).clicked() {
                app.edit(|p| p.messages.retain(|x| x.name != m.name));
            }
        },
        |app, ui| message_body(app, ui, project, m),
    );
}

fn message_body(app: &mut App, ui: &mut Ui, project: &Project, m: &Message) {
    if let Some(v) = field(ui, "Nom", "", |ui| {
        commit_entry(ui, ("msg-name", &m.name), &m.name, true, "", false)
    }) {
        if !is_pascal_ident(&v) {
            app.say("Le nom doit être en PascalCase, par exemple OrderCreated.");
        } else if project.message(&v).is_some() {
            app.say(&format!("Le schéma {v} existe déjà."));
        } else {
            if app.open.remove(&card_id(&m.name)) {
                app.open.insert(card_id(&v));
            }
            if app.canvas.selected == Some(Sel::Messages(m.name.clone())) {
                app.canvas.selected = Some(Sel::Messages(v.clone()));
            }
            app.edit(|p| rename::message(p, &m.name, &v));
        }
    }

    let kinds: Vec<String> = KINDS.iter().map(|k| one(*k).to_string()).collect();
    if let Some(v) = field(ui, "Type", hint(m.kind), |ui| {
        choice(ui, ("msg-kind", &m.name), &kinds, one(m.kind))
    }) && let Some(kind) = KINDS.into_iter().find(|k| one(*k) == v)
    {
        app.edit(|p| {
            if let Some(x) = message_mut(p, &m.name) {
                x.kind = kind;
            }
        });
    }

    ui.add_space(2.0);
    ui.spacing_mut().item_spacing.y = 6.0;
    for (i, f) in m.fields.iter().enumerate() {
        field_row(app, ui, project, m, i, f);
    }
    if link(ui, "+ Ajouter un champ").clicked() {
        app.edit(|p| {
            let Some(x) = message_mut(p, &m.name) else {
                return;
            };
            let mut n = x.fields.len() + 1;
            while x.fields.iter().any(|f| f.name == format!("field_{n}")) {
                n += 1;
            }
            let mut f = Field::new(format!("field_{n}"), FieldType::String);
            f.tag = Some(next_tag(x));
            x.fields.push(f);
        });
    }
}

/// A change of one field of one schema.
fn change(app: &mut App, message: &str, index: usize, f: impl FnOnce(&mut Field)) {
    app.edit(|p| {
        if let Some(field) = message_mut(p, message).and_then(|m| m.fields.get_mut(index)) {
            f(field);
        }
    });
}

fn field_row(app: &mut App, ui: &mut Ui, project: &Project, m: &Message, index: usize, f: &Field) {
    let id = ("field", &m.name, index);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        // tag 26, two checks 2 x 34, trash 26, six gaps.
        let flexible = (ui.available_width() - 26.0 - 68.0 - 26.0 - 30.0).max(100.0);
        let each = flexible / 2.0;
        let (rect, r) = ui.allocate_exact_size(Vec2::new(26.0, 24.0), egui::Sense::hover());
        ui.painter().text(
            rect.left_center(),
            egui::Align2::LEFT_CENTER,
            format!("#{}", f.tag.unwrap_or(index as u32 + 1)),
            mono(XS),
            FAINT,
        );
        r.on_hover_text("Numéro de champ protobuf, stable");

        if let Some(v) = ui
            .allocate_ui(Vec2::new(each, 24.0), |ui| {
                commit_entry(ui, (id, "name"), &f.name, true, "", false)
            })
            .inner
        {
            let taken = m.fields.iter().any(|x| x.name == v);
            if !is_snake_ident(&v) {
                app.say("Le nom doit être en snake_case, par exemple order_id.");
            } else if taken {
                app.say(&format!("Le champ {v} existe déjà."));
            } else {
                app.edit(|p| rename_field(p, &m.name, &f.name, &v));
            }
        }

        if let Some(v) = ui
            .allocate_ui(Vec2::new(each, 24.0), |ui| {
                choice(
                    ui,
                    (id, "type"),
                    &type_options(project, m.kind),
                    &type_name(&f.ty),
                )
            })
            .inner
        {
            change(app, &m.name, index, |x| {
                x.ty = parse_type(&v);
                if matches!(x.ty, FieldType::Message { .. }) {
                    x.optional = false;
                }
                if !is_text(&x.ty) {
                    x.rules.clear();
                    x.source = None;
                }
            });
        }

        let is_message = matches!(f.ty, FieldType::Message { .. });
        if let Some(on) = widgets::check(
            ui,
            f.repeated,
            RichText::new("[]").font(mono(11.0)).color(MUTED),
        ) {
            change(app, &m.name, index, |x| {
                x.repeated = on;
                if on {
                    x.optional = false;
                    x.source = None;
                }
            });
        }
        let optional = ui
            .add_enabled_ui(!is_message && !f.repeated, |ui| {
                widgets::check(
                    ui,
                    f.optional,
                    RichText::new("?").font(mono(11.0)).color(MUTED),
                )
            })
            .inner;
        if let Some(on) = optional {
            change(app, &m.name, index, |x| {
                x.optional = on;
                if on {
                    x.source = None;
                }
            });
        }
        if icon_btn(ui, "trash", &format!("Supprimer {}", f.name), true).clicked() {
            app.edit(|p| {
                if let Some(x) = message_mut(p, &m.name)
                    && index < x.fields.len()
                {
                    x.fields.remove(index);
                }
            });
        }
    });
    if m.kind == MessageKind::Api && is_text(&f.ty) {
        rules_row(app, ui, &m.name, index, f);
    }
}

/// What a request must satisfy, checked before any step runs (text fields of API schemas).
fn rules_row(app: &mut App, ui: &mut Ui, message: &str, index: usize, f: &Field) {
    let mut pending: Option<RulesChange> = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.add_space(36.0);
        ui.label(RichText::new("RÈGLES").size(10.0).color(FAINT));
        for (i, r) in f.rules.iter().enumerate() {
            Frame::new()
                .fill(PANEL2)
                .stroke(Stroke::new(1.0, BORDER))
                .corner_radius(6)
                .inner_margin(Margin {
                    left: 8,
                    right: 2,
                    top: 2,
                    bottom: 2,
                })
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    match r {
                        Rule::Length { min, max } => {
                            let (min, max) = (*min, *max);
                            ui.label(RichText::new("longueur").size(SM));
                            if let Some(v) = ui
                                .allocate_ui(Vec2::new(46.0, 22.0), |ui| {
                                    number(
                                        ui,
                                        ("rule-lo", message, index),
                                        i64::from(min),
                                        0,
                                        1_000_000,
                                    )
                                })
                                .inner
                            {
                                pending = Some(Box::new(move |rules| {
                                    rules[i] = Rule::Length { min: v as u32, max };
                                }));
                            }
                            ui.label(RichText::new("à").size(SM));
                            if let Some(v) = ui
                                .allocate_ui(Vec2::new(46.0, 22.0), |ui| {
                                    number(
                                        ui,
                                        ("rule-hi", message, index),
                                        i64::from(max),
                                        0,
                                        1_000_000,
                                    )
                                })
                                .inner
                            {
                                pending = Some(Box::new(move |rules| {
                                    rules[i] = Rule::Length { min, max: v as u32 };
                                }));
                            }
                            ui.label(RichText::new("car.").font(mono(XS)).color(FAINT));
                        }
                        Rule::Email => {
                            ui.label(RichText::new("E-mail").size(SM));
                        }
                        Rule::Siren => {
                            ui.label(RichText::new("SIREN").size(SM));
                        }
                    }
                    if icon_btn(
                        ui,
                        "x",
                        &format!("Retirer la règle {} de {}", r.kind(), f.name),
                        false,
                    )
                    .clicked()
                    {
                        pending = Some(Box::new(move |rules| {
                            rules.remove(i);
                        }));
                    }
                });
        }
        let left: Vec<(&str, Rule)> = [
            ("Longueur", Rule::Length { min: 1, max: 128 }),
            ("E-mail", Rule::Email),
            ("SIREN", Rule::Siren),
        ]
        .into_iter()
        .filter(|(_, r)| !f.rules.iter().any(|x| x.kind() == r.kind()))
        .collect();
        if !left.is_empty() {
            let prompt = "+ règle…".to_string();
            let mut options = vec![prompt.clone()];
            options.extend(left.iter().map(|(l, _)| (*l).to_string()));
            if let Some(v) = ui
                .allocate_ui(Vec2::new(100.0, 24.0), |ui| {
                    choice(ui, ("rule-add", message, index), &options, &prompt)
                })
                .inner
                && let Some((_, rule)) = left.iter().find(|(l, _)| *l == v)
            {
                let rule = rule.clone();
                pending = Some(Box::new(move |rules| rules.push(rule)));
            }
        }
    });
    if let Some(apply) = pending {
        change(app, message, index, |x| apply(&mut x.rules));
    }
}

/// Renames a field and what points at it: the key of the topics that carry the schema, the `{param}` of the paths.
fn rename_field(p: &mut Project, message: &str, from: &str, to: &str) {
    let Some(field) =
        message_mut(p, message).and_then(|m| m.fields.iter_mut().find(|f| f.name == from))
    else {
        return;
    };
    field.name = to.to_string();
    for t in p.topics.iter_mut().filter(|t| t.message == message) {
        if t.key_field.as_deref() == Some(from) {
            t.key_field = Some(to.to_string());
        }
    }
    for s in &mut p.services {
        for r in s.routes.iter_mut().filter(|r| r.request == message) {
            r.path = r.path.replace(&format!("{{{from}}}"), &format!("{{{to}}}"));
        }
    }
}
