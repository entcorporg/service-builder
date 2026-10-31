//! The ready-made mail service (`kind: "mta"`): fixed code, with two inputs from the project, the events it mails and
//! the mails.
//!
//! The queue, the worker, DKIM signing, SMTP delivery, the bounce receiver, health and metrics are the files of
//! `templates/mta`, copied as they are. The project decides which events are mailed (`mta_rules.rs`, `core/consumer.rs`)
//! and what the mails say (`seed.rs`, inserted into `mail_templates` on start-up).
//!
//! The Rust of that fixed code is in French, as it was written: it is not edited by the generator beyond the few
//! `::PLACEHOLDERS::` below.

use std::collections::BTreeSet;

use sb_model::names::{snake, topic_const};
use sb_model::{DedupFallback, MailRule, Project, Service};

use crate::{CodegenError, GeneratedFile};

/// The fixed files of the service, by path inside its directory.
const FIXED: &[(&str, &str)] = &[
    (
        "migrations/20260302000001_mail_queue.sql",
        include_str!("../templates/mta/migrations/20260302000001_mail_queue.sql"),
    ),
    (
        "migrations/20260302000002_bounces.sql",
        include_str!("../templates/mta/migrations/20260302000002_bounces.sql"),
    ),
    (
        "migrations/20260614000001_dedup_key.sql",
        include_str!("../templates/mta/migrations/20260614000001_dedup_key.sql"),
    ),
    (
        "migrations/20261027000001_mail_templates.sql",
        include_str!("../templates/mta/migrations/20261027000001_mail_templates.sql"),
    ),
    (
        "src/bounce/mod.rs",
        include_str!("../templates/mta/src/bounce/mod.rs"),
    ),
    (
        "src/bounce/server.rs",
        include_str!("../templates/mta/src/bounce/server.rs"),
    ),
    (
        "src/core/config.rs",
        include_str!("../templates/mta/src/core/config.rs"),
    ),
    (
        "src/core/consumer.rs",
        include_str!("../templates/mta/src/core/consumer.rs"),
    ),
    (
        "src/core/mod.rs",
        include_str!("../templates/mta/src/core/mod.rs"),
    ),
    (
        "src/handlers/health.rs",
        include_str!("../templates/mta/src/handlers/health.rs"),
    ),
    (
        "src/handlers/metrics.rs",
        include_str!("../templates/mta/src/handlers/metrics.rs"),
    ),
    (
        "src/handlers/mod.rs",
        include_str!("../templates/mta/src/handlers/mod.rs"),
    ),
    ("src/lib.rs", include_str!("../templates/mta/src/lib.rs")),
    ("src/main.rs", include_str!("../templates/mta/src/main.rs")),
    (
        "src/message/mod.rs",
        include_str!("../templates/mta/src/message/mod.rs"),
    ),
    (
        "src/metrics.rs",
        include_str!("../templates/mta/src/metrics.rs"),
    ),
    (
        "src/middlewares/metrics.rs",
        include_str!("../templates/mta/src/middlewares/metrics.rs"),
    ),
    (
        "src/middlewares/mod.rs",
        include_str!("../templates/mta/src/middlewares/mod.rs"),
    ),
    (
        "src/mta/dkim.rs",
        include_str!("../templates/mta/src/mta/dkim.rs"),
    ),
    (
        "src/mta/dns.rs",
        include_str!("../templates/mta/src/mta/dns.rs"),
    ),
    (
        "src/mta/mod.rs",
        include_str!("../templates/mta/src/mta/mod.rs"),
    ),
    (
        "src/mta/queue_worker.rs",
        include_str!("../templates/mta/src/mta/queue_worker.rs"),
    ),
    (
        "src/mta/smtp.rs",
        include_str!("../templates/mta/src/mta/smtp.rs"),
    ),
    (
        "src/mta_rules.rs",
        include_str!("../templates/mta/src/mta_rules.rs"),
    ),
    (
        "src/routes/mod.rs",
        include_str!("../templates/mta/src/routes/mod.rs"),
    ),
    ("src/seed.rs", include_str!("../templates/mta/src/seed.rs")),
    (
        "src/template/mod.rs",
        include_str!("../templates/mta/src/template/mod.rs"),
    ),
];

pub fn generate(project: &Project, service: &Service) -> Result<Vec<GeneratedFile>, CodegenError> {
    let dir = crate::service::dir(service);
    let db_env = format!("{}_DATABASE_URL", sb_model::names::shouty(&service.name));
    let db_default = format!(
        "postgres://postgres:postgres@localhost:5432/{}",
        snake(&service.name)
    );
    let http_port = service
        .http_port
        .ok_or_else(|| CodegenError::Internal("a mail service needs an HTTP port".into()))?;

    let mut files = vec![GeneratedFile::new(
        format!("{dir}/Cargo.toml"),
        cargo_toml(service),
    )];
    for (path, content) in FIXED {
        let content = match *path {
            "src/core/config.rs" => content
                .replace("::SERVER_PORT::", &http_port.to_string())
                .replace("::DATABASE_ENV::", &db_env)
                .replace("::DATABASE_DEFAULT::", &db_default),
            "src/mta_rules.rs" => content.replace("::EVENT_RULES::", &event_rules(service)),
            "src/core/consumer.rs" => content
                .replace("::TOPICS::", &topics_const(project, service))
                .replace("::SPAWN::", &spawn_consumers(project, service))
                .replace("::TESTS::", &field_tests(project, service)),
            "src/seed.rs" => content.replace("::SEED::", &seed(service)),
            _ => (*content).to_string(),
        };
        files.push(GeneratedFile::new(format!("{dir}/{path}"), content));
    }
    Ok(files)
}

/// The messages of the events a mail service mails, and the ones nested in them: they are read as JSON, so the contracts
/// derive serde for them.
pub fn json_messages(project: &Project) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let mut todo: Vec<&str> = project
        .services
        .iter()
        .filter(|s| s.kind == sb_model::ServiceKind::Mta)
        .flat_map(|s| &s.consumers)
        .filter_map(|c| project.topic(&c.topic))
        .map(|t| t.message.as_str())
        .collect();
    while let Some(name) = todo.pop() {
        if !found.insert(name.to_string()) {
            continue;
        }
        if let Some(m) = project.message(name) {
            for f in &m.fields {
                if let sb_model::FieldType::Message { name } = &f.ty {
                    todo.push(name);
                }
            }
        }
    }
    found
}

fn cargo_toml(service: &Service) -> String {
    format!(
        "# @generated by service-builder. Do not edit: changes are overwritten on export.\n\
         [package]\n\
         name = \"{}\"\n\
         version.workspace = true\n\
         edition.workspace = true\n\
         \n\
         [lib]\n\
         name = \"mta_service\"\n\
         path = \"src/lib.rs\"\n\
         \n\
         [dependencies]\n\
         anyhow.workspace = true\n\
         async-trait.workspace = true\n\
         axum = {{ workspace = true, features = [\"matched-path\"] }}\n\
         chrono.workspace = true\n\
         contracts = {{ path = \"../../contracts\" }}\n\
         dotenvy.workspace = true\n\
         eda-runtime = {{ path = \"../../runtime\" }}\n\
         futures.workspace = true\n\
         hickory-resolver.workspace = true\n\
         lettre.workspace = true\n\
         mail-auth.workspace = true\n\
         native-tls.workspace = true\n\
         prometheus.workspace = true\n\
         rand.workspace = true\n\
         rustls.workspace = true\n\
         rustls-pki-types.workspace = true\n\
         serde.workspace = true\n\
         serde_json.workspace = true\n\
         sha2.workspace = true\n\
         sqlx = {{ workspace = true, features = [\"uuid\", \"chrono\", \"json\", \"migrate\", \"macros\"] }}\n\
         subtle.workspace = true\n\
         tera.workspace = true\n\
         tokio = {{ workspace = true, features = [\"net\", \"io-util\", \"fs\"] }}\n\
         tokio-native-tls.workspace = true\n\
         tower-http = {{ workspace = true, features = [\"trace\"] }}\n\
         tracing.workspace = true\n\
         tracing-subscriber.workspace = true\n\
         uuid = {{ workspace = true, features = [\"serde\"] }}\n\
         \n\
         [dev-dependencies]\n\
         base64.workspace = true\n\
         rsa.workspace = true\n\
         tokio = {{ workspace = true, features = [\"test-util\"] }}\n\
         tower.workspace = true\n",
        crate::service::crate_name(service)
    )
}

fn consumers<'a>(
    project: &'a Project,
    service: &'a Service,
) -> impl Iterator<Item = (&'a sb_model::Consumer, &'a sb_model::Topic, &'a MailRule)> {
    service
        .consumers
        .iter()
        .filter_map(|c| Some((c, project.topic(&c.topic)?, c.mail.as_ref()?)))
}

fn topics_const(project: &Project, service: &Service) -> String {
    let list: Vec<String> = consumers(project, service)
        .map(|(_, t, _)| format!("topics::{}", topic_const(&t.name)))
        .collect();
    format!(
        "/// Les topics écoutés.\npub const TOPICS: &[&str] = &[{}];",
        list.join(", ")
    )
}

fn spawn_consumers(project: &Project, service: &Service) -> String {
    let mut out = String::new();
    for (c, topic, _) in consumers(project, service) {
        let konst = topic_const(&topic.name);
        out.push_str(&format!(
            "    // `{topic_name}` ({message}): un mail par événement\n    {{\n        let (brokers, db) = (brokers.to_string(), db.clone());\n        tasks.push(tokio::spawn(async move {{\n            let result = eda_runtime::consume::<pb::{message}, _, _>(&brokers, {group:?}, topics::{konst}, move |event| {{\n                let db = db.clone();\n                async move {{ enqueue_event(&db, topics::{konst}, &event).await }}\n            }})\n            .await;\n            if let Err(e) = result {{\n                tracing::error!(\"consumer {name} stopped: {{e:#}}\");\n            }}\n        }}));\n    }}\n",
            topic_name = topic.name,
            message = topic.message,
            group = crate::service::group_id(service, c),
            name = c.name,
        ));
    }
    out
}

fn event_rules(service: &Service) -> String {
    let mut out = String::from("pub const EVENT_RULES: &[EventRule] = &[\n");
    for c in &service.consumers {
        let Some(rule) = &c.mail else { continue };
        let fallback = match rule.dedup_fallback {
            DedupFallback::Recipient => "Recipient",
            DedupFallback::TopicAndRecipient => "TopicAndRecipient",
            DedupFallback::Reject => "Reject",
        };
        let sender = match &rule.sender_company_via {
            Some(field) => format!("SenderName::CompanyVia({field:?})"),
            None => "SenderName::Default".to_string(),
        };
        let reply_to = match &rule.reply_to {
            Some(field) => format!("Some({field:?})"),
            None => "None".to_string(),
        };
        out.push_str(&format!(
            "    EventRule {{\n        topic: {:?},\n        recipient_field: {:?},\n        optional_recipient: {},\n        dedup_field: {:?},\n        dedup_fallback: DedupFallback::{fallback},\n        sender: {sender},\n        reply_to_field: {reply_to},\n    }},\n",
            c.topic, rule.recipient, rule.optional_recipient, rule.dedup_field
        ));
    }
    out.push_str("];");
    out
}

fn seed(service: &Service) -> String {
    let mut out = String::from("/// Les mails du projet.\npub const SEED: &[Seed] = &[\n");
    for t in &service.templates {
        out.push_str(&format!(
            "    (\n        {:?},\n        {:?},\n        {:?},\n        {:?},\n    ),\n",
            t.event_type, t.subject, t.html, t.text
        ));
    }
    out.push_str("];");
    out
}

/// One test per event: the fields its rule reads exist under those names in the JSON of the message, which is where the
/// mail is rendered from.
fn field_tests(project: &Project, service: &Service) -> String {
    let mut out = String::new();
    for (c, topic, rule) in consumers(project, service) {
        let mut fields = vec![rule.recipient.as_str(), rule.dedup_field.as_str()];
        fields.extend(rule.sender_company_via.as_deref());
        fields.extend(rule.reply_to.as_deref());
        out.push_str(&format!(
            "#[test]\n    fn the_fields_of_{name}_are_in_the_json_of_its_message() {{\n        let json = serde_json::to_value(pb::{message}::default()).unwrap();\n        for field in {fields:?} {{\n            assert!(json.get(field).is_some(), \"{{field}} is not in {{json}}\");\n        }}\n    }}\n\n    ",
            name = c.name,
            message = topic.message,
        ));
    }
    out
}
