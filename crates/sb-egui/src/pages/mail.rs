//! The two ready-made services: the mail service (events it mails, and the mails) and the real-time one (five names).
//! PORT TARGET: `sb-gtk/src/uis/mail.rs` (owner: inspector agent). The pure helpers are already ported.

use eframe::egui::Ui;
use sb_model::{Consumer, DedupFallback, FieldType, MailRule, MailTemplate, Project};

use crate::app::App;

#[allow(dead_code)]
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
    let _ = (app, p, si);
    super::common::empty(ui, "À porter : mail.rs (draw_mail)");
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

/// The page of a real-time service, inside the inspector.
pub fn draw_realtime(app: &mut App, ui: &mut Ui, p: &Project, si: usize) {
    let _ = (app, p, si);
    super::common::empty(ui, "À porter : mail.rs (draw_realtime)");
}
