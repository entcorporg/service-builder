//! The Services and Messages-and-topics pages: lists with the buttons that edit them.
//!
//! Names of services, messages and topics are fixed once created: the steps refer to them by name, and renaming
//! would have to rewrite every step. The visual editor of the steps is not ported yet.

use std::rc::Rc;

use gtk::prelude::*;
use sb_model::names::{is_pascal_ident, is_snake_ident, is_topic_name};
use sb_model::{
    Field, FieldType, Message, MessageKind, Position, Project, RequestSource, Rule, Service,
    ServiceKind, Topic,
};

use sb_app::rename;

use super::edit::edit;
use super::flows::draw_flows;
use super::form::{Input, Kind, Value, ask};
use super::ui::Shared;
use super::widgets::{clear, names, row, section};

pub(super) const SCALARS: [(&str, FieldType); 9] = [
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

pub type Action = (&'static str, Rc<dyn Fn()>);

pub fn act(name: &'static str, f: impl Fn() + 'static) -> Action {
    (name, Rc::new(f))
}

/// A row with its text on the left and its buttons on the right.
pub fn card(title: &str, subtitle: &str, indent: i32, actions: Vec<Action>) -> gtk::ListBoxRow {
    let r = row(title, subtitle);
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let content = r.child().expect("row has a child");
    content.set_hexpand(true);
    content.set_margin_start(16 + indent);
    r.set_child(None::<&gtk::Widget>);
    line.append(&content);
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    buttons.set_valign(gtk::Align::Center);
    buttons.set_margin_end(12);
    for (name, f) in actions {
        let b = gtk::Button::with_label(name);
        if name == "Supprimer" {
            b.add_css_class("destructive-action");
        }
        b.connect_clicked(move |_| f());
        buttons.append(&b);
    }
    line.append(&buttons);
    r.set_child(Some(&line));
    r.add_css_class(if indent > 0 { "step" } else { "item" });
    r
}

pub fn add_row(text: &'static str, f: impl Fn() + 'static) -> gtk::ListBoxRow {
    let r = gtk::ListBoxRow::new();
    r.set_selectable(false);
    r.set_activatable(false);
    let b = gtk::Button::with_label(text);
    b.set_halign(gtk::Align::Start);
    b.set_margin_top(8);
    b.set_margin_bottom(8);
    b.set_margin_start(16);
    b.connect_clicked(move |_| f());
    r.set_child(Some(&b));
    r
}

pub fn draw_project(ui: &Shared) {
    let p = ui.state.borrow().project.clone();
    draw_services(ui, &p);
    draw_contracts(ui, &p);
    draw_flows(ui, &p);
    super::tablesview::draw(ui, &p);
    super::canvas::refresh(ui);
}

// ---- services ----

/// Removes the service, and its name from the producers of the topics.
pub fn delete_service(ui: &Shared, index: usize) {
    edit(ui, move |p| {
        if index >= p.services.len() {
            return;
        }
        let name = p.services.remove(index).name;
        for t in &mut p.topics {
            t.producers.retain(|s| *s != name);
        }
    })
}

pub fn delete_topic(ui: &Shared, index: usize) {
    edit(ui, move |p| {
        if index < p.topics.len() {
            p.topics.remove(index);
        }
    })
}

fn draw_services(ui: &Shared, p: &Project) {
    clear(&ui.services);
    ui.services.append(&add_row("Ajouter un service", {
        let ui = ui.clone();
        move || service_form(&ui, None)
    }));
    for (i, svc) in p.services.iter().enumerate() {
        let http = svc
            .http_port
            .map_or(String::new(), |h| format!(" · HTTP {h}"));
        let detail = format!(
            "gRPC {}{http}\nRPC : {}\nRoutes : {}\nConsommateurs : {}\nJobs : {}\nTables : {}",
            svc.port,
            names(svc.rpcs.iter().map(|r| r.name.as_str())),
            names(svc.routes.iter().map(|r| r.name.as_str())),
            names(svc.consumers.iter().map(|c| c.name.as_str())),
            names(svc.jobs.iter().map(|j| j.name.as_str())),
            names(svc.tables.iter().map(|t| t.name.as_str())),
        );
        let (u1, u2) = (ui.clone(), ui.clone());
        ui.services.append(&card(
            &format!("{} — {:?}", svc.name, svc.kind),
            &detail,
            0,
            vec![
                act("Modifier", move || service_form(&u1, Some(i))),
                act("Supprimer", move || delete_service(&u2, i)),
            ],
        ));
    }
    if p.services.is_empty() {
        ui.services.append(&row("Aucun service", ""));
    }
}

const SERVICE_KINDS: [(&str, ServiceKind); 3] = [
    ("Service standard", ServiceKind::Standard),
    ("Service mail (MTA prêt à l'emploi)", ServiceKind::Mta),
    (
        "Service temps réel (SSE prêt à l'emploi)",
        ServiceKind::Realtime,
    ),
];

pub fn service_form(ui: &Shared, index: Option<usize>) {
    let current = index.and_then(|i| ui.state.borrow().project.services.get(i).cloned());
    let number = |v: i64| Kind::Number {
        value: v,
        min: 0,
        max: 65535,
    };
    // The next free gRPC port, as the web editor chose it.
    let next_grpc = {
        let s = ui.state.borrow();
        i64::from(
            s.project
                .services
                .iter()
                .map(|s| s.port)
                .max()
                .unwrap_or(0)
                .max(50050),
        ) + 1
    };
    let mut inputs = vec![];
    if current.is_none() {
        let kinds: Vec<String> = SERVICE_KINDS
            .iter()
            .map(|(l, _)| (*l).to_string())
            .collect();
        inputs.push(Input::new(
            "Type",
            Kind::Choice {
                selected: kinds.first().cloned(),
                options: kinds,
            },
        ));
    }
    inputs.push(
        Input::new(
            "Nom (PascalCase)",
            Kind::Text(
                current
                    .as_ref()
                    .map_or_else(String::new, |s| s.name.clone()),
            ),
        )
        .help(
            "Renommer met à jour les producteurs, les appels et les étapes qui y font référence.",
        ),
    );
    inputs.push(Input::new(
        "Port gRPC",
        number(current.as_ref().map_or(next_grpc, |s| i64::from(s.port))),
    ));
    inputs.push(Input::new(
        "Port HTTP (0 : aucun)",
        number(match &current {
            Some(s) => s.http_port.map_or(0, i64::from),
            None => 0,
        }),
    )
    .help("Obligatoire pour un service mail ou temps réel (santé, métriques, flux) : laissé à 0, le prochain port libre est pris."));
    let title = if current.is_some() {
        "Modifier le service"
    } else {
        "Nouveau service"
    };
    ask(ui, title, inputs, move |ui, values| {
        let mut v = values.into_iter();
        let kind = if index.is_none() {
            let label = v.next().map(Value::text).unwrap_or_default();
            SERVICE_KINDS
                .iter()
                .find(|(l, _)| *l == label)
                .map_or(ServiceKind::Standard, |(_, k)| *k)
        } else {
            ServiceKind::Standard
        };
        let name = v.next().map(Value::text).unwrap_or_default();
        let port = v.next().map_or(0, Value::number) as u16;
        let http = v.next().map_or(0, Value::number) as u16;
        let old = current.as_ref().map(|s| s.name.clone());
        if old.as_deref() != Some(name.as_str()) {
            if !is_pascal_ident(&name) {
                return Err("Le nom doit être en PascalCase, par exemple Orders.".into());
            }
            if ui.state.borrow().project.service(&name).is_some() {
                return Err(format!("Le service {name} existe déjà."));
            }
        }
        if port == 0 {
            return Err("Le port gRPC est obligatoire.".into());
        }
        edit(ui, |p| {
            let mut http_port = (http != 0).then_some(http);
            match index {
                Some(i) => {
                    if let Some(old) = old.as_deref().filter(|o| *o != name) {
                        rename::service(p, old, &name);
                    }
                    if let Some(s) = p.services.get_mut(i) {
                        s.port = port;
                        s.http_port = http_port;
                    }
                }
                None => {
                    if kind != ServiceKind::Standard && http_port.is_none() {
                        let top = p.services.iter().filter_map(|s| s.http_port).max();
                        http_port = Some(top.unwrap_or(8080).max(8080) + 1);
                    }
                    let n = p.services.len() as f64;
                    let mut svc = new_service(&name, port, http_port, n);
                    svc.kind = kind;
                    if kind == ServiceKind::Realtime {
                        svc.realtime = Some(sb_model::RealtimeConfig::default());
                    }
                    p.services.push(svc);
                }
            }
        });
        Ok(())
    });
}

fn new_service(name: &str, port: u16, http_port: Option<u16>, n: f64) -> Service {
    Service {
        name: name.into(),
        port,
        rpcs: vec![],
        http_port,
        routes: vec![],
        tables: vec![],
        consumers: vec![],
        jobs: vec![],
        position: Position {
            x: n * 460.0,
            y: 120.0,
        },
        kind: ServiceKind::Standard,
        templates: vec![],
        realtime: None,
    }
}

// ---- messages and topics ----

fn draw_contracts(ui: &Shared, p: &Project) {
    clear(&ui.contracts);
    section(&ui.contracts, "Messages");
    ui.contracts.append(&add_row("Ajouter un message", {
        let ui = ui.clone();
        move || message_form(&ui)
    }));
    for (mi, m) in p.messages.iter().enumerate() {
        let (u1, u2, u3) = (ui.clone(), ui.clone(), ui.clone());
        ui.contracts.append(&card(
            &format!("{} ({:?})", m.name, m.kind),
            &format!("{} champ(s)", m.fields.len()),
            0,
            vec![
                act("Modifier", move || message_kind_form(&u3, mi)),
                act("Ajouter un champ", move || field_form(&u1, mi, None)),
                act("Supprimer", move || {
                    edit(&u2, move |p| {
                        p.messages.remove(mi);
                    })
                }),
            ],
        ));
        for (fi, f) in m.fields.iter().enumerate() {
            let (u1, u2) = (ui.clone(), ui.clone());
            let ty = type_name(&f.ty);
            let flags = [
                f.repeated.then_some("repeated"),
                f.optional.then_some("optional"),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ");
            ui.contracts.append(&card(
                &f.name,
                format!("{ty} {flags}").trim(),
                24,
                vec![
                    act("Modifier", move || field_form(&u1, mi, Some(fi))),
                    act("Supprimer", move || {
                        edit(&u2, move |p| {
                            if let Some(m) = p.messages.get_mut(mi) {
                                m.fields.remove(fi);
                            }
                        })
                    }),
                ],
            ));
        }
    }

    section(&ui.contracts, "Topics");
    ui.contracts.append(&add_row("Ajouter un topic", {
        let ui = ui.clone();
        move || topic_form(&ui, None)
    }));
    for (ti, t) in p.topics.iter().enumerate() {
        let key = t.key_field.as_deref().unwrap_or("—");
        let detail = format!(
            "Message : {}\nPartitions : {} · Réplication : {} · Clé : {key}\nProducteurs : {}",
            t.message,
            t.partitions,
            t.replication_factor,
            names(t.producers.iter().map(String::as_str)),
        );
        let (u1, u2) = (ui.clone(), ui.clone());
        ui.contracts.append(&card(
            &t.name,
            &detail,
            0,
            vec![
                act("Modifier", move || topic_form(&u1, Some(ti))),
                act("Supprimer", move || delete_topic(&u2, ti)),
            ],
        ));
    }
}

pub(super) fn type_name(ty: &FieldType) -> String {
    match ty {
        FieldType::Message { name } => name.clone(),
        other => SCALARS
            .iter()
            .find(|(_, t)| t == other)
            .map_or_else(|| format!("{other:?}"), |(n, _)| (*n).to_string()),
    }
}

fn message_form(ui: &Shared) {
    let kinds = vec!["Event".to_string(), "Proto".to_string(), "Api".to_string()];
    let inputs = vec![
        Input::new("Nom (PascalCase)", Kind::Text(String::new())),
        Input::new(
            "Rôle",
            Kind::Choice {
                options: kinds,
                selected: Some("Event".into()),
            },
        ),
    ];
    ask(ui, "Nouveau message", inputs, |ui, values| {
        let mut v = values.into_iter();
        let name = v.next().map(Value::text).unwrap_or_default();
        let kind = match v.next().map(Value::text).as_deref() {
            Some("Api") => MessageKind::Api,
            Some("Proto") => MessageKind::Proto,
            _ => MessageKind::Event,
        };
        if !is_pascal_ident(&name) {
            return Err("Le nom doit être en PascalCase, par exemple OrderCreated.".into());
        }
        if ui.state.borrow().project.message(&name).is_some() {
            return Err(format!("Le message {name} existe déjà."));
        }
        edit(ui, |p| {
            p.messages.push(Message {
                name,
                kind,
                fields: vec![],
            })
        });
        Ok(())
    });
}

const SOURCE_NONE: &str = "(corps ou paramètres)";
const SOURCE_HEADER: &str = "en-tête";
const SOURCE_COOKIE: &str = "cookie";

fn rules_text(rules: &[Rule]) -> String {
    rules
        .iter()
        .map(|r| match r {
            Rule::Length { min, max } => format!("length {min} {max}"),
            Rule::Email => "email".into(),
            Rule::Siren => "siren".into(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn parse_rules(text: &str) -> Result<Vec<Rule>, String> {
    let mut rules: Vec<Rule> = vec![];
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let words: Vec<&str> = line.split_whitespace().collect();
        let rule = match words.as_slice() {
            ["email"] => Rule::Email,
            ["siren"] => Rule::Siren,
            ["length", min, max] => match (min.parse(), max.parse()) {
                (Ok(min), Ok(max)) if min <= max => Rule::Length { min, max },
                _ => return Err(format!("« {line} » : length MIN MAX, avec MIN <= MAX.")),
            },
            _ => {
                return Err(format!(
                    "« {line} » : règles possibles : length MIN MAX, email, siren."
                ));
            }
        };
        if rules.iter().any(|r| r.kind() == rule.kind()) {
            return Err(format!("La règle {} est écrite deux fois.", rule.kind()));
        }
        rules.push(rule);
    }
    Ok(rules)
}

fn field_form(ui: &Shared, message: usize, index: Option<usize>) {
    let (current, mut types, api) = {
        let s = ui.state.borrow();
        let current = index.and_then(|i| s.project.messages.get(message)?.fields.get(i).cloned());
        let mut types: Vec<String> = SCALARS.iter().map(|(n, _)| (*n).to_string()).collect();
        types.extend(s.project.messages.iter().map(|m| m.name.clone()));
        let api = s
            .project
            .messages
            .get(message)
            .is_some_and(|m| m.kind == MessageKind::Api);
        (current, types, api)
    };
    types.dedup();
    let mut inputs = vec![
        Input::new(
            "Nom (snake_case)",
            Kind::Text(
                current
                    .as_ref()
                    .map_or_else(String::new, |f| f.name.clone()),
            ),
        ),
        Input::new(
            "Type",
            Kind::Choice {
                options: types.clone(),
                selected: current.as_ref().map(|f| type_name(&f.ty)),
            },
        ),
        Input::new(
            "Répété",
            Kind::Check(current.as_ref().is_some_and(|f| f.repeated)),
        ),
        Input::new(
            "Optionnel",
            Kind::Check(current.as_ref().is_some_and(|f| f.optional)),
        ),
    ];
    if api {
        let (source, name) = match current.as_ref().and_then(|f| f.source.clone()) {
            Some(RequestSource::Header { name }) => (SOURCE_HEADER, name),
            Some(RequestSource::Cookie { name }) => (SOURCE_COOKIE, name),
            None => (SOURCE_NONE, String::new()),
        };
        inputs.push(
            Input::new(
                "Règles (champs texte)",
                Kind::Multiline(rules_text(current.as_ref().map_or(&[][..], |f| &f.rules))),
            )
            .help("Une par ligne : length MIN MAX, email, siren. Vérifiées par la route avant toute étape (400 sinon)."),
        );
        inputs.push(Input::new(
            "Lu depuis",
            Kind::Choice {
                options: vec![
                    SOURCE_NONE.into(),
                    SOURCE_HEADER.into(),
                    SOURCE_COOKIE.into(),
                ],
                selected: Some(source.into()),
            },
        ));
        inputs.push(
            Input::new("Nom de l'en-tête ou du cookie", Kind::Text(name))
                .help("Remplacé à chaque requête : le corps ne peut pas le fournir."),
        );
    }
    let title = if current.is_some() {
        "Modifier le champ"
    } else {
        "Nouveau champ"
    };
    ask(ui, title, inputs, move |ui, values| {
        let mut v = values.into_iter();
        let name = v.next().map(Value::text).unwrap_or_default();
        let ty_name = v.next().map(Value::text).unwrap_or_default();
        let repeated = v.next().is_some_and(Value::check);
        let optional = v.next().is_some_and(Value::check);
        let (mut rules, mut source) = (vec![], None);
        if api {
            rules = parse_rules(&v.next().map(Value::text).unwrap_or_default())?;
            let from = v.next().map(Value::text).unwrap_or_default();
            let source_name = v.next().map(Value::text).unwrap_or_default();
            source = match from.as_str() {
                SOURCE_HEADER | SOURCE_COOKIE if source_name.is_empty() => {
                    return Err("Donnez le nom de l'en-tête ou du cookie.".into());
                }
                SOURCE_HEADER => Some(RequestSource::Header { name: source_name }),
                SOURCE_COOKIE => Some(RequestSource::Cookie { name: source_name }),
                _ => None,
            };
        }
        if !is_snake_ident(&name) {
            return Err("Le nom doit être en snake_case, par exemple order_id.".into());
        }
        let taken = ui
            .state
            .borrow()
            .project
            .messages
            .get(message)
            .is_some_and(|m| {
                m.fields
                    .iter()
                    .enumerate()
                    .any(|(i, f)| f.name == name && Some(i) != index)
            });
        if taken {
            return Err(format!("Le champ {name} existe déjà."));
        }
        let ty = SCALARS
            .iter()
            .find(|(n, _)| *n == ty_name)
            .map_or(FieldType::Message { name: ty_name }, |(_, t)| t.clone());
        // Rules and sources only make sense on a plain text field.
        let text = matches!(ty, FieldType::String | FieldType::Uuid) && !repeated;
        if !text {
            (rules, source) = (vec![], None);
        }
        edit(ui, |p| {
            let Some(m) = p.messages.get_mut(message) else {
                return;
            };
            let apply = |f: &mut Field| {
                f.name = name.clone();
                f.ty = ty.clone();
                f.repeated = repeated;
                f.optional = optional;
                f.rules = rules.clone();
                f.source = source.clone();
            };
            match index.and_then(|i| m.fields.get_mut(i)) {
                Some(f) => apply(f),
                None => {
                    let mut f = Field::new(name.clone(), ty.clone());
                    apply(&mut f);
                    m.fields.push(f);
                }
            }
        });
        Ok(())
    });
}

pub fn topic_form(ui: &Shared, index: Option<usize>) {
    let (current, messages, services) = {
        let s = ui.state.borrow();
        (
            index.and_then(|i| s.project.topics.get(i).cloned()),
            s.project
                .messages
                .iter()
                .map(|m| m.name.clone())
                .collect::<Vec<_>>(),
            s.project
                .services
                .iter()
                .map(|s| s.name.clone())
                .collect::<Vec<_>>(),
        )
    };
    if messages.is_empty() {
        super::widgets::say(ui, "Créez d'abord un message : un topic en porte un.");
        return;
    }
    let old = current.as_ref().map(|t| t.name.clone());
    let inputs = vec![
        Input::new("Nom", Kind::Text(old.clone().unwrap_or_default()))
            .help("Renommer met à jour les consommateurs, les mails et les étapes de publication."),
        Input::new(
            "Message",
            Kind::Choice {
                options: messages,
                selected: current.as_ref().map(|t| t.message.clone()),
            },
        ),
        Input::new(
            "Partitions",
            Kind::Number {
                value: current.as_ref().map_or(3, |t| t.partitions as i64),
                min: 1,
                max: 1000,
            },
        ),
        Input::new(
            "Réplication",
            Kind::Number {
                value: current.as_ref().map_or(1, |t| t.replication_factor as i64),
                min: 1,
                max: 10,
            },
        ),
        Input::new(
            "Champ clé (vide : aucun)",
            Kind::Text(
                current
                    .as_ref()
                    .and_then(|t| t.key_field.clone())
                    .unwrap_or_default(),
            ),
        ),
        Input::new(
            "Producteurs",
            Kind::Checks(
                services
                    .into_iter()
                    .map(|s| {
                        let on = current.as_ref().is_some_and(|t| t.producers.contains(&s));
                        (s, on)
                    })
                    .collect(),
            ),
        ),
    ];
    let title = if current.is_some() {
        "Modifier le topic"
    } else {
        "Nouveau topic"
    };
    ask(ui, title, inputs, move |ui, values| {
        let mut v = values.into_iter();
        let name = v.next().map(Value::text).unwrap_or_default();
        let message = v.next().map(Value::text).unwrap_or_default();
        let partitions = v.next().map_or(3, Value::number) as i32;
        let replication = v.next().map_or(1, Value::number) as i16;
        let key = Some(v.next().map(Value::text).unwrap_or_default()).filter(|k| !k.is_empty());
        let producers = v.next().map(Value::checks).unwrap_or_default();
        if old.as_deref() != Some(name.as_str()) {
            if !is_topic_name(&name) {
                return Err("Nom de topic invalide (lettres, chiffres, . _ -).".into());
            }
            if ui.state.borrow().project.topic(&name).is_some() {
                return Err(format!("Le topic {name} existe déjà."));
            }
        }
        edit(ui, |p| {
            if let Some(o) = old.as_deref().filter(|o| *o != name) {
                rename::topic(p, o, &name);
            }
            apply_topic(
                p,
                index,
                name,
                message,
                partitions,
                replication,
                key,
                producers,
            )
        });
        Ok(())
    });
}

#[allow(clippy::too_many_arguments)]
fn apply_topic(
    p: &mut Project,
    index: Option<usize>,
    name: String,
    message: String,
    partitions: i32,
    replication: i16,
    key: Option<String>,
    producers: Vec<String>,
) {
    match index.and_then(|i| p.topics.get_mut(i)) {
        Some(t) => {
            t.message = message;
            t.partitions = partitions;
            t.replication_factor = replication;
            t.key_field = key;
            t.producers = producers;
        }
        None => {
            let n = p.topics.len() as f64;
            p.topics.push(Topic {
                name,
                message,
                partitions,
                replication_factor: replication,
                key_field: key,
                producers,
                position: Position {
                    x: 230.0,
                    y: n * 120.0,
                },
            })
        }
    }
}

/// A schema changes role: an event for a topic, a proto for gRPC, an API schema for a route.
fn message_kind_form(ui: &Shared, index: usize) {
    let Some(m) = ui.state.borrow().project.messages.get(index).cloned() else {
        return;
    };
    let kinds = vec!["Event".to_string(), "Proto".to_string(), "Api".to_string()];
    let inputs = vec![
        Input::new("Nom (PascalCase)", Kind::Text(m.name.clone())).help(
            "Renommer met à jour les champs, topics, RPC, routes et étapes qui y font référence.",
        ),
        Input::new(
            "Rôle",
            Kind::Choice {
                options: kinds,
                selected: Some(format!("{:?}", m.kind)),
            },
        ),
    ];
    let old = m.name.clone();
    ask(ui, "Message", inputs, move |ui, values| {
        let mut v = values.into_iter();
        let name = v.next().map(Value::text).unwrap_or_default();
        if name != old {
            if !is_pascal_ident(&name) {
                return Err("Le nom doit être en PascalCase, par exemple OrderCreated.".into());
            }
            if ui.state.borrow().project.message(&name).is_some() {
                return Err(format!("Le message {name} existe déjà."));
            }
        }
        let kind = match v.next().map(Value::text).as_deref() {
            Some("Api") => MessageKind::Api,
            Some("Proto") => MessageKind::Proto,
            _ => MessageKind::Event,
        };
        edit(ui, |p| {
            if name != old {
                rename::message(p, &old, &name);
            }
            if let Some(m) = p.messages.get_mut(index) {
                m.kind = kind;
            }
        });
        Ok(())
    });
}
