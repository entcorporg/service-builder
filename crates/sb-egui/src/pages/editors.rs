//! The forms of the services, topics and messages (the list pages of the GTK version have no tab: not ported).
//!
//! Names of services, messages and topics can be changed (`sb_app::rename` rewrites what refers to them).

use sb_app::rename;
use sb_model::names::{is_pascal_ident, is_snake_ident, is_topic_name};
use sb_model::{
    Field, FieldType, Message, MessageKind, Position, Project, RequestSource, Rule, Service,
    ServiceKind, Topic,
};

use crate::app::App;
use crate::form::{Input, Kind, Value, ask};

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

const SERVICE_KINDS: [(&str, ServiceKind); 3] = [
    ("Service standard", ServiceKind::Standard),
    ("Service mail (MTA prêt à l'emploi)", ServiceKind::Mta),
    (
        "Service temps réel (SSE prêt à l'emploi)",
        ServiceKind::Realtime,
    ),
];

/// The modal form of a service (new when `index` is `None`).
pub fn service_form(app: &mut App, index: Option<usize>) {
    let project = app.project();
    let current = index.and_then(|i| project.services.get(i).cloned());
    let number = |v: i64| Kind::Number {
        value: v,
        min: 0,
        max: 65535,
    };
    // The next free gRPC port, as the web editor chose it.
    let next_grpc = i64::from(
        project
            .services
            .iter()
            .map(|s| s.port)
            .max()
            .unwrap_or(0)
            .max(50050),
    ) + 1;
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
    ask(app, title, inputs, move |app, values| {
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
            if app.project().service(&name).is_some() {
                return Err(format!("Le service {name} existe déjà."));
            }
        }
        if port == 0 {
            return Err("Le port gRPC est obligatoire.".into());
        }
        app.edit(|p| {
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

/// The modal form of a new message.
pub fn message_form(app: &mut App) {
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
    ask(app, "Nouveau message", inputs, |app, values| {
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
        if app.project().message(&name).is_some() {
            return Err(format!("Le message {name} existe déjà."));
        }
        app.edit(|p| {
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

/// The modal form of a field of a message (new when `index` is `None`).
pub fn field_form(app: &mut App, message: usize, index: Option<usize>) {
    let (current, mut types, api) = {
        let p = app.project();
        let current = index.and_then(|i| p.messages.get(message)?.fields.get(i).cloned());
        let mut types: Vec<String> = SCALARS.iter().map(|(n, _)| (*n).to_string()).collect();
        types.extend(p.messages.iter().map(|m| m.name.clone()));
        let api = p
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
    ask(app, title, inputs, move |app, values| {
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
        let taken = app.project().messages.get(message).is_some_and(|m| {
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
        app.edit(|p| {
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

/// The modal form of a topic (new when `index` is `None`).
pub fn topic_form(app: &mut App, index: Option<usize>) {
    let (current, messages, services) = {
        let p = app.project();
        (
            index.and_then(|i| p.topics.get(i).cloned()),
            p.messages
                .iter()
                .map(|m| m.name.clone())
                .collect::<Vec<_>>(),
            p.services
                .iter()
                .map(|s| s.name.clone())
                .collect::<Vec<_>>(),
        )
    };
    if messages.is_empty() {
        app.say("Créez d'abord un message : un topic en porte un.");
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
    ask(app, title, inputs, move |app, values| {
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
            if app.project().topic(&name).is_some() {
                return Err(format!("Le topic {name} existe déjà."));
            }
        }
        app.edit(|p| {
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
pub fn message_kind_form(app: &mut App, index: usize) {
    let Some(m) = app.project().messages.get(index).cloned() else {
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
    ask(app, "Message", inputs, move |app, values| {
        let mut v = values.into_iter();
        let name = v.next().map(Value::text).unwrap_or_default();
        if name != old {
            if !is_pascal_ident(&name) {
                return Err("Le nom doit être en PascalCase, par exemple OrderCreated.".into());
            }
            if app.project().message(&name).is_some() {
                return Err(format!("Le message {name} existe déjà."));
            }
        }
        let kind = match v.next().map(Value::text).as_deref() {
            Some("Api") => MessageKind::Api,
            Some("Proto") => MessageKind::Proto,
            _ => MessageKind::Event,
        };
        app.edit(|p| {
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
