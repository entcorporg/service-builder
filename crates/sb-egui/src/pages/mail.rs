//! The two ready-made services, in the inspector: the mail service (the list of its mails, each opening the mail editor,
//! and the sending settings of a mail, shown in the editor) and the real-time one (five names). Everything is edited in
//! place: no form opens over the window.

use eframe::egui::{self, Frame, Margin, RichText, Sense, Stroke, Ui};
use sb_model::{
    Consumer, DedupFallback, FieldType, MailBlock, MailRule, MailTemplate, Project, RealtimeConfig,
};

use super::common::{choice, commit_entry, empty, field, icon_btn, section, separator};
use crate::app::App;
use crate::theme::*;

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
        sender: None,
    }
}

/// A first mail, composed as blocks (the author writes no HTML): a heading, and a paragraph with a field of the event.
pub fn starter_template(p: &Project, topic: &str, rule: &MailRule) -> MailTemplate {
    let shown = mail_fields(p, topic)
        .into_iter()
        .find(|f| *f != rule.recipient && *f != rule.dedup_field);
    let line = shown.map_or_else(|| topic.to_string(), |f| format!("{{{{ {f} }}}}"));
    let subject = format!("Notification : {topic}");
    let mut design = super::mailcanvas::starter_design(&subject);
    design.blocks = vec![MailBlock::heading("Bonjour,"), MailBlock::paragraph(&line)];
    MailTemplate {
        event_type: topic.into(),
        subject,
        html: String::new(),
        text: String::new(),
        design: Some(design),
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

/// Adds the mail of `topic` to the mail service `si` and opens it in the editor.
pub fn add_mail(app: &mut App, si: usize, topic: &str) {
    app.edit(|p| add_mail_event(p, si, topic));
    super::mailcanvas::open(app, si, topic);
}

/// Removes the mail of `topic`: the consumer that reads the event and its template.
pub fn delete_mail(app: &mut App, si: usize, topic: &str) {
    let topic = topic.to_string();
    app.edit(move |p| {
        if let Some(svc) = p.services.get_mut(si) {
            svc.consumers.retain(|c| c.topic != topic);
            svc.templates.retain(|t| t.event_type != topic);
        }
    });
}

/// The topics that have no mail yet in service `si`.
pub fn free_topics(p: &Project, si: usize) -> Vec<String> {
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
}

/// What the list says of a mail: its subject, and how it is written.
pub fn mail_summary(p: &Project, si: usize, topic: &str) -> String {
    let template = p.services[si]
        .templates
        .iter()
        .find(|t| t.event_type == topic);
    let subject = template.map_or("sans sujet", |t| t.subject.as_str());
    let how = if template.is_some_and(|t| t.design.is_some()) {
        "blocs"
    } else {
        "HTML"
    };
    format!("{subject} · {how}")
}

/// The drop-down that adds a mail for a topic that has none (nothing when every topic has its mail).
pub fn add_mail_choice(app: &mut App, ui: &mut Ui, p: &Project, si: usize) {
    let free = free_topics(p, si);
    if free.is_empty() {
        if p.topics.is_empty() {
            empty(
                ui,
                "Créez d’abord un topic : chaque mail répond à un événement.",
            );
        }
        return;
    }
    let prompt = "+ Ajouter un mail…".to_string();
    let mut options = vec![prompt.clone()];
    options.extend(free);
    if let Some(topic) = choice(ui, ("mail-add", si), &options, &prompt)
        && topic != prompt
    {
        add_mail(app, si, &topic);
    }
}

/// The section of a mail service in the inspector: one row per mail, which opens the editor on it.
pub fn mails_section(app: &mut App, ui: &mut Ui, p: &Project, si: usize) {
    let svc = &p.services[si];
    section(
        ui,
        "Mails",
        "publish",
        |_| {},
        |ui| {
            if svc.consumers.is_empty() {
                empty(
                    ui,
                    "Aucun mail : chaque mail répond à un événement, ajoutez-en un.",
                );
            }
            for c in &svc.consumers {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    let w = ui.available_width() - 30.0;
                    let frame = Frame::new()
                        .fill(PANEL2)
                        .stroke(Stroke::new(1.0, BORDER))
                        .corner_radius(8)
                        .inner_margin(Margin::symmetric(10, 7))
                        .show(ui, |ui| {
                            ui.set_width(w - 22.0);
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 2.0;
                                ui.add(
                                    egui::Label::new(RichText::new(&c.topic).font(mono(SM)))
                                        .truncate(),
                                );
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(mail_summary(p, si, &c.topic))
                                            .size(SM)
                                            .color(MUTED),
                                    )
                                    .truncate(),
                                );
                            });
                        });
                    let hit = ui
                        .interact(
                            frame.response.rect,
                            ui.make_persistent_id(("mail-open", si, &c.topic)),
                            Sense::click(),
                        )
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .on_hover_text("Ouvrir l’éditeur du mail");
                    if hit.hovered() {
                        ui.painter().rect_stroke(
                            frame.response.rect,
                            8,
                            Stroke::new(1.0, ACCENT),
                            egui::StrokeKind::Inside,
                        );
                    }
                    if hit.clicked() {
                        super::mailcanvas::open(app, si, &c.topic);
                    }
                    if icon_btn(
                        ui,
                        "trash",
                        &format!("Supprimer le mail de {}", c.topic),
                        true,
                    )
                    .clicked()
                    {
                        delete_mail(app, si, &c.topic);
                    }
                });
            }
            add_mail_choice(app, ui, p, si);
        },
    );
}

/// The sending settings of the mail of consumer `ci` (address, identity, sender, subject), applied when left.
pub fn rule_settings(app: &mut App, ui: &mut Ui, p: &Project, si: usize, ci: usize) {
    let Some(c) = p.services[si].consumers.get(ci) else {
        return;
    };
    let topic = c.topic.clone();
    let fields = mail_fields(p, &topic);
    let rule = c.mail.clone().unwrap_or_else(|| default_rule(p, &topic));
    let subject = p.services[si]
        .templates
        .iter()
        .find(|t| t.event_type == topic)
        .map(|t| t.subject.clone())
        .unwrap_or_default();
    // Every change rewrites the rule as a whole, so that a mail without one gets the default first.
    let set_rule = |app: &mut App, change: &dyn Fn(&mut MailRule)| {
        let mut new = rule.clone();
        change(&mut new);
        app.edit(|p| {
            if let Some(c) = p.services[si].consumers.get_mut(ci) {
                c.mail = Some(new);
            }
        });
    };
    if fields.is_empty() {
        empty(
            ui,
            "L’événement n’a aucun champ texte : ajoutez-en un au message pour y lire l’adresse.",
        );
    }
    if let Some(v) = field(
        ui,
        "Sujet",
        "Les variables s’écrivent {{ champ }}.",
        |ui| commit_entry(ui, ("mail-subject", si, &topic), &subject, false, "", false),
    ) {
        app.edit(|p| {
            if let Some(t) = p.services[si]
                .templates
                .iter_mut()
                .find(|t| t.event_type == topic)
            {
                t.subject = v;
            }
        });
    }
    if let Some(v) = field(
        ui,
        "Adresse",
        "Le champ de l’événement qui porte l’adresse du destinataire.",
        |ui| choice(ui, ("mail-to", si, &topic), &fields, &rule.recipient),
    ) {
        set_rule(app, &|r| r.recipient = v.clone());
    }
    if let Some(v) = field(
        ui,
        "Facultative",
        "Sans adresse, l’événement est ignoré au lieu d’être une erreur.",
        |ui| crate::widgets::check(ui, rule.optional_recipient, "L’adresse peut manquer"),
    ) {
        set_rule(app, &|r| r.optional_recipient = v);
    }
    if let Some(v) = field(
        ui,
        "Identité",
        "Le même événement redélivré par Kafka ne part qu’une fois.",
        |ui| choice(ui, ("mail-id", si, &topic), &fields, &rule.dedup_field),
    ) {
        set_rule(app, &|r| r.dedup_field = v.clone());
    }
    let labels: Vec<String> = FALLBACKS.iter().map(|(_, l)| (*l).to_string()).collect();
    let current = FALLBACKS
        .iter()
        .find(|(f, _)| *f == rule.dedup_fallback)
        .map_or("", |(_, l)| *l);
    if let Some(v) = field(ui, "Sans identité", "", |ui| {
        choice(ui, ("mail-fallback", si, &topic), &labels, current)
    }) && let Some((f, _)) = FALLBACKS.iter().find(|(_, l)| *l == v)
    {
        set_rule(app, &|r| r.dedup_fallback = *f);
    }
    let with_none = |none: &str| {
        let mut v = vec![none.to_string()];
        v.extend(fields.iter().cloned());
        v
    };
    for (label, hint, none, current, company) in [
        (
            "Entreprise",
            "Un champ avec un nom d’entreprise : l’expéditeur s’affiche « entreprise via <service> ».",
            "(aucune)",
            &rule.sender_company_via,
            true,
        ),
        ("Répondre à", "", "(expéditeur)", &rule.reply_to, false),
    ] {
        let selected = current.clone().unwrap_or_else(|| none.to_string());
        if let Some(v) = field(ui, label, hint, |ui| {
            choice(
                ui,
                ("mail-opt", si, &topic, company),
                &with_none(none),
                &selected,
            )
        }) {
            let v = Some(v).filter(|v| v != none);
            set_rule(app, &|r| {
                if company {
                    r.sender_company_via = v.clone();
                } else {
                    r.reply_to = v.clone();
                }
            });
        }
    }
}

/// The section of a real-time service in the inspector: the five names it uses, applied when left.
pub fn realtime_section(app: &mut App, ui: &mut Ui, p: &Project, si: usize) {
    let cfg = p.services[si].realtime.clone().unwrap_or_default();
    separator(ui);
    let changed = section(
        ui,
        "Flux d’événements (SSE)",
        "publish",
        |_| {},
        |ui| {
            let rows: [(&str, &str, &str); 5] = [
                (
                    "Canal",
                    "Le canal DragonflyDB (ou Valkey, ou Redis) où l’API publie.",
                    &cfg.channel,
                ),
                (
                    "Chemin",
                    "Le chemin du flux SSE sur le port HTTP.",
                    &cfg.path,
                ),
                (
                    "Cookie",
                    "Le cookie qui porte le jeton d’accès.",
                    &cfg.access_cookie,
                ),
                (
                    "Émetteur",
                    "L’émetteur (iss) attendu du jeton.",
                    &cfg.issuer,
                ),
                (
                    "Sessions",
                    "Le préfixe des clés de session.",
                    &cfg.session_key_prefix,
                ),
            ];
            let mut changed = None;
            for (i, (label, hint, value)) in rows.into_iter().enumerate() {
                if let Some(v) = field(ui, label, hint, |ui| {
                    commit_entry(ui, ("rt", si, i), value, true, "", false)
                }) {
                    changed = Some((i, v));
                }
            }
            changed
        },
    );
    if let Some((i, v)) = changed {
        let mut new: RealtimeConfig = cfg;
        let slot = match i {
            0 => &mut new.channel,
            1 => &mut new.path,
            2 => &mut new.access_cookie,
            3 => &mut new.issuer,
            _ => &mut new.session_key_prefix,
        };
        *slot = v;
        app.edit(|p| p.services[si].realtime = Some(new));
    }
}
