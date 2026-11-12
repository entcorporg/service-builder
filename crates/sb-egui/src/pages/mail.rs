//! The two ready-made services: the mail service (events it mails, and the mails) and the real-time one (five names).

use eframe::egui::Ui;
use sb_model::{
    Consumer, DedupFallback, FieldType, MailRule, MailTemplate, Project, RealtimeConfig,
};

use super::common::{empty, list_card};
use crate::app::App;
use crate::form::{Input, Kind, Value, ask};
use crate::widgets::add_row;

const FALLBACKS: [(DedupFallback, &str); 3] = [
    (
        DedupFallback::TopicAndRecipient,
        "un mail par sujet et adresse",
    ),
    (DedupFallback::Recipient, "un mail par adresse"),
    (DedupFallback::Reject, "refuser l'événement"),
];

/// The text fields of the event of a topic: where a mail reads the address, the identity and the sender from.
pub fn mail_fields(p: &Project, topic: &str) -> Vec<String> {
    let message = p.topic(topic).map(|t| t.message.as_str());
    message
        .and_then(|m| p.message(m))
        .map(|m| {
            m.fields
                .iter()
                .filter(|f| !f.repeated && matches!(f.ty, FieldType::String | FieldType::Uuid))
                .map(|f| f.name.clone())
                .collect()
        })
        .unwrap_or_default()
}

/// A first rule: the field that looks like an address, and one that looks like an identity.
pub fn default_rule(p: &Project, topic: &str) -> MailRule {
    let fields = mail_fields(p, topic);
    let has = |needle: &str| fields.iter().find(|f| f.to_lowercase().contains(needle));
    let recipient = has("mail")
        .or_else(|| has("address"))
        .or_else(|| has("recipient"))
        .or(fields.first())
        .cloned()
        .unwrap_or_default();
    let dedup_field = fields
        .iter()
        .find(|f| matches!(f.to_lowercase().as_str(), "event_id" | "eventid"))
        .or_else(|| fields.iter().find(|f| f.ends_with("_id") || *f == "id"))
        .or_else(|| fields.iter().find(|f| **f != recipient))
        .cloned()
        .unwrap_or_else(|| recipient.clone());
    MailRule {
        recipient,
        optional_recipient: false,
        dedup_field,
        dedup_fallback: DedupFallback::TopicAndRecipient,
        sender_company_via: None,
        reply_to: None,
    }
}

pub fn starter_template(p: &Project, topic: &str, rule: &MailRule) -> MailTemplate {
    let shown = mail_fields(p, topic)
        .into_iter()
        .find(|f| *f != rule.recipient && *f != rule.dedup_field);
    let line = shown.map_or_else(|| topic.to_string(), |f| format!("{{{{ {f} }}}}"));
    MailTemplate {
        event_type: topic.into(),
        subject: format!("Notification : {topic}"),
        html: format!("<p>Bonjour,</p>\n<p>{line}</p>\n"),
        text: format!("Bonjour,\n\n{line}\n"),
    }
}

pub fn consumer_name(topic: &str, taken: &[String]) -> String {
    let base = format!(
        "on_{}",
        topic
            .to_lowercase()
            .replace(|c: char| !c.is_ascii_alphanumeric(), "_")
    );
    let mut name = base.clone();
    let mut i = 2;
    while taken.contains(&name) {
        name = format!("{base}{i}");
        i += 1;
    }
    name
}

/// The page of a mail service, inside the inspector: the events it mails and their mails.
pub fn draw_mail(app: &mut App, ui: &mut Ui, p: &Project, si: usize) {
    let svc = &p.services[si];
    if add_row(ui, "Ajouter un mail").clicked() {
        add_mail(app, si);
    }
    for (ci, c) in svc.consumers.iter().enumerate() {
        let template = svc.templates.iter().find(|t| t.event_type == c.topic);
        let subject = template.map_or("sans sujet", |t| t.subject.as_str());
        match list_card(
            ui,
            &format!("Mail de {}", c.topic),
            subject,
            0.0,
            &["Modifier", "Supprimer"],
        ) {
            Some(0) => mail_form(app, si, ci),
            Some(1) => {
                let topic = c.topic.clone();
                app.edit(move |p| {
                    let svc = &mut p.services[si];
                    if ci < svc.consumers.len() {
                        svc.consumers.remove(ci);
                    }
                    svc.templates.retain(|t| t.event_type != topic);
                });
            }
            _ => {}
        }
    }
    if svc.consumers.is_empty() {
        ui.label(eframe::egui::RichText::new("Aucun mail").strong());
        empty(ui, "Chaque mail répond à un événement : ajoutez-en un.");
    }
}

/// A mail for an event: the consumer that reads it, with a first rule and a first template.
pub fn add_mail_event(p: &mut Project, si: usize, topic: &str) {
    let rule = default_rule(p, topic);
    let template = starter_template(p, topic, &rule);
    let taken: Vec<String> = p.services[si]
        .consumers
        .iter()
        .map(|c| c.name.clone())
        .collect();
    let svc = &mut p.services[si];
    svc.consumers.push(Consumer {
        name: consumer_name(topic, &taken),
        topic: topic.into(),
        group_id: None,
        idempotent_on: None,
        mail: Some(rule),
        steps: vec![],
    });
    svc.templates.push(template);
}

fn add_mail(app: &mut App, si: usize) {
    let free: Vec<String> = {
        let p = app.project();
        let taken: Vec<&str> = p.services[si]
            .consumers
            .iter()
            .map(|c| c.topic.as_str())
            .collect();
        p.topics
            .iter()
            .filter(|t| !taken.contains(&t.name.as_str()))
            .map(|t| t.name.clone())
            .collect()
    };
    if free.is_empty() {
        return app.say("Aucun topic sans mail : créez d'abord un topic.");
    }
    let inputs = vec![Input::new(
        "Événement à envoyer par mail",
        Kind::Choice {
            selected: free.first().cloned(),
            options: free,
        },
    )];
    ask(app, "Nouveau mail", inputs, move |app, values| {
        let topic = values
            .into_iter()
            .next()
            .map(Value::text)
            .unwrap_or_default();
        let ci = app.project().services[si].consumers.len();
        app.edit(|p| {
            let rule = default_rule(p, &topic);
            let template = starter_template(p, &topic, &rule);
            let taken: Vec<String> = p.services[si]
                .consumers
                .iter()
                .map(|c| c.name.clone())
                .collect();
            let svc = &mut p.services[si];
            svc.consumers.push(Consumer {
                name: consumer_name(&topic, &taken),
                topic,
                group_id: None,
                idempotent_on: None,
                mail: Some(rule),
                steps: vec![],
            });
            svc.templates.push(template);
        });
        mail_form(app, si, ci);
        Ok(())
    });
}

fn mail_form(app: &mut App, si: usize, ci: usize) {
    let (consumer, template, fields) = {
        let p = app.project();
        let Some(c) = p.services[si].consumers.get(ci).cloned() else {
            return;
        };
        let t = p.services[si]
            .templates
            .iter()
            .find(|t| t.event_type == c.topic)
            .cloned();
        let f = mail_fields(&p, &c.topic);
        (c, t, f)
    };
    let rule = consumer
        .mail
        .clone()
        .unwrap_or_else(|| default_rule(&app.project(), &consumer.topic));
    let template =
        template.unwrap_or_else(|| starter_template(&app.project(), &consumer.topic, &rule));
    let with_none = |none: &str| {
        let mut v = vec![none.to_string()];
        v.extend(fields.iter().cloned());
        v
    };
    let labels: Vec<String> = FALLBACKS.iter().map(|(_, l)| (*l).to_string()).collect();
    let current_label = FALLBACKS
        .iter()
        .find(|(f, _)| *f == rule.dedup_fallback)
        .map(|(_, l)| (*l).to_string());
    let inputs = vec![
        Input::new("Événement", Kind::Fixed(consumer.topic.clone())),
        Input::new(
            "Adresse",
            Kind::Choice {
                options: fields.clone(),
                selected: Some(rule.recipient.clone()),
            },
        )
        .help("Le champ de l'événement qui porte l'adresse du destinataire."),
        Input::new("Adresse facultative", Kind::Check(rule.optional_recipient))
            .help("Sans adresse, l'événement est ignoré au lieu d'être une erreur."),
        Input::new(
            "Identité",
            Kind::Choice {
                options: fields.clone(),
                selected: Some(rule.dedup_field.clone()),
            },
        )
        .help("Le même événement redélivré par Kafka ne part qu'une fois."),
        Input::new(
            "Sans identité",
            Kind::Choice {
                options: labels,
                selected: current_label,
            },
        ),
        Input::new(
            "Entreprise",
            Kind::Choice {
                options: with_none("(aucune)"),
                selected: rule.sender_company_via.clone().or(Some("(aucune)".into())),
            },
        )
        .help("Un champ avec un nom d'entreprise : l'expéditeur s'affiche « entreprise via <service> »."),
        Input::new(
            "Répondre à",
            Kind::Choice {
                options: with_none("(expéditeur)"),
                selected: rule.reply_to.clone().or(Some("(expéditeur)".into())),
            },
        ),
        Input::new("Sujet", Kind::Text(template.subject.clone())),
        Input::new("HTML", Kind::Multiline(template.html.clone())),
        Input::new("Texte", Kind::Multiline(template.text.clone())),
    ];
    let topic = consumer.topic.clone();
    ask(app, "Mail", inputs, move |app, values| {
        let mut v = values.into_iter();
        let mut next = || v.next();
        let _event = next();
        let recipient = next().map(Value::text).unwrap_or_default();
        let optional_recipient = next().is_some_and(Value::check);
        let dedup_field = next().map(Value::text).unwrap_or_default();
        let fallback_label = next().map(Value::text).unwrap_or_default();
        let company = next().map(Value::text).unwrap_or_default();
        let reply = next().map(Value::text).unwrap_or_default();
        let subject = next().map(Value::text).unwrap_or_default();
        let html = next().map(Value::text).unwrap_or_default();
        let text = next().map(Value::text).unwrap_or_default();
        if recipient.is_empty() || dedup_field.is_empty() {
            return Err("L'événement n'a aucun champ texte : ajoutez-en un au message.".into());
        }
        let dedup_fallback = FALLBACKS
            .iter()
            .find(|(_, l)| *l == fallback_label)
            .map_or(DedupFallback::default(), |(f, _)| *f);
        let topic = topic.clone();
        app.edit(|p| {
            let svc = &mut p.services[si];
            if let Some(c) = svc.consumers.get_mut(ci) {
                c.mail = Some(MailRule {
                    recipient,
                    optional_recipient,
                    dedup_field,
                    dedup_fallback,
                    sender_company_via: Some(company).filter(|c| !c.starts_with('(')),
                    reply_to: Some(reply).filter(|c| !c.starts_with('(')),
                });
            }
            let new = MailTemplate {
                event_type: topic.clone(),
                subject,
                html,
                text,
            };
            match svc.templates.iter_mut().find(|t| t.event_type == topic) {
                Some(t) => *t = new,
                None => svc.templates.push(new),
            }
        });
        Ok(())
    });
}

/// The page of a real-time service, inside the inspector.
pub fn draw_realtime(app: &mut App, ui: &mut Ui, p: &Project, si: usize) {
    let cfg = p.services[si].realtime.clone().unwrap_or_default();
    let summary = format!(
        "canal {} · chemin {} · cookie {} · émetteur {} · préfixe de session {}",
        cfg.channel, cfg.path, cfg.access_cookie, cfg.issuer, cfg.session_key_prefix
    );
    if list_card(ui, "Flux d'événements (SSE)", &summary, 0.0, &["Modifier"]) == Some(0) {
        realtime_form(app, si);
    }
}

fn realtime_form(app: &mut App, si: usize) {
    let cfg = app.project().services[si]
        .realtime
        .clone()
        .unwrap_or_default();
    let inputs = vec![
        Input::new("Canal", Kind::Text(cfg.channel))
            .help("Le canal DragonflyDB (ou Valkey, ou Redis) où l'API publie."),
        Input::new("Chemin du flux", Kind::Text(cfg.path)),
        Input::new("Cookie du jeton", Kind::Text(cfg.access_cookie)),
        Input::new("Émetteur du jeton", Kind::Text(cfg.issuer)),
        Input::new(
            "Préfixe des clés de session",
            Kind::Text(cfg.session_key_prefix),
        ),
    ];
    ask(app, "Service temps réel", inputs, move |app, values| {
        let mut v = values.into_iter().map(Value::text);
        let mut next = || v.next().unwrap_or_default();
        let new = RealtimeConfig {
            channel: next(),
            path: next(),
            access_cookie: next(),
            issuer: next(),
            session_key_prefix: next(),
        };
        if [
            &new.channel,
            &new.path,
            &new.access_cookie,
            &new.issuer,
            &new.session_key_prefix,
        ]
        .iter()
        .any(|s| s.is_empty())
        {
            return Err("Aucun de ces champs ne peut être vide.".into());
        }
        app.edit(|p| p.services[si].realtime = Some(new));
        Ok(())
    });
}
