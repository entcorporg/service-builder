# MTA : service préfabriqué pour le Service Builder

Le MTA de Clairtyx (`backend/svc-mta`) n'a rien de personnalisable au sens du builder : il consomme des
événements Kafka, met un e-mail en file, le rend, le signe en DKIM et le livre en SMTP. Ce qui change d'un
projet à l'autre tient en **deux listes de données** :

1. les **événements écoutés** (quel topic, où est l'adresse, comment dédoublonner) ;
2. les **templates** (sujet, HTML, texte), semés par une migration.

Tout le reste est du **code fixe**, fourni tel quel par le builder (`kind: "mta"`). Ce document donne ce code
complet, les règles d'événements en données, et les preuves.

Document principal : [`SERVICE_BUILDER_COUVERTURE.md`](SERVICE_BUILDER_COUVERTURE.md).

## Pipeline

```
Kafka (topics = EVENT_RULES) → mail_queue (pending, dedup_key unique)
  → worker : verrouille un lot (FOR UPDATE SKIP LOCKED)
  → charge mail_templates, rend sujet + HTML + texte (Tera)
  → MIME → signature DKIM → MX ou relais SMTP
  → livré (ligne supprimée) | nouvel essai avec délai croissant | failed | bounced
Serveur SMTP entrant → bounces (DSN) → corrélation par Message-ID → statut bounced
```

## Ce qui est prouvé

| Marque | Quoi | Résultat |
|---|---|---|
| ✅ | Les tests du dépôt (`cargo test -p svc-mta --lib`, dont 7 `sqlx::test` sur base jetable) | **19 passés** |
| ✅ | Règles d'événements, templates, migration générée (§1) | **12 passés** + 1 sur la vraie migration |
| ✅ | 23 tests complémentaires sur le code existant, copié **octet pour octet** (§4) | sur le code actuel : **1 échec = un défaut** (§3), les autres passent (le test `dot_stuff` publié suppose le correctif) |
| ✅ | Le correctif du défaut, sur la copie de travail (dépôt non modifié) | **26 passés** (23 + 3 tests du dépôt sans base) |
| ✅ | La colle générée (§2) compile contre les vrais types `sqlx` | `cargo check` propre |
| 📝 | Le SQL de la file (`INSERT ... ON CONFLICT`, `FOR UPDATE SKIP LOCKED`, replanification) | non rejoué ici : pas d'accès PostgreSQL |
| 💡 | Le format `mta.json` | proposition, le builder ne le lit pas encore |

Ce qui n'est **pas** testé : une livraison vers un vrai MX sur Internet, le STARTTLS contre un vrai serveur
(les tests parlent à un faux serveur sans STARTTLS), le débit sous charge.

---

## 1. Les deux entrées de données

### 1.1 Proposition : `mta.json` 💡

```json
{
  "kind": "mta",
  "events": [
    { "topic": "auth.user.registered",              "recipient": "email", "dedup": { "field": "user_id", "fallback": "recipient" } },
    { "topic": "auth.email_change.requested",       "recipient": "email", "dedup": { "field": "eventId", "fallback": "topicAndRecipient" } },
    { "topic": "auth.password_reset.requested",     "recipient": "email", "dedup": { "field": "eventId", "fallback": "topicAndRecipient" } },
    { "topic": "auth.email_verification.requested", "recipient": "email", "dedup": { "field": "eventId", "fallback": "topicAndRecipient" } },
    { "topic": "auth.invitation.created",           "recipient": "email", "dedup": { "field": "eventId", "fallback": "reject" } },
    { "topic": "core.chantier.delay_reported",      "recipient": "clientEmail", "optionalRecipient": true,
       "dedup": { "field": "eventId", "fallback": "reject" },
       "sender": { "companyVia": "companyName" }, "replyTo": "contactEmail" }
  ],
  "templates": [
    { "eventType": "auth.user.registered", "subject": "Bienvenue sur Clairtyx !",
       "html": "templates/welcome.html", "text": "templates/welcome.txt" }
  ]
}
```

Le builder génère à partir de là : la liste `EVENT_RULES` (§1.2), les topics à consommer, et une migration
réversible (§1.3). Les variables de template sont les clés **camelCase** du payload de l'événement.

La cohérence est vérifiée à la génération (`check_consistency`) : un événement sans template, ou un template
sans événement, est une erreur.

### 1.2 Les règles, en code testé ✅

Ce module **remplace** `EmailEvent` (le `match` par topic) et les six `enqueue_*` du dépôt. Les tests prouvent
l'équivalence avec l'ancien code, y compris les clés de dédup de repli et le cas « client sans e-mail ». Un test
rend aussi les **six vrais templates** de la migration du dépôt avec ce moteur.

```rust
//! Service MTA préfabriqué : les deux seules entrées propres à un projet sont
//! la liste des événements écoutés (`EventRule`) et leurs templates
//! (`MailTemplate`, semés par une migration). Le reste du service (file, worker,
//! DKIM, SMTP, bounces) est du code fixe.
//!
//! Ce module remplace les six `enqueue_*` et les `match` de `EmailEvent` du
//! service actuel par des données : ajouter un e-mail = une règle + un template,
//! sans Rust.

use std::collections::HashMap;

use anyhow::{Context, Result, bail};
use serde_json::Value;
use tera::{Context as TeraCtx, Tera};

// ─── Règles d'événement ───────────────────────────────────────────────────────

/// Que faire quand le champ d'identité de l'événement manque.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DedupFallback {
    /// Clé = adresse du destinataire (un seul mail par adresse).
    Recipient,
    /// Clé = `<topic>:<adresse>` (forme historique des mails de sécurité).
    TopicAndRecipient,
    /// Payload invalide : erreur (jamais de mail sans identité d'événement).
    Reject,
}

/// Nom d'expéditeur affiché.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SenderName {
    /// `SMTP_FROM_NAME` ("Clairtyx").
    Default,
    /// `<entreprise> via Clairtyx` quand le champ est renseigné : le client final
    /// de l'entreprise ne connaît pas Clairtyx.
    CompanyVia(&'static str),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventRule {
    /// Sujet du topic Kafka = `mail_queue.event_type` = `mail_templates.event_type`.
    pub topic: &'static str,
    /// Champ du payload qui porte l'adresse du destinataire.
    pub recipient_field: &'static str,
    /// Sans adresse : rien à envoyer (`Plan::Skip`) au lieu d'une erreur.
    pub optional_recipient: bool,
    /// Champ qui identifie l'événement pour la dédup (at-least-once).
    pub dedup_field: &'static str,
    pub dedup_fallback: DedupFallback,
    pub sender: SenderName,
    /// Champ du payload qui porte l'adresse `Reply-To`.
    pub reply_to_field: Option<&'static str>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Plan {
    Enqueue {
        recipient: String,
        dedup_key: String,
    },
    Skip,
}

impl EventRule {
    /// Décide de la mise en file d'un événement. Fonction pure : l'INSERT
    /// `ON CONFLICT (dedup_key) WHERE dedup_key IS NOT NULL DO NOTHING` est fixe.
    pub fn plan(&self, payload: &Value) -> Result<Plan> {
        let recipient = match payload[self.recipient_field].as_str() {
            Some(r) if !(self.optional_recipient && r.is_empty()) => r,
            _ if self.optional_recipient => return Ok(Plan::Skip),
            _ => bail!(
                "champ '{}' absent du payload {}",
                self.recipient_field,
                self.topic
            ),
        };

        let identity = match (payload[self.dedup_field].as_str(), self.dedup_fallback) {
            (Some(id), _) => id.to_owned(),
            (None, DedupFallback::Recipient) => recipient.to_owned(),
            (None, DedupFallback::TopicAndRecipient) => format!("{}:{recipient}", self.topic),
            (None, DedupFallback::Reject) => {
                bail!(
                    "champ '{}' absent du payload {}",
                    self.dedup_field,
                    self.topic
                )
            }
        };
        Ok(Plan::Enqueue {
            recipient: recipient.to_owned(),
            dedup_key: format!("{}:{identity}", self.topic),
        })
    }

    pub fn sender_name(&self, vars: &HashMap<String, Value>, default_name: &str) -> String {
        match self.sender {
            SenderName::CompanyVia(field) => match vars.get(field).and_then(Value::as_str) {
                Some(company) if !company.is_empty() => format!("{company} via Clairtyx"),
                _ => default_name.to_owned(),
            },
            SenderName::Default => default_name.to_owned(),
        }
    }

    pub fn reply_to<'v>(&self, vars: &'v HashMap<String, Value>) -> Option<&'v str> {
        let field = self.reply_to_field?;
        vars.get(field)
            .and_then(Value::as_str)
            .filter(|email| !email.is_empty())
    }
}

/// Les six événements de Clairtyx aujourd'hui.
pub const EVENT_RULES: &[EventRule] = &[
    EventRule {
        topic: "auth.user.registered",
        recipient_field: "email",
        optional_recipient: false,
        dedup_field: "user_id",
        dedup_fallback: DedupFallback::Recipient,
        sender: SenderName::Default,
        reply_to_field: None,
    },
    EventRule {
        topic: "auth.email_change.requested",
        recipient_field: "email",
        optional_recipient: false,
        dedup_field: "eventId",
        dedup_fallback: DedupFallback::TopicAndRecipient,
        sender: SenderName::Default,
        reply_to_field: None,
    },
    EventRule {
        topic: "auth.password_reset.requested",
        recipient_field: "email",
        optional_recipient: false,
        dedup_field: "eventId",
        dedup_fallback: DedupFallback::TopicAndRecipient,
        sender: SenderName::Default,
        reply_to_field: None,
    },
    EventRule {
        topic: "auth.email_verification.requested",
        recipient_field: "email",
        optional_recipient: false,
        dedup_field: "eventId",
        dedup_fallback: DedupFallback::TopicAndRecipient,
        sender: SenderName::Default,
        reply_to_field: None,
    },
    EventRule {
        topic: "auth.invitation.created",
        recipient_field: "email",
        optional_recipient: false,
        dedup_field: "eventId",
        dedup_fallback: DedupFallback::Reject,
        sender: SenderName::Default,
        reply_to_field: None,
    },
    EventRule {
        topic: "core.chantier.delay_reported",
        recipient_field: "clientEmail",
        optional_recipient: true,
        dedup_field: "eventId",
        dedup_fallback: DedupFallback::Reject,
        sender: SenderName::CompanyVia("companyName"),
        reply_to_field: Some("contactEmail"),
    },
];

pub fn rule_for(topic: &str) -> Option<&'static EventRule> {
    EVENT_RULES.iter().find(|rule| rule.topic == topic)
}

// ─── Templates ────────────────────────────────────────────────────────────────

/// Une ligne de `mail_templates` : `subject`, `html_body` et `text_body` sont des
/// templates Tera rendus avec le payload de l'événement (clés en camelCase).
#[derive(Debug, Clone)]
pub struct MailTemplate {
    pub event_type: String,
    pub subject: String,
    pub html_body: String,
    pub text_body: String,
}

pub struct Rendered {
    pub subject: String,
    pub html: String,
    pub text: String,
}

pub struct TemplateSet {
    tera: Tera,
}

impl TemplateSet {
    /// Compile les templates. `.html` est auto-échappé par Tera ; le sujet et le
    /// texte brut ne le sont pas (d'où `| safe` sur les URL du HTML).
    pub fn from_rows(rows: &[MailTemplate]) -> Result<Self> {
        let raw = rows.iter().flat_map(|row| {
            [
                (format!("{}.subject", row.event_type), row.subject.as_str()),
                (format!("{}.html", row.event_type), row.html_body.as_str()),
                (format!("{}.txt", row.event_type), row.text_body.as_str()),
            ]
        });
        let mut tera = Tera::default();
        tera.add_raw_templates(raw)
            .context("Tera : compilation des templates")?;
        Ok(Self { tera })
    }

    pub fn render(&self, event_type: &str, vars: &HashMap<String, Value>) -> Result<Rendered> {
        let mut ctx = TeraCtx::new();
        for (k, v) in vars {
            ctx.insert(k, v);
        }
        let render = |ext: &str| {
            self.tera
                .render(&format!("{event_type}.{ext}"), &ctx)
                .with_context(|| format!("Tera : rendu de '{event_type}.{ext}'"))
        };
        Ok(Rendered {
            subject: render("subject")?,
            html: render("html")?,
            text: render("txt")?,
        })
    }
}

// ─── Migration générée ────────────────────────────────────────────────────────

const QUOTE: &str = "$tpl$";

fn quoted(value: &str) -> Result<String> {
    if value.contains(QUOTE) {
        bail!("le template contient le délimiteur {QUOTE}");
    }
    Ok(format!("{QUOTE}{value}{QUOTE}"))
}

/// `<version>_mail_templates_seed.up.sql` : sème les templates. Idempotent
/// (`ON CONFLICT DO NOTHING`) : une modification faite en base ensuite n'est pas écrasée.
pub fn seed_up_sql(templates: &[MailTemplate]) -> Result<String> {
    let rows = templates
        .iter()
        .map(|t| {
            Ok(format!(
                "({},\n{},\n{},\n{})",
                quoted(&t.event_type)?,
                quoted(&t.subject)?,
                quoted(&t.html_body)?,
                quoted(&t.text_body)?
            ))
        })
        .collect::<Result<Vec<_>>>()?
        .join(",\n");
    Ok(format!(
        "INSERT INTO mail_templates (event_type, subject, html_body, text_body) VALUES\n{rows}\nON CONFLICT (event_type) DO NOTHING;\n"
    ))
}

/// `.down.sql` : retire exactement ce que le `.up.sql` a semé.
pub fn seed_down_sql(templates: &[MailTemplate]) -> Result<String> {
    let keys = templates
        .iter()
        .map(|t| quoted(&t.event_type))
        .collect::<Result<Vec<_>>>()?
        .join(", ");
    Ok(format!(
        "DELETE FROM mail_templates WHERE event_type IN ({keys});\n"
    ))
}

/// Cohérence du projet : chaque règle a son template, chaque template sa règle.
pub fn check_consistency(rules: &[EventRule], templates: &[MailTemplate]) -> Result<()> {
    for rule in rules {
        if !templates.iter().any(|t| t.event_type == rule.topic) {
            bail!("l'événement '{}' n'a pas de template", rule.topic);
        }
    }
    for template in templates {
        if !rules.iter().any(|r| r.topic == template.event_type) {
            bail!(
                "le template '{}' ne correspond à aucun événement écouté",
                template.event_type
            );
        }
    }
    Ok(())
}
```

<details><summary>Tests de <code>mta_rules.rs</code> (12 + 1)</summary>

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rule(topic: &str) -> &'static EventRule {
        rule_for(topic).unwrap()
    }

    fn key(rule: &EventRule, payload: Value) -> String {
        match rule.plan(&payload).unwrap() {
            Plan::Enqueue { dedup_key, .. } => dedup_key,
            Plan::Skip => panic!("devrait être mis en file"),
        }
    }

    // ── Équivalence avec les six enqueue_* actuels ──

    #[test]
    fn welcome_is_deduplicated_per_user_not_per_event() {
        let r = rule("auth.user.registered");
        let payload = json!({ "email": "a@b.fr", "user_id": "u-1" });
        assert_eq!(key(r, payload.clone()), "auth.user.registered:u-1");
        // Rejoué avec un autre eventId : toujours le même mail de bienvenue.
        assert_eq!(
            key(
                r,
                json!({ "email": "a@b.fr", "user_id": "u-1", "eventId": "x" })
            ),
            key(r, payload)
        );
        // Sans user_id : repli sur l'adresse.
        assert_eq!(
            key(r, json!({ "email": "a@b.fr" })),
            "auth.user.registered:a@b.fr"
        );
    }

    #[test]
    fn security_mails_are_deduplicated_per_request_so_each_legit_request_is_sent() {
        for topic in [
            "auth.email_change.requested",
            "auth.password_reset.requested",
            "auth.email_verification.requested",
        ] {
            let r = rule(topic);
            let first = key(r, json!({ "email": "a@b.fr", "eventId": "e-1" }));
            let second = key(r, json!({ "email": "a@b.fr", "eventId": "e-2" }));
            assert_eq!(first, format!("{topic}:e-1"));
            assert_ne!(first, second, "deux demandes légitimes = deux mails");
            assert_eq!(
                first,
                key(r, json!({ "email": "a@b.fr", "eventId": "e-1" })),
                "rejeu = no-op"
            );
            // Repli historique (event sans eventId), clé identique à l'ancien code.
            assert_eq!(
                key(r, json!({ "email": "a@b.fr" })),
                format!("{topic}:{topic}:a@b.fr")
            );
        }
    }

    #[test]
    fn invitation_requires_an_event_id() {
        let r = rule("auth.invitation.created");
        assert_eq!(
            key(r, json!({ "email": "a@b.fr", "eventId": "inv-1" })),
            "auth.invitation.created:inv-1"
        );
        let err = r
            .plan(&json!({ "email": "a@b.fr" }))
            .unwrap_err()
            .to_string();
        assert!(err.contains("eventId"), "{err}");
    }

    #[test]
    fn delay_report_is_skipped_without_client_email_but_requires_event_id_with_one() {
        let r = rule("core.chantier.delay_reported");
        for payload in [
            json!({ "eventId": "d-1" }),
            json!({ "eventId": "d-1", "clientEmail": "" }),
            json!({ "eventId": "d-1", "clientEmail": null }),
        ] {
            assert_eq!(r.plan(&payload).unwrap(), Plan::Skip, "{payload}");
        }
        assert_eq!(
            r.plan(&json!({ "eventId": "d-1", "clientEmail": "c@x.fr" }))
                .unwrap(),
            Plan::Enqueue {
                recipient: "c@x.fr".into(),
                dedup_key: "core.chantier.delay_reported:d-1".into()
            }
        );
        assert!(r.plan(&json!({ "clientEmail": "c@x.fr" })).is_err());
    }

    #[test]
    fn a_missing_recipient_is_a_payload_error_for_mandatory_recipients() {
        let err = rule("auth.password_reset.requested")
            .plan(&json!({ "eventId": "e" }))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("'email'") && err.contains("auth.password_reset.requested"),
            "{err}"
        );
    }

    #[test]
    fn sender_name_and_reply_to_follow_the_rule() {
        let delay = rule("core.chantier.delay_reported");
        let vars: HashMap<String, Value> = serde_json::from_value(json!({
            "companyName": "Dupont Plomberie", "contactEmail": "contact@dupont.fr"
        }))
        .unwrap();
        assert_eq!(
            delay.sender_name(&vars, "Clairtyx"),
            "Dupont Plomberie via Clairtyx"
        );
        assert_eq!(delay.reply_to(&vars), Some("contact@dupont.fr"));

        let empty = HashMap::new();
        assert_eq!(delay.sender_name(&empty, "Clairtyx"), "Clairtyx");
        assert_eq!(delay.reply_to(&empty), None);

        let welcome = rule("auth.user.registered");
        assert_eq!(welcome.sender_name(&vars, "Clairtyx"), "Clairtyx");
        assert_eq!(welcome.reply_to(&vars), None);
    }

    #[test]
    fn rules_are_unique_per_topic_and_lookup_works() {
        let mut topics: Vec<_> = EVENT_RULES.iter().map(|r| r.topic).collect();
        topics.sort_unstable();
        topics.dedup();
        assert_eq!(topics.len(), EVENT_RULES.len());
        assert!(rule_for("inconnu").is_none());
    }

    // ── Templates ──

    fn tpl(event_type: &str) -> MailTemplate {
        MailTemplate {
            event_type: event_type.into(),
            subject: "Bonjour {{ name }}".into(),
            html_body:
                r#"<p>{{ name | default(value="") }}</p><a href="{{ resetUrl | safe }}">lien</a>"#
                    .into(),
            text_body: "Lien : {{ resetUrl }} (expire le {{ expiresAt }})".into(),
        }
    }

    fn vars(value: Value) -> HashMap<String, Value> {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn html_is_escaped_but_subject_and_text_are_not_and_safe_keeps_urls_intact() {
        let set = TemplateSet::from_rows(&[tpl("auth.password_reset.requested")]).unwrap();
        let out = set
            .render(
                "auth.password_reset.requested",
                &vars(json!({ "name": "<script>x</script>", "resetUrl": "https://a.fr/r?t=1&u=2", "expiresAt": "demain" })),
            )
            .unwrap();
        assert!(out.html.contains("&lt;script&gt;"), "{}", out.html);
        assert!(!out.html.contains("<script>"));
        assert!(
            out.html.contains(r#"href="https://a.fr/r?t=1&u=2""#),
            "URL intacte grâce à | safe"
        );
        assert!(out.text.contains("https://a.fr/r?t=1&u=2") && out.text.contains("demain"));
        assert_eq!(
            out.subject, "Bonjour <script>x</script>",
            "le sujet n'est pas du HTML"
        );
    }

    #[test]
    fn a_broken_template_fails_at_load_and_a_missing_variable_fails_at_render() {
        let mut broken = tpl("a.b");
        broken.html_body = "{% if %}".into();
        assert!(TemplateSet::from_rows(&[broken]).is_err());

        let set = TemplateSet::from_rows(&[tpl("a.b")]).unwrap();
        assert!(
            set.render("a.b", &vars(json!({}))).is_err(),
            "name/resetUrl absents"
        );
        assert!(set.render("inconnu", &vars(json!({}))).is_err());
    }

    // ── Migration générée ──

    #[test]
    fn seed_migration_is_reversible_and_idempotent() {
        let templates = [
            tpl("auth.password_reset.requested"),
            tpl("auth.user.registered"),
        ];
        let up = seed_up_sql(&templates).unwrap();
        assert!(up.starts_with(
            "INSERT INTO mail_templates (event_type, subject, html_body, text_body) VALUES"
        ));
        assert!(up.contains("$tpl$auth.password_reset.requested$tpl$"));
        assert!(
            up.contains("{{ resetUrl | safe }}"),
            "corps inséré tel quel"
        );
        assert!(
            up.trim_end()
                .ends_with("ON CONFLICT (event_type) DO NOTHING;")
        );

        let down = seed_down_sql(&templates).unwrap();
        assert_eq!(
            down,
            "DELETE FROM mail_templates WHERE event_type IN ($tpl$auth.password_reset.requested$tpl$, $tpl$auth.user.registered$tpl$);\n"
        );
    }

    #[test]
    fn a_template_containing_the_dollar_quote_is_refused() {
        let mut bad = tpl("a.b");
        bad.text_body = "prix $tpl$ injecté".into();
        assert!(seed_up_sql(&[bad]).is_err());
    }

    #[test]
    fn rules_and_templates_must_match_one_to_one() {
        let templates: Vec<_> = EVENT_RULES.iter().map(|r| tpl(r.topic)).collect();
        assert!(check_consistency(EVENT_RULES, &templates).is_ok());
        assert!(
            check_consistency(EVENT_RULES, &templates[1..])
                .unwrap_err()
                .to_string()
                .contains("auth.user.registered")
        );
        let mut extra = templates.clone();
        extra.push(tpl("orphelin.event"));
        assert!(
            check_consistency(EVENT_RULES, &extra)
                .unwrap_err()
                .to_string()
                .contains("orphelin.event")
        );
    }

    /// Vérification contre les 6 templates réels de la migration du dépôt :
    /// `SB_MTA_MIGRATION=<chemin du .sql> cargo test -- --ignored real_migration`.
    #[test]
    #[ignore = "nécessite SB_MTA_MIGRATION"]
    fn real_migration_templates_all_compile_and_match_the_rules() {
        let path = std::env::var("SB_MTA_MIGRATION").expect("SB_MTA_MIGRATION");
        let sql = std::fs::read_to_string(path).unwrap();
        let mut templates = Vec::new();
        // Chaque ligne : ($tpl$event$tpl$, $tpl$subject$tpl$, $tpl$html$tpl$, $tpl$text$tpl$)
        let parts: Vec<&str> = sql.split("$tpl$").collect();
        // parts[1], [3], [5], [7] = event, subject, html, text ; les impairs sont les valeurs.
        let values: Vec<&str> = parts.iter().skip(1).step_by(2).copied().collect();
        for chunk in values.chunks(4) {
            templates.push(MailTemplate {
                event_type: chunk[0].into(),
                subject: chunk[1].into(),
                html_body: chunk[2].into(),
                text_body: chunk[3].into(),
            });
        }
        assert_eq!(templates.len(), 6);
        check_consistency(EVENT_RULES, &templates).unwrap();
        let set = TemplateSet::from_rows(&templates).unwrap();
        let sample = vars(json!({
            "name": "Camille", "resetUrl": "https://a.fr/r", "confirmUrl": "https://a.fr/c", "verifyUrl": "https://a.fr/v",
            "inviteUrl": "https://a.fr/i", "expiresAt": "2026-09-23", "expiresOn": "01/10/2026", "inviterName": "Paul",
            "companyName": "Dupont", "clientName": "Mme Dubois", "workerName": "Paul", "reasonLabel": "Panne",
            "address": "12 rue des Lilas", "note": "", "newEstimatedTimeOn": "10:30"
        }));
        for rule in EVENT_RULES {
            let out = set
                .render(rule.topic, &sample)
                .unwrap_or_else(|e| panic!("{}: {e:#}", rule.topic));
            assert!(
                !out.subject.is_empty() && out.html.contains("<html") && !out.text.is_empty(),
                "{}",
                rule.topic
            );
        }
    }
}
```

</details>

### 1.3 La migration générée ✅

`seed_up_sql` / `seed_down_sql` (dans le module ci-dessus) produisent la paire `.up.sql` / `.down.sql` exigée par
les règles du projet. Les corps sont insérés dans des chaînes `$tpl$...$tpl$` ; un template qui contiendrait ce
délimiteur est refusé. L'`.up.sql` est idempotent (`ON CONFLICT DO NOTHING`) : modifier un template en base
ensuite n'est pas écrasé par un redéploiement. Le worker relit `mail_templates` à chaque cycle, donc une
modification s'applique **sans redémarrage**.

---

## 2. La colle générée (compilée, SQL non rejoué)

Une seule fonction remplace les six `enqueue_*`. Le SQL est celui du dépôt, mot pour mot.

```rust
//! Colle générée entre les règles (données) et la file `mail_queue` (fixe) :
//! remplace les six `enqueue_*` de `core/consumer.rs`.

use anyhow::Result;
use sqlx::PgPool;

use crate::mta_rules::{EventRule, Plan};

/// Issue d'une mise en file : nouveau job, événement rejoué déjà en file, ou rien
/// à envoyer (pas d'adresse e-mail).
pub enum Enqueued {
    New,
    Duplicate,
    Skipped,
}

/// Met un événement en file selon sa règle. `dedup_key` est unique : un événement
/// rejoué (at-least-once) est un no-op grâce à `ON CONFLICT DO NOTHING`.
pub async fn enqueue_event(
    db: &PgPool,
    rule: &EventRule,
    payload: &serde_json::Value,
) -> Result<Enqueued> {
    let (recipient, dedup_key) = match rule.plan(payload)? {
        Plan::Skip => {
            tracing::debug!(topic = rule.topic, "pas d'adresse — rien à mettre en file");
            return Ok(Enqueued::Skipped);
        }
        Plan::Enqueue {
            recipient,
            dedup_key,
        } => (recipient, dedup_key),
    };

    let inserted = sqlx::query_scalar::<_, uuid::Uuid>(
        // L'index unique sur dedup_key est PARTIEL (WHERE dedup_key IS NOT NULL) :
        // l'inférence ON CONFLICT doit répéter ce prédicat pour le matcher.
        r#"INSERT INTO mail_queue (event_type, recipient, payload, dedup_key)
           VALUES ($1, $2, $3, $4)
           ON CONFLICT (dedup_key) WHERE dedup_key IS NOT NULL DO NOTHING
           RETURNING id"#,
    )
    .bind(rule.topic)
    .bind(&recipient)
    .bind(payload)
    .bind(&dedup_key)
    .fetch_optional(db)
    .await?;

    Ok(match inserted {
        Some(_) => Enqueued::New,
        None => Enqueued::Duplicate,
    })
}
```


Les trois substitutions à faire dans le reste du code (📝 non compilées, elles touchent `shared::kafka`) :

```diff
 // core/consumer.rs — apply_or_dead_letter
-        let result = match topic {
-            AUTH_USER_REGISTERED => enqueue_welcome(db, &value).await,
-            ... (cinq autres branches)
-        };
+        let Some(rule) = rule_for(topic) else {
+            tracing::warn!(topic, "topic non géré — ignoré");
+            count_message(topic, "invalid");
+            return true;
+        };
+        let result = enqueue_event(db, rule, &value).await;

 // core/consumer.rs
-pub const TOPICS: &[&str] = &[AUTH_USER_REGISTERED, ... ];
+// généré : EVENT_RULES.iter().map(|r| r.topic)

 // mta/queue_worker.rs — process_job
-    let event = EmailEvent::from_subject(&job.event_type)
-        .ok_or_else(|| anyhow::anyhow!("type d'événement inconnu : '{}'", job.event_type))?;
+    let rule = rule_for(&job.event_type)
+        .ok_or_else(|| anyhow::anyhow!("type d'événement inconnu : '{}'", job.event_type))?;
-    let rendered = templates.render(&event, &vars)?;
+    let rendered = templates.render(rule.topic, &vars)?;
-    let sender_name = event.sender_name(&vars, &state.config.smtp_from_name);
-    let reply_to = event.reply_to(&vars);
+    let sender_name = rule.sender_name(&vars, &state.config.smtp_from_name);
+    let reply_to = rule.reply_to(&vars);
```

---

## 3. Défaut trouvé dans le code existant : LF seul dans les données SMTP

**Constat.** Un test a échoué sur le code actuel de `src/mta/smtp.rs` :
`the_data_section_never_contains_a_bare_lf` → `lignes terminées par LF seul : ["\n"]`.

**Cause.** `dot_stuff` découpe sur `\n` puis ajoute un `\n` après **chaque** segment, y compris le dernier segment
vide qui suit la fin du message. Résultat : un `\n` isolé (sans `\r`) juste avant la ligne `.` finale. La
RFC 5321 §2.3.8 exige des CRLF partout dans les données.

**Portée.** Le défaut est établi sur ce qui est **envoyé** (les octets reçus par le faux serveur), pas sur le refus
d'un serveur réel : je n'ai testé aucun MX. Certains serveurs normalisent ou rejettent les fins de ligne en LF seul
(par exemple Postfix, option `smtpd_forbid_bare_newline`, introduite après les attaques dites de « SMTP
smuggling »). Le risque est donc en production chez des destinataires stricts, pas forcément en dev.

**Correctif testé** sur la copie de travail (le dépôt n'est **pas** modifié) : 26 tests passent, dont celui-ci.
`split_inclusive` garde chaque fin de ligne telle quelle, et le terminateur ne double plus le CRLF final.

```diff
--- a/src/mta/smtp.rs
+++ b/src/mta/smtp.rs
@@ -182,7 +182,13 @@
     // Transparence : doubler les lignes commençant par '.'
     let body = dot_stuff(raw_message);
     transport.write_all(&body).await?;
-    transport.write_all(b"\r\n.\r\n").await?;
+    // Le message se termine déjà par CRLF : pas de ligne vide avant le point final.
+    let terminator: &[u8] = if body.ends_with(b"\r\n") {
+        b".\r\n"
+    } else {
+        b"\r\n.\r\n"
+    };
+    transport.write_all(terminator).await?;
 
     let (code, msg) = transport.read_reply().await?;
     expect_2xx(code, &format!("fin DATA — {msg}"))?;
@@ -236,12 +242,11 @@
 /// chaque ligne commençant par '.' est préfixée d'un '.' supplémentaire.
 fn dot_stuff(message: &[u8]) -> Vec<u8> {
     let mut out = Vec::with_capacity(message.len() + 64);
-    for line in message.split(|&b| b == b'\n') {
+    for line in message.split_inclusive(|&b| b == b'\n') {
         if line.starts_with(b".") {
             out.push(b'.');
         }
         out.extend_from_slice(line);
-        out.push(b'\n');
     }
     out
 }
```


---

## 4. Tests complémentaires sur le code existant ✅

Chaque bloc s'ajoute **en bas du fichier correspondant**, qui reste identique au dépôt (vérifié par comparaison),
à une exception près : `smtp.rs` porte le correctif du §3. Le test `dot_stuffing_doubles_only_leading_dots`
vérifie le comportement **corrigé** ; sur le code actuel, il échoue lui aussi (le `\n` en trop).
Le serveur SMTP est un faux serveur sur un port local ; la clé DKIM est générée à la volée, jamais lue ni stockée.

- **`smtp.rs`** (6) : enveloppe et dot-stuffing, `550` = adresse refusée, `452` = erreur à réessayer, port fermé
  sans blocage, aucun LF seul.
- **`dkim.rs`** (5) : la signature est en tête et le message suit **octet pour octet** ; un **récepteur
  indépendant** (`mail-auth`) vérifie la signature avec la clé publiée en DNS = `Pass` ; un octet modifié ou une
  autre clé = échec ; une clé invalide est une erreur.
- **`bounce/server.rs`** (8) : 5xx = permanent, 4xx = transitoire, DSN illisible = transitoire sans code,
  `Message-ID` d'origine trouvé par ordre de fiabilité, et une **session SMTP complète** sur le vrai
  `handle_connection`.
- **`queue_worker.rs`** (4) : délai doublé à chaque essai avec moins d'une minute d'aléa, plafonné à 24 h sans
  débordement, payload non objet refusé, nom d'affichage.

<details><summary>Tests de <code>src/mta/smtp.rs</code></summary>

```rust
#[cfg(test)]
mod extra_tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::TcpListener;

    /// Faux serveur SMTP (sans STARTTLS) : répond `rcpt_reply` au `RCPT TO` et
    /// renvoie toutes les lignes reçues, commandes et données.
    async fn fake_smtp(rcpt_reply: &'static str) -> (u16, tokio::task::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (read, mut write) = stream.into_split();
            let mut read = BufReader::new(read);
            let mut received = Vec::new();
            let mut in_data = false;
            write.write_all(b"220 fake ESMTP\r\n").await.unwrap();
            loop {
                let mut line = String::new();
                if read.read_line(&mut line).await.unwrap() == 0 {
                    break;
                }
                received.push(line.clone());
                if in_data {
                    if line == ".\r\n" {
                        in_data = false;
                        write.write_all(b"250 queued\r\n").await.unwrap();
                    }
                    continue;
                }
                let upper = line.to_uppercase();
                let reply = if upper.starts_with("EHLO") {
                    "250-fake\r\n250 SIZE 10485760\r\n"
                } else if upper.starts_with("MAIL FROM") {
                    "250 ok\r\n"
                } else if upper.starts_with("RCPT TO") {
                    rcpt_reply
                } else if upper.starts_with("DATA") {
                    in_data = true;
                    "354 go\r\n"
                } else if upper.starts_with("QUIT") {
                    write.write_all(b"221 bye\r\n").await.unwrap();
                    break;
                } else {
                    "502 ?\r\n"
                };
                write.write_all(reply.as_bytes()).await.unwrap();
            }
            received
        });
        (port, handle)
    }

    #[test]
    fn dot_stuffing_doubles_only_leading_dots() {
        assert_eq!(
            dot_stuff(b".a\r\nb.c\r\n..d\r\n"),
            b"..a\r\nb.c\r\n...d\r\n"
        );
        assert_eq!(dot_stuff(b"sans fin de ligne"), b"sans fin de ligne");
        assert_eq!(dot_stuff(b""), b"");
    }

    #[tokio::test]
    async fn delivers_a_message_with_the_expected_envelope_and_dot_stuffing() {
        let (port, server) = fake_smtp("250 ok\r\n").await;
        let raw = b"Subject: test\r\n\r\n.ligne qui commence par un point\r\nnormale\r\n";
        deliver(
            "127.0.0.1",
            port,
            "noreply@clairtyx.dev",
            "client@exemple.fr",
            raw,
            "clairtyx.dev",
        )
        .await
        .unwrap();
        let received = server.await.unwrap();
        assert_eq!(received[0], "EHLO clairtyx.dev\r\n");
        assert_eq!(received[1], "MAIL FROM:<noreply@clairtyx.dev>\r\n");
        assert_eq!(received[2], "RCPT TO:<client@exemple.fr>\r\n");
        assert!(
            received.contains(&"..ligne qui commence par un point\r\n".to_string()),
            "{received:?}"
        );
        assert!(received.contains(&"normale\r\n".to_string()));
        assert!(received.iter().any(|l| l == "QUIT\r\n"));
    }

    #[tokio::test]
    async fn a_550_on_rcpt_is_reported_as_a_refused_address() {
        let (port, _server) = fake_smtp("550 5.1.1 no such user\r\n").await;
        let err = deliver(
            "127.0.0.1",
            port,
            "a@clairtyx.dev",
            "x@exemple.fr",
            b"Subject: x\r\n\r\ny\r\n",
            "clairtyx.dev",
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(
            err.contains("adresse refusée") && err.contains("550"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn a_452_on_rcpt_is_an_error_so_the_job_is_retried() {
        let (port, _server) = fake_smtp("452 4.2.2 mailbox full\r\n").await;
        let err = deliver(
            "127.0.0.1",
            port,
            "a@clairtyx.dev",
            "x@exemple.fr",
            b"Subject: x\r\n\r\ny\r\n",
            "clairtyx.dev",
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(err.contains("452"), "{err}");
    }

    #[tokio::test]
    async fn a_closed_port_is_an_error_not_a_hang() {
        let closed = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = closed.local_addr().unwrap().port();
        drop(closed);
        assert!(
            deliver("127.0.0.1", port, "a@b.fr", "c@d.fr", b"x", "b.fr")
                .await
                .is_err()
        );
    }

    /// RFC 5321 §2.3.8 : toute fin de ligne des données est CRLF. Un LF seul est
    /// refusé ou normalisé par certains serveurs (Postfix `smtpd_forbid_bare_newline`).
    #[tokio::test]
    async fn the_data_section_never_contains_a_bare_lf() {
        let (port, server) = fake_smtp("250 ok\r\n").await;
        let raw = b"Subject: test\r\n\r\ncorps\r\n";
        deliver(
            "127.0.0.1",
            port,
            "a@clairtyx.dev",
            "x@exemple.fr",
            raw,
            "clairtyx.dev",
        )
        .await
        .unwrap();
        let received = server.await.unwrap();
        let bare: Vec<_> = received
            .iter()
            .filter(|l| l.ends_with('\n') && !l.ends_with("\r\n"))
            .collect();
        assert!(bare.is_empty(), "lignes terminées par LF seul : {bare:?}");
    }
}
```

</details>

<details><summary>Tests de <code>src/mta/dkim.rs</code></summary>

```rust
#[cfg(test)]
mod extra_tests {
    use super::*;
    use crate::message;
    use base64::{Engine, engine::general_purpose::STANDARD};
    use mail_auth::common::{parse::TxtRecordParser, verify::DomainKey};
    use mail_auth::{
        AuthenticatedMessage, DkimResult, MessageAuthenticator, Parameters, ResolverCache, Txt,
    };
    use rsa::pkcs1::EncodeRsaPrivateKey;
    use rsa::pkcs8::EncodePublicKey;
    use std::borrow::Borrow;
    use std::collections::HashMap;
    use std::hash::Hash;
    use std::sync::Mutex;
    use std::time::Instant;

    /// Zone DNS en mémoire : le récepteur « trouve » la clé publiée sans réseau.
    struct Zone(Mutex<HashMap<String, Txt>>);

    impl ResolverCache<String, Txt> for Zone {
        fn get<Q>(&self, name: &Q) -> Option<Txt>
        where
            String: Borrow<Q>,
            Q: Hash + Eq + ?Sized,
        {
            self.0.lock().unwrap().get(name).cloned()
        }
        fn remove<Q>(&self, name: &Q) -> Option<Txt>
        where
            String: Borrow<Q>,
            Q: Hash + Eq + ?Sized,
        {
            self.0.lock().unwrap().remove(name)
        }
        fn insert(&self, name: String, value: Txt, _valid_until: Instant) {
            self.0.lock().unwrap().insert(name, value);
        }
    }

    fn zone_with_key(public: &str) -> Zone {
        let zone = Zone(Mutex::new(HashMap::new()));
        let record = DomainKey::parse(format!("v=DKIM1; k=rsa; p={public}").as_bytes()).unwrap();
        zone.insert(
            "mail._domainkey.clairtyx.dev.".to_string(),
            record.into(),
            Instant::now(),
        );
        zone
    }

    async fn verdict(signed: &[u8], zone: &Zone) -> String {
        let parsed = AuthenticatedMessage::parse(signed).unwrap();
        let outputs = MessageAuthenticator::new_system_conf()
            .unwrap()
            .verify_dkim(Parameters::new(&parsed).with_txt_cache(zone))
            .await;
        assert_eq!(outputs.len(), 1);
        format!("{:?}", outputs[0].result())
    }

    /// Clé jetable générée pour le test : rien n'est lu ni stocké.
    fn throwaway_key() -> (String, String) {
        let key = rsa::RsaPrivateKey::new(&mut rsa::rand_core::OsRng, 2048).unwrap();
        let pem = key
            .to_pkcs1_pem(rsa::pkcs8::LineEnding::LF)
            .unwrap()
            .to_string();
        let public = STANDARD.encode(key.to_public_key().to_public_key_der().unwrap().as_bytes());
        (pem, public)
    }

    fn message() -> message::OutgoingMessage {
        message::build(
            "noreply@clairtyx.dev",
            "Clairtyx",
            "client@exemple.fr",
            Some("Camille"),
            None,
            "Bonjour",
            "<p>html</p>",
            "texte",
            "clairtyx.dev",
        )
        .unwrap()
    }

    #[test]
    fn the_signature_is_prepended_and_the_message_is_untouched() {
        let (pem, _) = throwaway_key();
        let msg = message();
        let signed =
            String::from_utf8(sign(&msg.raw, &pem, "clairtyx.dev", "mail").unwrap()).unwrap();
        let raw = String::from_utf8(msg.raw).unwrap();
        assert!(signed.starts_with("DKIM-Signature:"));
        assert!(
            signed.ends_with(&raw),
            "le message d'origine suit, octet pour octet"
        );
        let header: String = signed[..signed.len() - raw.len()]
            .replace("\r\n ", " ")
            .replace("\r\n\t", " ");
        for expected in ["d=clairtyx.dev", "s=mail", "a=rsa-sha256", "bh="] {
            assert!(header.contains(expected), "{expected} absent de {header}");
        }
        for signed_header in [
            "From",
            "To",
            "Subject",
            "Date",
            "Message-ID",
            "MIME-Version",
            "Content-Type",
        ] {
            assert!(header.contains(signed_header), "{signed_header} non signé");
        }
    }

    #[tokio::test]
    async fn a_receiver_can_verify_the_signature_with_the_published_dns_key() {
        let (pem, public) = throwaway_key();
        let signed = sign(&message().raw, &pem, "clairtyx.dev", "mail").unwrap();
        assert_eq!(
            verdict(&signed, &zone_with_key(&public)).await,
            format!("{:?}", DkimResult::Pass)
        );
    }

    #[tokio::test]
    async fn tampering_after_signing_breaks_verification() {
        let (pem, public) = throwaway_key();
        let mut signed = sign(&message().raw, &pem, "clairtyx.dev", "mail").unwrap();
        let pos = signed.windows(5).position(|w| w == b"texte").unwrap();
        signed[pos] = b'T';
        assert_ne!(
            verdict(&signed, &zone_with_key(&public)).await,
            format!("{:?}", DkimResult::Pass)
        );
    }

    #[tokio::test]
    async fn another_domain_key_does_not_verify() {
        let (pem, _) = throwaway_key();
        let (_, other_public) = throwaway_key();
        let signed = sign(&message().raw, &pem, "clairtyx.dev", "mail").unwrap();
        assert_ne!(
            verdict(&signed, &zone_with_key(&other_public)).await,
            format!("{:?}", DkimResult::Pass)
        );
    }

    #[test]
    fn an_invalid_key_is_an_error() {
        assert!(
            sign(
                b"Subject: x\r\n\r\ny",
                "pas une clé",
                "clairtyx.dev",
                "mail"
            )
            .is_err()
        );
    }
}
```

</details>

<details><summary>Tests de <code>src/bounce/server.rs</code></summary>

```rust
#[cfg(test)]
mod extra_tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const PERMANENT: &str = "In-Reply-To: <abc@clairtyx.dev>\r\n\r\nFinal-Recipient: rfc822; x@exemple.fr\r\nAction: failed\r\nStatus: 5.1.1\r\nDiagnostic-Code: smtp; 550 5.1.1 User unknown\r\n";
    const TRANSIENT: &str = "Original-Message-ID: <def@clairtyx.dev>\r\nStatus: 4.2.2\r\nDiagnostic-Code: smtp; 452 mailbox full\r\n";

    #[test]
    fn a_5xx_status_is_permanent_with_its_smtp_code() {
        assert_eq!(detect_bounce_type(PERMANENT), ("permanent", Some(550)));
    }

    #[test]
    fn a_4xx_status_is_transient_with_its_smtp_code() {
        assert_eq!(detect_bounce_type(TRANSIENT), ("transient", Some(452)));
    }

    #[test]
    fn an_unreadable_dsn_defaults_to_transient_without_code() {
        assert_eq!(detect_bounce_type("n'importe quoi"), ("transient", None));
        assert_eq!(detect_bounce_type(""), ("transient", None));
    }

    #[test]
    fn a_diagnostic_code_alone_is_enough_to_call_it_permanent() {
        assert_eq!(
            detect_bounce_type("Diagnostic-Code: smtp; 554 5.7.1 refused"),
            ("permanent", Some(554))
        );
    }

    #[test]
    fn the_original_message_id_is_found_by_reliability_order() {
        assert_eq!(
            extract_original_message_id(PERMANENT).as_deref(),
            Some("<abc@clairtyx.dev>")
        );
        assert_eq!(
            extract_original_message_id(TRANSIENT).as_deref(),
            Some("<def@clairtyx.dev>")
        );
        assert_eq!(extract_original_message_id("Status: 5.0.0"), None);
        let both = "Original-Message-ID: <b@x>\r\nIn-Reply-To: <a@x>\r\n";
        assert_eq!(
            extract_original_message_id(both).as_deref(),
            Some("<a@x>"),
            "In-Reply-To prime"
        );
    }

    #[test]
    fn header_lookup_is_case_insensitive_and_ignores_empty_values() {
        assert_eq!(
            find_header("in-reply-to:   <a@x>  ", "In-Reply-To").as_deref(),
            Some("<a@x>")
        );
        assert_eq!(find_header("In-Reply-To:   \r\n", "In-Reply-To"), None);
    }

    #[test]
    fn only_codes_between_400_and_599_are_taken_from_a_diagnostic() {
        assert_eq!(
            extract_code_from_diagnostic("Diagnostic-Code: smtp; 250 ok 550"),
            Some(550)
        );
        assert_eq!(
            extract_code_from_diagnostic("Diagnostic-Code: smtp; 12345"),
            None
        );
    }

    /// Une session complète sur le vrai `handle_connection` : le DSN est reçu et
    /// acquitté. L'enregistrement en base échoue ici (pas de PostgreSQL), il est
    /// journalisé sans casser la session.
    #[tokio::test]
    async fn a_full_smtp_session_is_acknowledged() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let db = sqlx::postgres::PgPoolOptions::new()
            .acquire_timeout(std::time::Duration::from_millis(200))
            .connect_lazy("postgres://u@127.0.0.1:1/x")
            .unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            handle_connection(stream, Arc::new("mx.clairtyx.dev".to_string()), db).await
        });

        let mut client = tokio::net::TcpStream::connect(addr).await.unwrap();
        let script = format!(
            "EHLO relais.exemple.fr\r\nMAIL FROM:<>\r\nRCPT TO:<bounce@clairtyx.dev>\r\nDATA\r\n{PERMANENT}..ligne avec point\r\n.\r\nVRFY x\r\nQUIT\r\n"
        );
        client.write_all(script.as_bytes()).await.unwrap();
        let mut transcript = String::new();
        client.read_to_string(&mut transcript).await.unwrap();
        server.await.unwrap().unwrap();

        assert!(
            transcript.starts_with("220 mx.clairtyx.dev ESMTP Clairtyx Bounce Receiver"),
            "{transcript}"
        );
        for expected in [
            "250-mx.clairtyx.dev",
            "250 OK",
            "354 ",
            "250 Ok: message reçu",
            "502 Commande non implémentée",
            "221 mx.clairtyx.dev Bye",
        ] {
            assert!(
                transcript.contains(expected),
                "{expected} absent de :\n{transcript}"
            );
        }
    }
}
```

</details>

<details><summary>Tests de <code>src/mta/queue_worker.rs</code></summary>

```rust
#[cfg(test)]
mod extra_tests {
    use super::*;
    use serde_json::json;

    fn vars(value: Value) -> HashMap<String, Value> {
        payload_to_map(&value).unwrap()
    }

    #[test]
    fn backoff_doubles_from_two_minutes_with_under_a_minute_of_jitter() {
        for (attempts, base) in [(0, 60), (1, 120), (2, 240), (3, 480), (4, 960)] {
            for _ in 0..50 {
                let delay = backoff_secs(attempts);
                assert!(
                    (base..base + 60).contains(&delay),
                    "tentative {attempts} : {delay}"
                );
            }
        }
    }

    #[test]
    fn backoff_is_capped_at_24h_and_never_overflows() {
        for attempts in [11, 20, 63, 64, 1000, i32::MAX] {
            let delay = backoff_secs(attempts);
            assert!(
                (86_400..86_460).contains(&delay),
                "tentative {attempts} : {delay}"
            );
        }
    }

    #[test]
    fn only_a_json_object_is_a_valid_payload() {
        assert!(payload_to_map(&json!({ "a": 1 })).is_ok());
        for bad in [json!([1]), json!("x"), json!(null), json!(3)] {
            assert!(payload_to_map(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn display_name_prefers_firstname_and_name_then_falls_back_to_client_name() {
        assert_eq!(
            build_display_name(&vars(json!({ "firstname": "Camille", "name": "Dupont" })))
                .as_deref(),
            Some("Camille Dupont")
        );
        assert_eq!(
            build_display_name(&vars(json!({ "firstname": "Camille" }))).as_deref(),
            Some("Camille")
        );
        assert_eq!(
            build_display_name(&vars(json!({ "name": "Dupont" }))).as_deref(),
            Some("Dupont")
        );
        assert_eq!(
            build_display_name(&vars(json!({ "clientName": "Mme Dubois" }))).as_deref(),
            Some("Mme Dubois")
        );
        assert_eq!(build_display_name(&vars(json!({ "name": 3 }))), None);
        assert_eq!(build_display_name(&vars(json!({}))), None);
    }
}
```

</details>

Dépendances de test ajoutées : `rsa` (clé DKIM jetable) et `base64`.

---

## 5. Configuration

Variables lues dans `src/core/config.rs`. Les secrets viennent de l'environnement, jamais du dépôt.

| Variable | Rôle |
|---|---|
| `DATABASE_URL` | base PostgreSQL dédiée (`mail_queue`, `bounces`, `mail_templates`) |
| `KAFKA_BROKERS`, `KAFKA_GROUP_ID` | consommateur d'événements |
| `SMTP_FROM_EMAIL`, `SMTP_FROM_NAME` | expéditeur par défaut |
| `SMTP_HOST`, `SMTP_PORT` | relais (Mailpit en dev) ; absents = livraison directe par MX |
| `DKIM_PRIVATE_KEY_PATH` ou `DKIM_PRIVATE_KEY_PEM` | clé privée RSA de signature |
| `DKIM_SELECTOR`, `DKIM_DOMAIN` | enregistrement DNS `<selector>._domainkey.<domaine>` |
| `BOUNCE_LISTEN_PORT`, `BOUNCE_HOSTNAME` | serveur SMTP entrant (2525 en dev, 25 en prod) |
| `WORKER_BATCH_SIZE`, `WORKER_CONCURRENCY` | lot (500) et livraisons en parallèle (50) |
| `SERVER_PORT`, `METRICS_TOKEN`, `METRICS_ALLOW_OPEN` | sondes `/health` `/livez` `/readyz` et `/metrics` |

La clé DKIM de dev (`dkim_dev.pem`) et le `.env` du service ne sont **pas** reproduits ici.

---

## 6. Code complet du service

Fichiers lus directement dans `backend/svc-mta` au moment de la génération de ce document.

### 6.1 Partie fixe (identique pour tous les projets)

<details><summary><code>Cargo.toml</code> (46 lignes)</summary>

```toml
[package]
name = "svc-mta"
version = "0.1.0"
edition.workspace = true
default-run = "svc-mta"

[lib]
name = "svc_mta"
path = "src/lib.rs"

[[bin]]
name = "svc-mta"
path = "src/main.rs"

[dependencies]
axum = { workspace = true }
tokio = { workspace = true }
tower-http = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
sqlx = { workspace = true }
uuid = { workspace = true }
chrono = { workspace = true }
shared = { workspace = true }
async-trait = { workspace = true }
rustls = { workspace = true }
anyhow = { workspace = true }
tracing = { workspace = true }
tracing-subscriber = { workspace = true }
dotenvy = { workspace = true }
tikv-jemallocator = { workspace = true }
lettre = { workspace = true }
tera = { workspace = true }
futures = { workspace = true }
rand = { workspace = true }
hickory-resolver = { workspace = true }
mail-auth = { workspace = true }
tokio-native-tls = { workspace = true }
prometheus = { workspace = true }
sha2 = { workspace = true }
subtle = { workspace = true }
native-tls = { workspace = true }

[dev-dependencies]
tokio = { workspace = true, features = ["test-util"] }
tower = { workspace = true, features = ["util"] }
```

</details>

<details><summary><code>src/main.rs</code> (117 lignes)</summary>

```rust
use std::sync::Arc;

use anyhow::{Context, Result};
use sqlx::postgres::PgPoolOptions;
use svc_mta::core::{config::Config, consumer};
use svc_mta::handlers::metrics::MetricsAccess;
use svc_mta::mta::queue_worker::{self, WorkerState};
use svc_mta::{bounce, routes};
use tikv_jemallocator::Jemalloc;
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

// Même allocateur que les autres services du workspace.
#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

#[tokio::main]
async fn main() -> Result<()> {
    // Requis par sqlx (tls-rustls) — même init que svc-auth.
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("failed to install ring crypto provider");

    dotenvy::from_path(concat!(env!("CARGO_MANIFEST_DIR"), "/.env")).ok();

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "svc_mta=debug,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    tracing::info!(version = env!("CARGO_PKG_VERSION"), "Démarrage svc-mta");

    let config = Config::from_env().context("chargement de la configuration")?;

    // Séries métier créées à 0 avant toute mesure (voir `metrics::init`).
    svc_mta::metrics::init(consumer::TOPICS);

    let db_max_connections: u32 = std::env::var("DB_MAX_CONNECTIONS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10);
    let db = PgPoolOptions::new()
        .max_connections(db_max_connections)
        .min_connections(1)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .connect(&config.database_url)
        .await
        .context("connexion PostgreSQL")?;

    sqlx::migrate!("./migrations")
        .run(&db)
        .await
        .context("migrations SQL")?;

    // Remet les jobs bloqués en `sending` à `pending` (reprise après crash).
    let stale = queue_worker::reset_stale_sending(&db).await?;
    if stale > 0 {
        tracing::warn!(count = stale, "jobs 'sending' réinitialisés à 'pending'");
    }

    let config = Arc::new(config);

    // ── Tâches de fond ────────────────────────────────────────────────────────
    // 1. Consumer Kafka — insère les events dans mail_queue (idempotent).
    // 2. Queue worker  — livre les jobs en attente (SMTP raw + DKIM).
    // 3. Bounce SMTP   — reçoit les NDR/DSN entrants et met à jour la file.
    let kafka_handle = consumer::spawn(&config.kafka, db.clone())?;

    let worker_handle = {
        let state = WorkerState {
            db: db.clone(),
            config: config.clone(),
        };
        tokio::spawn(async move { queue_worker::run(state).await })
    };

    let bounce_handle = {
        let addr = config.bounce_addr();
        let hostname = Arc::new(config.bounce_hostname.clone());
        let db2 = db.clone();
        tokio::spawn(async move {
            loop {
                if let Err(e) = bounce::server::run(&addr, hostname.clone(), db2.clone()).await {
                    tracing::error!(error = %e, "serveur bounce arrêté, redémarrage dans 5s");
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                }
            }
        })
    };

    // ── Serveur HTTP (health / readiness / metrics) ─────────────────────────────
    let http_addr = format!("0.0.0.0:{}", config.server_port);
    let metrics_access =
        MetricsAccess::new(config.metrics_token.as_deref(), config.metrics_allow_open);
    let app = routes::build(db.clone(), metrics_access).layer(TraceLayer::new_for_http());
    let listener = tokio::net::TcpListener::bind(&http_addr)
        .await
        .with_context(|| format!("bind HTTP {http_addr}"))?;
    tracing::info!(
        "svc-mta HTTP sur {http_addr} — kafka: {}",
        config.kafka.brokers
    );
    let http_handle = tokio::spawn(async move { axum::serve(listener, app).await });

    // Attendre qu'une tâche se termine (crash non géré).
    tokio::select! {
        res = kafka_handle  => tracing::error!("tâche kafka terminée : {res:?}"),
        res = worker_handle => tracing::error!("tâche worker terminée : {res:?}"),
        res = bounce_handle => tracing::error!("tâche bounce terminée : {res:?}"),
        res = http_handle   => tracing::error!("serveur HTTP terminé : {res:?}"),
    }

    Ok(())
}
```

</details>

<details><summary><code>src/lib.rs</code> (9 lignes)</summary>

```rust
pub mod bounce;
pub mod core;
pub mod handlers;
pub mod message;
pub mod metrics;
pub mod middlewares;
pub mod mta;
pub mod routes;
pub mod template;
```

</details>

<details><summary><code>src/core/mod.rs</code> (2 lignes)</summary>

```rust
pub mod config;
pub mod consumer;
```

</details>

<details><summary><code>src/core/config.rs</code> (114 lignes)</summary>

```rust
use anyhow::{Context, Result};
use shared::kafka::ConsumerConfig;
use std::env;

/// Configuration du service MTA (svc-mta).
///
/// Chargée depuis les variables d'environnement. Le `.env` du crate est lu en
/// amont dans `main` (`dotenvy::from_path`), comme svc-auth.
#[derive(Clone, Debug)]
pub struct Config {
    // ── Serveur HTTP (health / readiness / metrics) ───────────────────────────
    pub server_port: u16,
    /// Jeton Bearer attendu sur GET /metrics (METRICS_TOKEN).
    pub metrics_token: Option<String>,
    /// Dérogation explicite : /metrics ouvert sans jeton (METRICS_ALLOW_OPEN).
    pub metrics_allow_open: bool,

    // ── Base de données (file mail_queue + bounces) ───────────────────────────
    pub database_url: String,

    // ── Event bus (Kafka) ─────────────────────────────────────────────────────
    /// `KAFKA_BROKERS` et `KAFKA_GROUP_ID`, obligatoires.
    pub kafka: ConsumerConfig,

    // ── Expéditeur ────────────────────────────────────────────────────────────
    pub smtp_from_email: String,
    pub smtp_from_name: String,

    // ── Relay SMTP (smarthost) ────────────────────────────────────────────────
    /// Si défini, tous les emails sont livrés via ce relay (ex: Mailpit en dev)
    /// au lieu d'un lookup DNS MX direct. Défini via SMTP_HOST / SMTP_PORT.
    pub smtp_relay_host: Option<String>,
    pub smtp_relay_port: u16,

    // ── DKIM ──────────────────────────────────────────────────────────────────
    /// Contenu PEM de la clé privée RSA ou ED25519.
    pub dkim_private_key_pem: String,
    /// Sélecteur DNS (ex: "mail", "2026").
    pub dkim_selector: String,
    /// Domaine signataire (ex: "clairtyx.dev").
    pub dkim_domain: String,

    // ── Serveur SMTP entrant (bounces) ────────────────────────────────────────
    /// Port d'écoute pour les NDR/DSN entrants (défaut : 2525 en dev, 25 en prod).
    pub bounce_listen_port: u16,
    /// Nom d'hôte annoncé par le serveur SMTP entrant (banner 220).
    pub bounce_hostname: String,

    // ── Worker ────────────────────────────────────────────────────────────────
    /// Nombre de jobs chargés par itération (WORKER_BATCH_SIZE, défaut 500).
    pub worker_batch_size: u32,
    /// Nombre de jobs traités en parallèle (WORKER_CONCURRENCY, défaut 50).
    pub worker_concurrency: usize,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        // ── Clé privée DKIM : chemin de fichier OU valeur PEM directe ──────────
        let dkim_private_key_pem = if let Ok(path) = env::var("DKIM_PRIVATE_KEY_PATH") {
            std::fs::read_to_string(&path)
                .with_context(|| format!("DKIM : lecture de la clé depuis '{path}'"))?
        } else {
            env::var("DKIM_PRIVATE_KEY_PEM")
                .context("DKIM_PRIVATE_KEY_PEM ou DKIM_PRIVATE_KEY_PATH requis")?
        };

        Ok(Config {
            server_port: env::var("SERVER_PORT")
                .unwrap_or_else(|_| "4006".into())
                .parse()
                .context("SERVER_PORT invalide")?,
            metrics_token: env::var("METRICS_TOKEN").ok().filter(|s| !s.is_empty()),
            metrics_allow_open: env::var("METRICS_ALLOW_OPEN")
                .map(|v| v == "true" || v == "1")
                .unwrap_or(false),

            database_url: env::var("DATABASE_URL").context("DATABASE_URL manquant")?,

            kafka: ConsumerConfig::from_env()?,

            smtp_from_email: env::var("SMTP_FROM_EMAIL").context("SMTP_FROM_EMAIL manquant")?,
            smtp_from_name: env::var("SMTP_FROM_NAME").context("SMTP_FROM_NAME manquant")?,

            smtp_relay_host: env::var("SMTP_HOST").ok(),
            smtp_relay_port: env::var("SMTP_PORT")
                .unwrap_or_else(|_| "25".into())
                .parse()
                .context("SMTP_PORT invalide")?,

            dkim_private_key_pem,
            dkim_selector: env::var("DKIM_SELECTOR").context("DKIM_SELECTOR manquant")?,
            dkim_domain: env::var("DKIM_DOMAIN").context("DKIM_DOMAIN manquant")?,

            bounce_listen_port: env::var("BOUNCE_LISTEN_PORT")
                .unwrap_or_else(|_| "2525".into())
                .parse()
                .context("BOUNCE_LISTEN_PORT invalide")?,
            bounce_hostname: env::var("BOUNCE_HOSTNAME").unwrap_or_else(|_| "clairtyx.dev".into()),

            worker_batch_size: env::var("WORKER_BATCH_SIZE")
                .unwrap_or_else(|_| "500".into())
                .parse()
                .context("WORKER_BATCH_SIZE invalide")?,
            worker_concurrency: env::var("WORKER_CONCURRENCY")
                .unwrap_or_else(|_| "50".into())
                .parse()
                .context("WORKER_CONCURRENCY invalide")?,
        })
    }

    pub fn bounce_addr(&self) -> String {
        format!("0.0.0.0:{}", self.bounce_listen_port)
    }
}
```

</details>

<details><summary><code>src/metrics.rs</code> (138 lignes)</summary>

```rust
//! Métriques Prometheus métier de svc-mta : consumer Kafka, livraison SMTP,
//! bounces et état de la file `mail_queue`. Exposées sur GET /metrics avec les
//! métriques HTTP (registre global). Labels à valeurs bornées uniquement
//! (topics, types d'événements, statuts) : jamais d'adresse e-mail.

use std::sync::LazyLock;

use prometheus::{
    Histogram, IntCounterVec, IntGaugeVec, register_histogram, register_int_counter_vec,
    register_int_gauge_vec,
};
use sqlx::PgPool;

/// Messages Kafka traités, par topic et issue (`enqueued`, `duplicate`,
/// `invalid`, `error`).
pub static KAFKA_MESSAGES: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "mta_kafka_messages_total",
        "Kafka messages processed by topic and outcome",
        &["topic", "outcome"]
    )
    .expect("register mta_kafka_messages_total")
});

/// Tentatives de livraison SMTP, par type d'e-mail et issue (`sent`, `retry`,
/// `failed` = abandon après max_attempts).
pub static DELIVERIES: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "mta_deliveries_total",
        "SMTP delivery attempts by event type and outcome",
        &["event_type", "outcome"]
    )
    .expect("register mta_deliveries_total")
});

/// Durée d'une tentative de livraison (rendu, DKIM, résolution MX, SMTP).
pub static DELIVERY_DURATION: LazyLock<Histogram> = LazyLock::new(|| {
    register_histogram!(
        "mta_delivery_duration_seconds",
        "Duration of one SMTP delivery attempt",
        vec![0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0]
    )
    .expect("register mta_delivery_duration_seconds")
});

/// Bounces (NDR/DSN) reçus, par type (`hard`, `soft`...).
pub static BOUNCES: LazyLock<IntCounterVec> = LazyLock::new(|| {
    register_int_counter_vec!(
        "mta_bounces_total",
        "Bounces received by bounce type",
        &["bounce_type"]
    )
    .expect("register mta_bounces_total")
});

/// Nombre de jobs dans `mail_queue` par statut (les jobs livrés sont
/// supprimés, `sent` reste donc à 0).
pub static QUEUE_JOBS: LazyLock<IntGaugeVec> = LazyLock::new(|| {
    register_int_gauge_vec!(
        "mta_queue_jobs",
        "Jobs currently in mail_queue by status",
        &["status"]
    )
    .expect("register mta_queue_jobs")
});

/// Valeurs de l'enum PostgreSQL `mail_status`.
const STATUSES: &[&str] = &["pending", "sending", "sent", "failed", "bounced"];

/// Crée à 0 toutes les séries métier aux labels connus d'avance. Sans ça, une
/// série naîtrait à sa première mesure et Prometheus la découvrirait déjà à 1
/// (ou plus) : `increase()` / `rate()` ne compteraient jamais ces premières
/// mesures. `topics` sert aussi de liste des types d'e-mails (`event_type` =
/// topic Kafka d'origine).
pub fn init(topics: &[&str]) {
    for topic in topics.iter().copied().chain(["other"]) {
        for outcome in ["enqueued", "duplicate", "invalid", "error"] {
            KAFKA_MESSAGES.with_label_values(&[topic, outcome]);
        }
    }
    for &event_type in topics {
        for outcome in ["sent", "retry", "failed"] {
            DELIVERIES.with_label_values(&[event_type, outcome]);
        }
    }
    // Valeurs renvoyées par `bounce::server::detect_bounce_type`.
    for bounce_type in ["permanent", "transient"] {
        BOUNCES.with_label_values(&[bounce_type]);
    }
    LazyLock::force(&DELIVERY_DURATION);
    for status in STATUSES {
        QUEUE_JOBS.with_label_values(&[status]);
    }
}

/// Rafraîchit `mta_queue_jobs` depuis la base. Les statuts absents du
/// résultat sont remis à 0 pour ne pas garder une ancienne valeur.
pub async fn refresh_queue_jobs(db: &PgPool) -> sqlx::Result<()> {
    let rows: Vec<(String, i64)> =
        sqlx::query_as("SELECT status::text, COUNT(*) FROM mail_queue GROUP BY status")
            .fetch_all(db)
            .await?;
    for status in STATUSES {
        let count = rows
            .iter()
            .find(|(s, _)| s == status)
            .map_or(0, |(_, c)| *c);
        QUEUE_JOBS.with_label_values(&[status]).set(count);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_exposes_known_series_at_zero() {
        init(&["auth.user.registered"]);
        assert_eq!(
            DELIVERIES
                .with_label_values(&["auth.user.registered", "sent"])
                .get(),
            0
        );
        let families = prometheus::gather();
        let names: Vec<_> = families.iter().map(|f| f.name()).collect();
        for name in [
            "mta_kafka_messages_total",
            "mta_deliveries_total",
            "mta_bounces_total",
            "mta_delivery_duration_seconds",
            "mta_queue_jobs",
        ] {
            assert!(names.contains(&name), "{name} absent de /metrics");
        }
    }
}
```

</details>

<details><summary><code>src/middlewares/mod.rs</code> (1 lignes)</summary>

```rust
pub mod metrics;
```

</details>

<details><summary><code>src/middlewares/metrics.rs</code> (238 lignes)</summary>

```rust
//! Instrumentation HTTP Prometheus partagée par les services Clairtyx.
//!
//! Mêmes noms de métriques (`http_requests_total`...) et mêmes labels dans
//! chaque service : Prometheus ajoute le label `service` au scrape, un seul
//! dashboard Grafana couvre donc svc-auth, svc-connect, svc-mta et svc-realtime. Fichier
//! dupliqué dans chaque service : toute modification se reporte dans chacun.

use axum::{
    extract::{MatchedPath, Request},
    middleware::Next,
    response::Response,
};
use prometheus::{
    Encoder, HistogramVec, IntCounterVec, IntGauge, TextEncoder, register_histogram_vec,
    register_int_counter_vec, register_int_gauge,
};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, PoisonError};
use subtle::ConstantTimeEq;
use tokio::time::{Duration, Instant};

/// Compteurs `http_*`, registre global, construits une seule fois.
static METRICS: LazyLock<HttpMetrics> = LazyLock::new(HttpMetrics::register);

/// Délai pendant lequel les mesures d'une série neuve sont différées.
///
/// Une série (méthode, route, statut) n'existe qu'à partir de sa première
/// requête, et Prometheus ne la découvre qu'au scrape suivant : si 2 requêtes
/// arrivent entre deux scrapes, la série apparaît directement à 2, sans point
/// à 0, et `increase()` / `rate()` ne comptent jamais ces 2 requêtes. La série
/// est donc créée à 0 tout de suite, et ses mesures ne sont appliquées qu'après
/// ce délai, une fois que Prometheus l'a vue à 0. Doit dépasser deux fois
/// l'intervalle de scrape (30 s, deploy/infra/monitoring/prometheus.yaml).
const NEW_SERIES_DELAY: Duration = Duration::from_secs(65);

pub async fn metrics_middleware(request: Request, next: Next) -> Response {
    METRICS.track(request, next).await
}

/// Compteurs HTTP d'un service. À construire UNE fois (LazyLock) : le
/// registre Prometheus est global, un double register paniquerait.
pub struct HttpMetrics {
    requests: IntCounterVec,
    duration: HistogramVec,
    in_flight: IntGauge,
    /// Instant de création de chaque série (méthode, route, statut).
    born: Mutex<HashMap<(String, String, String), Instant>>,
}

impl HttpMetrics {
    pub fn register() -> Self {
        Self {
            requests: register_int_counter_vec!(
                "http_requests_total",
                "HTTP requests by method, matched route and status code",
                &["method", "path", "status"]
            )
            .expect("register requests counter"),
            duration: register_histogram_vec!(
                "http_request_duration_seconds",
                "HTTP request latency by method, matched route and status code",
                &["method", "path", "status"],
                vec![
                    0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0
                ]
            )
            .expect("register duration histogram"),
            in_flight: register_int_gauge!(
                "http_requests_in_flight",
                "HTTP requests currently being processed"
            )
            .expect("register in-flight gauge"),
            born: Mutex::new(HashMap::new()),
        }
    }

    /// Corps du middleware axum. MatchedPath (template de route, ex:
    /// /me/profil) plutôt que l'URI brute : cardinalité bornée, pas
    /// d'explosion de labels Prometheus.
    pub async fn track(&'static self, request: Request, next: Next) -> Response {
        let path = request
            .extensions()
            .get::<MatchedPath>()
            .map(|p| p.as_str().to_owned())
            .unwrap_or_else(|| "unmatched".to_owned());
        let method = request.method().to_string();

        self.in_flight.inc();
        let start = Instant::now();
        let response = next.run(request).await;
        self.in_flight.dec();

        let status = response.status().as_u16().to_string();
        self.record(method, path, status, start.elapsed().as_secs_f64());

        response
    }

    /// Compte une requête. Pour une série née il y a moins de
    /// `NEW_SERIES_DELAY`, la mesure est différée (voir la constante).
    fn record(&'static self, method: String, path: String, status: String, seconds: f64) {
        let now = Instant::now();
        let born = *self
            .born
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry((method.clone(), path.clone(), status.clone()))
            .or_insert_with(|| {
                // Séries créées à 0 : visibles telles quelles au prochain scrape.
                let labels = [method.as_str(), path.as_str(), status.as_str()];
                self.requests.with_label_values(&labels);
                self.duration.with_label_values(&labels);
                now
            });

        let apply = move || {
            let labels = [method.as_str(), path.as_str(), status.as_str()];
            self.requests.with_label_values(&labels).inc();
            self.duration.with_label_values(&labels).observe(seconds);
        };
        let ready_at = born + NEW_SERIES_DELAY;
        if ready_at <= now {
            apply();
        } else {
            tokio::spawn(async move {
                tokio::time::sleep_until(ready_at).await;
                apply();
            });
        }
    }
}

/// Contrôle d'accès à GET /metrics : Bearer comparé en temps constant au hash
/// SHA-256 attendu. `expected_hash = None` (METRICS_TOKEN absent) → **fail-closed**
/// (refus), sauf dérogation explicite `allow_open` (METRICS_ALLOW_OPEN=true). Un
/// oubli de token n'expose donc jamais les compteurs en silence.
pub fn is_authorized(
    headers: &axum::http::HeaderMap,
    expected_hash: Option<&[u8; 32]>,
    allow_open: bool,
) -> bool {
    let Some(expected) = expected_hash else {
        return allow_open;
    };
    let Some(token) = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
    else {
        return false;
    };
    let provided_hash: [u8; 32] = Sha256::digest(token.as_bytes()).into();
    provided_hash.ct_eq(expected).into()
}

/// Sérialise le registre Prometheus global au format texte d'exposition.
pub fn render() -> (&'static str, Vec<u8>) {
    let mut buffer = Vec::new();
    TextEncoder::new()
        .encode(&prometheus::gather(), &mut buffer)
        .unwrap_or_default();
    ("text/plain; version=0.0.4; charset=utf-8", buffer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderValue, header};

    fn hash(token: &str) -> [u8; 32] {
        Sha256::digest(token.as_bytes()).into()
    }

    fn bearer(token: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
        );
        headers
    }

    #[tokio::test(start_paused = true)]
    async fn new_series_starts_at_zero_then_catches_up() {
        let labels = ["GET", "/test/new-series", "200"];
        let count = || METRICS.requests.with_label_values(&labels).get();
        let record = || {
            METRICS.record(
                labels[0].to_owned(),
                labels[1].to_owned(),
                labels[2].to_owned(),
                0.01,
            )
        };

        // Série neuve : exposée à 0, les 2 requêtes sont différées.
        record();
        record();
        assert_eq!(count(), 0);

        tokio::time::sleep(NEW_SERIES_DELAY + Duration::from_secs(1)).await;
        assert_eq!(count(), 2);

        // Série déjà vue par Prometheus : comptée immédiatement.
        record();
        assert_eq!(count(), 3);
    }

    #[test]
    fn no_token_fails_closed_by_default() {
        // METRICS_TOKEN absent + pas de dérogation → refus.
        assert!(!is_authorized(&HeaderMap::new(), None, false));
    }

    #[test]
    fn no_token_open_only_with_explicit_optin() {
        // METRICS_ALLOW_OPEN=true → ouvert (dev / réseau cloisonné).
        assert!(is_authorized(&HeaderMap::new(), None, true));
    }

    #[test]
    fn correct_token_accepted() {
        let expected = hash("scrape-token");
        assert!(is_authorized(
            &bearer("scrape-token"),
            Some(&expected),
            false
        ));
    }

    #[test]
    fn wrong_or_missing_token_rejected() {
        let expected = hash("scrape-token");
        assert!(!is_authorized(&bearer("autre"), Some(&expected), false));
        assert!(!is_authorized(&HeaderMap::new(), Some(&expected), false));
    }
}
```

</details>

<details><summary><code>src/routes/mod.rs</code> (78 lignes)</summary>

```rust
use std::sync::Arc;

use crate::handlers::{self, metrics::MetricsAccess};
use crate::middlewares::metrics::metrics_middleware;
use axum::{Router, middleware, routing::get};
use sqlx::PgPool;

/// Routeur HTTP minimal : svc-mta n'expose aucune route métier (il consomme
/// Kafka et livre par SMTP). Seules les sondes de santé et GET /metrics sont
/// publiées, alignées sur svc-auth.
pub fn build(db: PgPool, metrics: MetricsAccess) -> Router {
    let metrics_router = Router::new()
        .route("/metrics", get(handlers::metrics::metrics))
        .with_state(Arc::new(metrics));

    Router::new()
        .route("/health", get(handlers::health::health))
        .route("/livez", get(handlers::health::livez))
        .route("/readyz", get(handlers::health::readyz))
        .with_state(db)
        .merge(metrics_router)
        .layer(middleware::from_fn(metrics_middleware))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode, header},
    };
    use sqlx::postgres::PgPoolOptions;
    use tower::ServiceExt;

    /// Pool paresseux : aucune connexion n'est ouverte tant qu'aucune requête
    /// SQL ne part, /livez et /metrics n'en font pas.
    fn app() -> Router {
        let db = PgPoolOptions::new()
            .connect_lazy("postgres://test@localhost/test")
            .expect("pool paresseux");
        build(db, MetricsAccess::new(Some("scrape-token"), false))
    }

    fn get(uri: &str, token: Option<&str>) -> Request<Body> {
        let mut req = Request::get(uri);
        if let Some(t) = token {
            req = req.header(header::AUTHORIZATION, format!("Bearer {t}"));
        }
        req.body(Body::empty()).unwrap()
    }

    #[tokio::test]
    async fn metrics_requires_token() {
        let res = app().oneshot(get("/metrics", None)).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

        let res = app().oneshot(get("/metrics", Some("autre"))).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn metrics_exposes_http_counters() {
        let app = app();
        app.clone().oneshot(get("/livez", None)).await.unwrap();

        let res = app
            .oneshot(get("/metrics", Some("scrape-token")))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let body = String::from_utf8(body.to_vec()).unwrap();
        // Série neuve : exposée tout de suite (à 0, la mesure est différée).
        assert!(body.contains(r#"http_requests_total{method="GET",path="/livez",status="200"}"#));
        // Feature `process` du crate prometheus : mémoire/CPU du process.
        assert!(body.contains("process_resident_memory_bytes"));
    }
}
```

</details>

<details><summary><code>src/handlers/mod.rs</code> (2 lignes)</summary>

```rust
pub mod health;
pub mod metrics;
```

</details>

<details><summary><code>src/handlers/health.rs</code> (41 lignes)</summary>

```rust
use axum::{Json, extract::State, http::StatusCode};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::time::Duration;

const READYZ_DEP_TIMEOUT: Duration = Duration::from_secs(2);

/// Liveness : le process répond. Aucune dépendance externe — un échec doit
/// déclencher un restart du pod, pas un retrait du load balancer.
pub async fn livez() -> Json<Value> {
    Json(json!({"status": "alive"}))
}

/// Alias historique de `livez` (healthcheck Docker `curl /health`).
pub async fn health() -> Json<Value> {
    Json(json!({"status": "ok"}))
}

/// Readiness : sonde réelle de Postgres (file `mail_queue` / `bounces`), seule
/// dépendance synchrone critique. Kafka volontairement exclu : le consumer
/// rattrape son retard tout seul.
pub async fn readyz(State(db): State<PgPool>) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    match tokio::time::timeout(READYZ_DEP_TIMEOUT, sqlx::query("SELECT 1").execute(&db)).await {
        Ok(Ok(_)) => Ok(Json(json!({"status": "ready"}))),
        Ok(Err(e)) => {
            tracing::warn!("readyz: postgres error: {e}");
            Err(unavailable("database"))
        }
        Err(_) => {
            tracing::warn!("readyz: postgres timeout (> {READYZ_DEP_TIMEOUT:?})");
            Err(unavailable("database timeout"))
        }
    }
}

fn unavailable(dep: &str) -> (StatusCode, Json<Value>) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({"status": "unavailable", "dependency": dep})),
    )
}
```

</details>

<details><summary><code>src/handlers/metrics.rs</code> (37 lignes)</summary>

```rust
use std::sync::Arc;

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use sha2::{Digest, Sha256};

use crate::middlewares::metrics::{is_authorized, render};

/// Contrôle d'accès de GET /metrics, construit une fois depuis la config.
#[derive(Clone, Debug)]
pub struct MetricsAccess {
    pub token_hash: Option<[u8; 32]>,
    pub allow_open: bool,
}

impl MetricsAccess {
    pub fn new(token: Option<&str>, allow_open: bool) -> Self {
        Self {
            token_hash: token.map(|t| Sha256::digest(t.as_bytes()).into()),
            allow_open,
        }
    }
}

/// Endpoint de scraping Prometheus. Protégé par `METRICS_TOKEN` (Bearer),
/// fail-closed si le token est absent sauf `METRICS_ALLOW_OPEN=true`.
pub async fn metrics(State(access): State<Arc<MetricsAccess>>, headers: HeaderMap) -> Response {
    if !is_authorized(&headers, access.token_hash.as_ref(), access.allow_open) {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    let (content_type, body) = render();
    ([(header::CONTENT_TYPE, content_type)], body).into_response()
}
```

</details>

<details><summary><code>src/message/mod.rs</code> (115 lignes)</summary>

```rust
use anyhow::{Context, Result};
use lettre::{
    Address, Message,
    message::{Mailbox, MultiPart, SinglePart},
};
use uuid::Uuid;

/// Message MIME construit et prêt à être signé puis livré.
pub struct OutgoingMessage {
    /// Bytes bruts RFC 5322 (avec DKIM-Signature préfixé ensuite)
    pub raw: Vec<u8>,
    /// Message-ID unique pour le suivi en BDD et corrélation des bounces
    /// Format : `<uuid@domain>`
    pub message_id: String,
}

/// Construit un message MIME multipart/alternative (HTML + texte).
///
/// # Paramètres
/// - `from_email` / `from_name` : expéditeur
/// - `to_email` / `to_name`   : destinataire
/// - `reply_to`               : adresse de réponse, si différente de l'expéditeur
///   (retard signalé : adresse professionnelle du patron — voir
///   `services::companies`, monolithe)
/// - `subject`                : sujet
/// - `html` / `text`          : corps (Tera déjà rendu)
/// - `dkim_domain`            : utilisé pour générer le Message-ID
#[allow(clippy::too_many_arguments)]
pub fn build(
    from_email: &str,
    from_name: &str,
    to_email: &str,
    to_name: Option<&str>,
    reply_to: Option<&str>,
    subject: &str,
    html: &str,
    text: &str,
    dkim_domain: &str,
) -> Result<OutgoingMessage> {
    let message_id = format!("<{}@{}>", Uuid::new_v4(), dkim_domain);

    let from_addr: Address = from_email.parse().context("adresse expéditeur invalide")?;
    let to_addr: Address = to_email.parse().context("adresse destinataire invalide")?;

    let from_box = Mailbox::new(Some(from_name.to_string()), from_addr);
    let to_box = Mailbox::new(to_name.map(|s| s.to_string()), to_addr);

    let mut builder = Message::builder()
        .from(from_box)
        .to(to_box)
        .subject(subject)
        .message_id(Some(message_id.clone()));

    if let Some(reply_to) = reply_to {
        let reply_to_addr: Address = reply_to.parse().context("adresse Reply-To invalide")?;
        builder = builder.reply_to(Mailbox::new(None, reply_to_addr));
    }

    let msg = builder
        .multipart(
            MultiPart::alternative()
                .singlepart(SinglePart::plain(text.to_string()))
                .singlepart(SinglePart::html(html.to_string())),
        )
        .context("construction du message MIME échouée")?;

    let raw = msg.formatted();

    Ok(OutgoingMessage { raw, message_id })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Retard signalé : le client final répond directement au patron
    /// (adresse professionnelle), pas à l'adresse technique d'expédition.
    #[test]
    fn reply_to_header_is_set_when_provided() {
        let msg = build(
            "notifications@clairtyx.com",
            "Dupont Plomberie via Clairtyx",
            "client@exemple.fr",
            None,
            Some("contact@dupont-plomberie.fr"),
            "Retard signalé",
            "<p>html</p>",
            "text",
            "clairtyx.com",
        )
        .unwrap();
        let raw = String::from_utf8(msg.raw).unwrap();
        assert!(raw.contains("Reply-To: contact@dupont-plomberie.fr"));
    }

    /// Sans adresse professionnelle connue : pas d'en-tête `Reply-To` du
    /// tout, jamais une valeur devinée.
    #[test]
    fn reply_to_header_is_absent_without_a_contact_email() {
        let msg = build(
            "notifications@clairtyx.com",
            "Clairtyx",
            "client@exemple.fr",
            None,
            None,
            "Bienvenue",
            "<p>html</p>",
            "text",
            "clairtyx.com",
        )
        .unwrap();
        let raw = String::from_utf8(msg.raw).unwrap();
        assert!(!raw.contains("Reply-To"));
    }
}
```

</details>

<details><summary><code>src/mta/mod.rs</code> (4 lignes)</summary>

```rust
pub mod dkim;
pub mod dns;
pub mod queue_worker;
pub mod smtp;
```

</details>

<details><summary><code>src/mta/dns.rs</code> (75 lignes)</summary>

```rust
use anyhow::{Context, Result};
use hickory_resolver::{TokioResolver, proto::rr::rdata::MX};

/// Un enregistrement MX : serveur + priorité (plus basse = plus prioritaire)
#[derive(Debug, Clone)]
pub struct MxRecord {
    pub priority: u16,
    pub host: String,
}

/// Construit le resolver en utilisant la configuration DNS du système (/etc/resolv.conf).
fn make_resolver() -> Result<TokioResolver> {
    // builder_tokio() lit /etc/resolv.conf pour utiliser la config DNS du système
    TokioResolver::builder_tokio()
        .context("DNS : initialisation du resolver système")
        .map(|b| b.build())
}

/// Résout les enregistrements MX d'un domaine destinataire.
///
/// Essaie d'abord les MX ; si aucun n'existe, tente un A/AAAA (RFC 5321 §5).
/// Le résultat est trié par priorité croissante.
pub async fn resolve_mx(domain: &str) -> Result<Vec<MxRecord>> {
    let resolver = make_resolver()?;

    match resolver.mx_lookup(domain).await {
        Ok(response) => {
            let mut records: Vec<MxRecord> = response
                .iter()
                .map(|mx: &MX| MxRecord {
                    priority: mx.preference(),
                    host: mx.exchange().to_string().trim_end_matches('.').to_string(),
                })
                .collect();

            if records.is_empty() {
                tracing::warn!(domain, "Aucun enregistrement MX — fallback sur A/AAAA");
                return fallback_a(domain).await;
            }

            records.sort_by_key(|r| r.priority);
            tracing::debug!(domain, count = records.len(), "MX résolus");
            Ok(records)
        }
        Err(e) => {
            tracing::warn!(domain, error = %e, "MX lookup échoué — tentative fallback A");
            fallback_a(domain).await
        }
    }
}

/// Retourne un MX synthétique pointant directement sur le domaine
/// (fallback RFC 5321 si aucun MX n'est publié).
async fn fallback_a(domain: &str) -> Result<Vec<MxRecord>> {
    let resolver = make_resolver()?;

    resolver
        .lookup_ip(domain)
        .await
        .with_context(|| format!("DNS : impossible de résoudre '{domain}' (ni MX ni A/AAAA)"))?;

    // Si la résolution A réussit, on retourne le domaine lui-même comme hôte SMTP
    Ok(vec![MxRecord {
        priority: 0,
        host: domain.trim_end_matches('.').to_string(),
    }])
}

/// Extrait le domaine d'une adresse email ("user@domain.tld" → "domain.tld")
pub fn domain_of(email: &str) -> Result<&str> {
    email
        .rsplit_once('@')
        .map(|(_, d)| d)
        .ok_or_else(|| anyhow::anyhow!("adresse email invalide : '{email}'"))
}
```

</details>

<details><summary><code>src/mta/dkim.rs</code> (54 lignes)</summary>

```rust
//! Signature DKIM des messages RFC 5322 (RFC 6376).
//!
//! Utilise le crate `mail-auth` pour signer avec RSA-2048/SHA-256
//! ou ED25519 selon la clé fournie.
//!
//! La signature DKIM-Signature: est **préfixée** au message brut
//! car certains serveurs vérifient l'ordre des en-têtes.

use anyhow::Result;
use mail_auth::common::headers::HeaderWriter;
use mail_auth::{
    common::crypto::{RsaKey, Sha256},
    dkim::DkimSigner,
};

/// Signe un message RFC 5322 et retourne le message avec le header
/// `DKIM-Signature:` correctement positionné en tête.
///
/// # Paramètres
/// - `raw`         : bytes du message RFC 5322 (sans DKIM, produit par `message::build`)
/// - `pem`         : clé privée RSA au format PEM
/// - `domain`      : domaine signataire (ex: "clairtyx.dev")
/// - `selector`    : sélecteur DNS (ex: "mail" → TXT mail._domainkey.clairtyx.dev)
pub fn sign(raw: &[u8], pem: &str, domain: &str, selector: &str) -> Result<Vec<u8>> {
    // from_rsa_pem est déprécié au profit de from_key_der (entrée DER) ; on
    // garde l'entrée PEM telle quelle — convertir en DER serait du churn pur.
    #[allow(deprecated)]
    let rsa_key = RsaKey::<Sha256>::from_rsa_pem(pem)
        .map_err(|e| anyhow::anyhow!("DKIM : chargement clé RSA échoué — {e:?}"))?;

    let signed = DkimSigner::from_key(rsa_key)
        .domain(domain)
        .selector(selector)
        .headers([
            "From",
            "To",
            "Subject",
            "Date",
            "Message-ID",
            "MIME-Version",
            "Content-Type",
        ])
        .sign(raw)
        .map_err(|e| anyhow::anyhow!("DKIM : signature échouée — {e:?}"))?;

    // `to_header()` retourne la ligne "DKIM-Signature: ...\r\n"
    let dkim_header = signed.to_header();
    tracing::debug!(header = %dkim_header.trim(), "DKIM-Signature ajoutée");

    let mut result = Vec::with_capacity(dkim_header.len() + raw.len());
    result.extend_from_slice(dkim_header.as_bytes());
    result.extend_from_slice(raw);
    Ok(result)
}
```

</details>

<details><summary><code>src/mta/smtp.rs</code> (247 lignes)</summary>

```rust
//! Client SMTP raw (RFC 5321) avec STARTTLS optionnel.
//!
//! Ce module implémente un client SMTP minimal capable de livrer un message
//! déjà formaté RFC 5322 (avec DKIM-Signature) directement à un serveur MX cible.
//!
//! Séquence :
//!   TCP connect → EHLO → [STARTTLS + EHLO] → MAIL FROM → RCPT TO → DATA → QUIT

use anyhow::{Context, Result, bail};
use native_tls::TlsConnector;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::TcpStream,
    time::{Duration, timeout},
};
use tokio_native_tls::TlsStream;

/// Timeout par commande SMTP (RFC 5321 §4.5.3.2)
const CMD_TIMEOUT: Duration = Duration::from_secs(30);
/// Timeout connexion TCP initiale
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

// ─── Lecture de réponses SMTP ─────────────────────────────────────────────────

enum Transport {
    Plain(BufReader<TcpStream>),
    Tls(BufReader<TlsStream<TcpStream>>),
}

impl Transport {
    async fn read_reply(&mut self) -> Result<(u16, String)> {
        let reply = match self {
            Transport::Plain(r) => read_multiline(r).await?,
            Transport::Tls(r) => read_multiline(r).await?,
        };
        Ok(reply)
    }

    async fn write_all(&mut self, buf: &[u8]) -> Result<()> {
        match self {
            Transport::Plain(r) => {
                r.get_mut().write_all(buf).await?;
                r.get_mut().flush().await?;
            }
            Transport::Tls(r) => {
                r.get_mut().write_all(buf).await?;
                r.get_mut().flush().await?;
            }
        }
        Ok(())
    }
}

/// Lit une réponse SMTP multi-ligne.
/// Format attendu : `NNN-text\r\n` (continuation) ou `NNN text\r\n` (fin).
async fn read_multiline<R>(reader: &mut BufReader<R>) -> Result<(u16, String)>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut full = String::new();
    // Réassigné à chaque ligne avant lecture ; l'init ne sert qu'à satisfaire
    // l'analyse de flux (la boucle assigne toujours avant `break`).
    #[allow(unused_assignments)]
    let mut code: u16 = 0;
    loop {
        let mut line = String::new();
        timeout(CMD_TIMEOUT, reader.read_line(&mut line))
            .await
            .context("SMTP : timeout lecture réponse")?
            .context("SMTP : lecture réponse")?;

        if line.is_empty() {
            bail!("SMTP : connexion fermée inopinément");
        }
        if line.len() < 3 {
            bail!("SMTP : réponse malformée : {:?}", line);
        }

        let line_code: u16 = line[..3]
            .parse()
            .with_context(|| format!("SMTP : code invalide dans '{line}'"))?;
        code = line_code;
        full.push_str(line.trim_end());

        // Le 4e caractère est ' ' → dernière ligne de la réponse
        if line.len() <= 4 || line.as_bytes()[3] == b' ' {
            break;
        }
        full.push('\n');
    }
    Ok((code, full))
}

// ─── Vérification du code ─────────────────────────────────────────────────────

fn expect(code: u16, expected: u16, context: &str) -> Result<()> {
    if code == expected {
        Ok(())
    } else {
        bail!("SMTP {context} : code {code} (attendu {expected})")
    }
}

fn expect_2xx(code: u16, context: &str) -> Result<()> {
    if (200..300).contains(&code) {
        Ok(())
    } else {
        bail!("SMTP {context} : code {code} (attendu 2xx)")
    }
}

// ─── Client SMTP ─────────────────────────────────────────────────────────────

/// Livre un message RFC 5322 (bytes bruts, avec en-têtes DKIM) au serveur SMTP.
///
/// # Paramètres
/// - `host`       : nom d'hôte MX (ex: "mail.google.com")
/// - `port`       : port SMTP (25 ou 587)
/// - `from`       : adresse enveloppe MAIL FROM (ex: "noreply@clairtyx.dev")
/// - `to`         : adresse enveloppe RCPT TO (ex: "jean@gmail.com")
/// - `raw_message`: message RFC 5322 complet (avec DKIM-Signature en tête)
/// - `our_domain` : notre domaine pour le EHLO (ex: "clairtyx.dev")
pub async fn deliver(
    host: &str,
    port: u16,
    from: &str,
    to: &str,
    raw_message: &[u8],
    our_domain: &str,
) -> Result<()> {
    tracing::debug!(host, port, to, "SMTP : connexion");

    // ── TCP connect ──────────────────────────────────────────────────────────
    let stream = timeout(
        CONNECT_TIMEOUT,
        TcpStream::connect(format!("{host}:{port}")),
    )
    .await
    .context("SMTP : timeout TCP connect")?
    .with_context(|| format!("SMTP : connexion à {host}:{port} refusée"))?;

    let mut transport = Transport::Plain(BufReader::new(stream));

    // ── Bannière 220 ─────────────────────────────────────────────────────────
    let (code, _banner) = transport.read_reply().await?;
    expect(code, 220, "banner")?;

    // ── EHLO ─────────────────────────────────────────────────────────────────
    let ehlo_resp = smtp_ehlo(&mut transport, our_domain).await?;
    let supports_starttls = ehlo_resp.to_uppercase().contains("STARTTLS");

    // ── STARTTLS ──────────────────────────────────────────────────────────────
    if supports_starttls {
        transport = start_tls(transport, host, our_domain).await?;
        tracing::debug!(host, "STARTTLS négocié");
    } else {
        tracing::warn!(host, "Serveur sans STARTTLS — livraison en clair");
    }

    // ── MAIL FROM ─────────────────────────────────────────────────────────────
    transport
        .write_all(format!("MAIL FROM:<{from}>\r\n").as_bytes())
        .await?;
    let (code, msg) = transport.read_reply().await?;
    expect_2xx(code, &format!("MAIL FROM — {msg}"))?;

    // ── RCPT TO ───────────────────────────────────────────────────────────────
    transport
        .write_all(format!("RCPT TO:<{to}>\r\n").as_bytes())
        .await?;
    let (code, msg) = transport.read_reply().await?;
    if code == 550 || code == 551 || code == 553 {
        bail!("SMTP RCPT TO : adresse refusée ({code}) — {msg}");
    }
    expect_2xx(code, &format!("RCPT TO — {msg}"))?;

    // ── DATA ──────────────────────────────────────────────────────────────────
    transport.write_all(b"DATA\r\n").await?;
    let (code, msg) = transport.read_reply().await?;
    expect(code, 354, &format!("DATA — {msg}"))?;

    // Transparence : doubler les lignes commençant par '.'
    let body = dot_stuff(raw_message);
    transport.write_all(&body).await?;
    transport.write_all(b"\r\n.\r\n").await?;

    let (code, msg) = transport.read_reply().await?;
    expect_2xx(code, &format!("fin DATA — {msg}"))?;

    // ── QUIT ──────────────────────────────────────────────────────────────────
    transport.write_all(b"QUIT\r\n").await?;
    let _ = transport.read_reply().await; // ignore l'erreur sur QUIT

    tracing::info!(host, to, "SMTP : message livré ✓");
    Ok(())
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

async fn smtp_ehlo(transport: &mut Transport, domain: &str) -> Result<String> {
    transport
        .write_all(format!("EHLO {domain}\r\n").as_bytes())
        .await?;
    let (code, resp) = transport.read_reply().await?;
    expect_2xx(code, &format!("EHLO — {resp}"))?;
    Ok(resp)
}

async fn start_tls(mut transport: Transport, host: &str, our_domain: &str) -> Result<Transport> {
    transport.write_all(b"STARTTLS\r\n").await?;
    let (code, msg) = transport.read_reply().await?;
    expect(code, 220, &format!("STARTTLS — {msg}"))?;

    // Upgrade du TcpStream en TLS
    let connector = TlsConnector::new().context("TLS : création du connecteur échouée")?;
    let tokio_connector = tokio_native_tls::TlsConnector::from(connector);

    let plain_reader = match transport {
        Transport::Plain(r) => r.into_inner(),
        _ => bail!("STARTTLS attendu sur transport plain"),
    };

    let tls_stream = tokio_connector
        .connect(host, plain_reader)
        .await
        .with_context(|| format!("STARTTLS : handshake TLS avec '{host}' échoué"))?;

    let mut upgraded = Transport::Tls(BufReader::new(tls_stream));

    // Re-EHLO après TLS
    smtp_ehlo(&mut upgraded, our_domain).await?;
    Ok(upgraded)
}

/// Transparence du point (RFC 5321 §4.5.2) :
/// chaque ligne commençant par '.' est préfixée d'un '.' supplémentaire.
fn dot_stuff(message: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(message.len() + 64);
    for line in message.split(|&b| b == b'\n') {
        if line.starts_with(b".") {
            out.push(b'.');
        }
        out.extend_from_slice(line);
        out.push(b'\n');
    }
    out
}
```

</details>

<details><summary><code>src/mta/queue_worker.rs</code> (446 lignes)</summary>

```rust
//! Worker de la file d'attente : lit les jobs `pending` dans `mail_queue`,
//! construit le message MIME, le signe en DKIM, résout les MX et livre via SMTP.

use std::{collections::HashMap, sync::Arc, time::Duration};

use crate::core::config::Config;
use anyhow::{Context, Result};
use futures::stream::{FuturesUnordered, StreamExt};
use rand::RngExt;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    message,
    metrics::{self, DELIVERIES, DELIVERY_DURATION},
    mta::{dkim, dns, smtp},
    template::{EmailEvent, TemplateEngine},
};

// ─── Types de statuts (miroir de l'enum PostgreSQL `mail_status`) ─────────────

#[derive(Debug, Clone, PartialEq, Eq, sqlx::Type)]
#[sqlx(type_name = "mail_status", rename_all = "lowercase")]
pub enum MailStatus {
    Pending,
    Sending,
    Sent,
    Failed,
    Bounced,
}

// ─── Struct de la ligne DB ────────────────────────────────────────────────────

#[derive(Debug, sqlx::FromRow)]
struct MailJob {
    id: Uuid,
    event_type: String,
    recipient: String,
    payload: Value, // JSONB → serde_json::Value
}

// ─── État partagé du worker ───────────────────────────────────────────────────

pub struct WorkerState {
    pub db: PgPool,
    pub config: Arc<Config>,
}

// ─── Boucle principale ────────────────────────────────────────────────────────

/// Démarre la boucle infinie du worker.
///
/// Comportement :
/// - Charge jusqu'à `worker_batch_size` jobs et les traite tous en parallèle
///   (concurrence limitée par `worker_concurrency` connexions SMTP simultanées).
/// - Si le batch était plein → il reste probablement des jobs → relance immédiatement.
/// - Si le batch était vide ou partiel → dort 5s avant le prochain cycle.
pub async fn run(state: WorkerState) -> ! {
    let batch_size = state.config.worker_batch_size;
    let concurrency = state.config.worker_concurrency;
    tracing::info!(batch_size, concurrency, "Queue worker démarré");

    let state = Arc::new(state);

    loop {
        if let Err(e) = metrics::refresh_queue_jobs(&state.db).await {
            tracing::warn!(error = %e, "Rafraîchissement de mta_queue_jobs échoué");
        }
        match tick(state.clone(), batch_size, concurrency).await {
            Ok(n) if n == batch_size as usize => {
                tracing::debug!(processed = n, "Batch plein, enchaînement immédiat");
            }
            Ok(n) if n > 0 => {
                tracing::info!(processed = n, "Batch traité");
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
            Ok(_) => {
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
            Err(e) => {
                tracing::error!(error = %e, "Erreur dans le queue worker");
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }
}

// ─── Une itération de la boucle ───────────────────────────────────────────────

async fn tick(state: Arc<WorkerState>, batch_size: u32, concurrency: usize) -> Result<usize> {
    // Templates relus à chaque cycle (une modification en base s'applique sans
    // redémarrage), avant de verrouiller : une erreur ici ne laisse aucun job en `sending`.
    let templates = Arc::new(TemplateEngine::load(&state.db).await?);

    // ── 1. Récupérer et verrouiller un batch de jobs ──────────────────────────
    let mut tx = state.db.begin().await?;

    let jobs: Vec<MailJob> = sqlx::query_as(
        r#"
        SELECT id, event_type, recipient, payload
        FROM   mail_queue
        WHERE  status = 'pending'
          AND  next_attempt_at <= NOW()
        ORDER  BY next_attempt_at
        LIMIT  $1
        FOR UPDATE SKIP LOCKED
        "#,
    )
    .bind(batch_size as i64)
    .fetch_all(&mut *tx)
    .await?;

    if jobs.is_empty() {
        tx.rollback().await?;
        return Ok(0);
    }

    let count = jobs.len();

    // Marquer tout le batch comme `sending` en une seule requête
    let ids: Vec<Uuid> = jobs.iter().map(|j| j.id).collect();
    sqlx::query(
        "UPDATE mail_queue
         SET    status = 'sending', attempts = attempts + 1
         WHERE  id = ANY($1)",
    )
    .bind(&ids)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    // ── 2. Traiter tous les jobs en parallèle, concurrence limitée ────────────
    let semaphore = Arc::new(tokio::sync::Semaphore::new(concurrency));

    let mut futures: FuturesUnordered<_> = jobs
        .into_iter()
        .map(|job| {
            let sem = semaphore.clone();
            let state = state.clone();
            let templates = templates.clone();
            tokio::spawn(async move {
                let _permit = sem.acquire_owned().await;
                let id = job.id;
                let event_type = job.event_type.clone();
                let timer = DELIVERY_DURATION.start_timer();
                let outcome = process_job(&state, &templates, job).await;
                timer.observe_duration();
                (id, event_type, outcome)
            })
        })
        .collect();

    while let Some(result) = futures.next().await {
        let (id, event_type, outcome) = result.map_err(|e| anyhow::anyhow!("tokio join: {e}"))?;
        match outcome {
            Ok(message_id) => {
                DELIVERIES
                    .with_label_values(&[event_type.as_str(), "sent"])
                    .inc();
                match delete_sent(&state.db, id).await {
                    Ok(()) => tracing::info!(%id, %message_id, "Email livré ✓"),
                    Err(e) => tracing::error!(%id, error = %e, "Suppression BDD échouée"),
                }
            }
            Err(e) => {
                tracing::warn!(%id, error = %e, "Échec de livraison");
                match reschedule_or_fail(&state.db, id).await {
                    Ok(Retry::Scheduled) => {
                        DELIVERIES
                            .with_label_values(&[event_type.as_str(), "retry"])
                            .inc();
                    }
                    Ok(Retry::GaveUp) => {
                        DELIVERIES
                            .with_label_values(&[event_type.as_str(), "failed"])
                            .inc();
                    }
                    Err(e) => tracing::error!(%id, error = %e, "Replanification échouée"),
                }
            }
        }
    }

    Ok(count)
}

// ─── Traitement d'un job ──────────────────────────────────────────────────────

async fn process_job(
    state: &WorkerState,
    templates: &TemplateEngine,
    job: MailJob,
) -> Result<String> {
    // 1. récupérer le recipient
    let recipient = &job.recipient;

    // 2. Identifier l'événement NATS et vérifier qu'il est supporté
    let event = EmailEvent::from_subject(&job.event_type)
        .ok_or_else(|| anyhow::anyhow!("type d'événement inconnu : '{}'", job.event_type))?;

    // 4. Rendu des templates HTML + texte
    let vars = payload_to_map(&job.payload)?;
    let rendered = templates.render(&event, &vars)?;

    // 5. Construire le nom d'affichage du destinataire depuis le payload
    let display_name = build_display_name(&vars);
    let sender_name = event.sender_name(&vars, &state.config.smtp_from_name);
    let reply_to = event.reply_to(&vars);

    // 6. Construire le message MIME
    let msg = message::build(
        &state.config.smtp_from_email,
        &sender_name,
        recipient,
        display_name.as_deref(),
        reply_to,
        &rendered.subject,
        &rendered.html,
        &rendered.text,
        &state.config.dkim_domain,
    )?;

    // 7. Signer avec DKIM
    let signed = dkim::sign(
        &msg.raw,
        &state.config.dkim_private_key_pem,
        &state.config.dkim_domain,
        &state.config.dkim_selector,
    )?;

    // 8. Déterminer la liste des hôtes SMTP cibles
    //    - Mode relay (SMTP_HOST défini) : livraison directe via le smarthost (ex: Mailpit en dev)
    //    - Mode MTA direct : lookup DNS MX puis essai dans l'ordre de priorité
    let smtp_targets: Vec<(String, u16)> = match &state.config.smtp_relay_host {
        Some(relay_host) => {
            tracing::debug!(
                relay = %relay_host,
                port = state.config.smtp_relay_port,
                "Mode relay SMTP — bypass DNS MX"
            );
            vec![(relay_host.clone(), state.config.smtp_relay_port)]
        }
        None => {
            let domain = dns::domain_of(recipient)?;
            let mx_records = dns::resolve_mx(domain).await?;
            if mx_records.is_empty() {
                anyhow::bail!("Aucun MX trouvé pour {domain}");
            }
            mx_records.into_iter().map(|mx| (mx.host, 25)).collect()
        }
    };

    // 9. Livrer via SMTP en essayant les cibles dans l'ordre
    let mut last_err = None;
    for (host, port) in &smtp_targets {
        match smtp::deliver(
            host,
            *port,
            &state.config.smtp_from_email,
            recipient,
            &signed,
            &state.config.dkim_domain,
        )
        .await
        {
            Ok(()) => return Ok(msg.message_id),
            Err(e) => {
                tracing::warn!(%host, %port, error = %e, "Hôte SMTP suivant essayé");
                last_err = Some(e);
            }
        }
    }

    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("tous les hôtes SMTP ont échoué")))
}

// ─── Replanification / abandon ────────────────────────────────────────────────

/// Suite donnée à un échec de livraison.
enum Retry {
    /// Nouvelle tentative planifiée (backoff exponentiel).
    Scheduled,
    /// Abandon : `max_attempts` atteint, job passé en `failed`.
    GaveUp,
}

async fn reschedule_or_fail(db: &PgPool, id: Uuid) -> Result<Retry> {
    // Récupère le nombre de tentatives actuelles
    let row: (i32, i32) =
        sqlx::query_as("SELECT attempts, max_attempts FROM mail_queue WHERE id = $1")
            .bind(id)
            .fetch_one(db)
            .await?;
    let (attempts, max_attempts) = row;

    if attempts >= max_attempts {
        sqlx::query(
            "UPDATE mail_queue SET status = 'failed', error = 'max_attempts atteints' WHERE id = $1",
        )
        .bind(id)
        .execute(db)
        .await?;
        tracing::warn!(%id, "Job marqué failed (max_attempts atteints)");
        Ok(Retry::GaveUp)
    } else {
        // Backoff exponentiel : 2^attempts * 60s + jitter(0..60s)
        let delay_secs = backoff_secs(attempts);
        sqlx::query(
            "UPDATE mail_queue
             SET status = 'pending',
                 next_attempt_at = NOW() + make_interval(secs => $2)
             WHERE id = $1",
        )
        .bind(id)
        .bind(delay_secs as f64)
        .execute(db)
        .await?;
        tracing::debug!(%id, delay_secs, "Job replanifié");
        Ok(Retry::Scheduled)
    }
}

/// Formule de backoff : `min(2^n * 60, 86400) + jitter(0..60)` secondes
fn backoff_secs(attempts: i32) -> u64 {
    let base: u64 = 2u64
        .saturating_pow(attempts as u32)
        .saturating_mul(60)
        .min(86400); // max 24h
    let jitter = rand::rng().random_range(0..60u64);
    base + jitter
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn payload_to_map(value: &Value) -> Result<HashMap<String, Value>> {
    match value {
        Value::Object(map) => Ok(map.iter().map(|(k, v)| (k.clone(), v.clone())).collect()),
        _ => anyhow::bail!("payload JSONB n'est pas un objet"),
    }
}

/// Construit les variables de template depuis le payload JSONB.
fn build_display_name(vars: &HashMap<String, Value>) -> Option<String> {
    let firstname = vars.get("firstname").and_then(|v| v.as_str());
    let name = vars.get("name").and_then(|v| v.as_str());
    match (firstname, name) {
        (Some(f), Some(n)) => Some(format!("{f} {n}")),
        (Some(f), None) => Some(f.to_string()),
        (None, Some(n)) => Some(n.to_string()),
        // Retard signalé (core.chantier.delay_reported) : le destinataire est
        // le client final, pas un compte Clairtyx — pas de champ "name".
        _ => vars
            .get("clientName")
            .and_then(|v| v.as_str())
            .map(str::to_string),
    }
}

// ─── Insertion dans la file ───────────────────────────────────────────────────

/// Insère un nouveau job dans `mail_queue`.
/// `recipient_enc` doit déjà être chiffré (produit par `crypto::encrypt`).
pub async fn enqueue(
    db: &PgPool,
    event_type: &str,
    recipient: &str,
    payload: &Value,
) -> Result<Uuid> {
    let id: (Uuid,) = sqlx::query_as(
        r#"
        INSERT INTO mail_queue (event_type, recipient, payload)
        VALUES ($1, $2, $3)
        RETURNING id
        "#,
    )
    .bind(event_type)
    .bind(recipient)
    .bind(payload)
    .fetch_one(db)
    .await?;

    Ok(id.0)
}

// ─── Corrélation bounce ───────────────────────────────────────────────────────

/// Met à jour le statut d'un job à `bounced` en corrélant via le `message_id`.
/// Insère également une ligne dans la table `bounces`.
pub async fn record_bounce(
    db: &PgPool,
    message_id: &str,
    bounce_type: &str,
    bounce_code: Option<i32>,
    raw_dsn: &str,
) -> Result<()> {
    // 1. Mettre à jour mail_queue
    sqlx::query("UPDATE mail_queue SET status = 'bounced' WHERE message_id = $1")
        .bind(message_id)
        .execute(db)
        .await?;

    // 2. Insérer dans bounces
    sqlx::query(
        r#"
        INSERT INTO bounces (message_id, bounce_type, bounce_code, raw_dsn)
        VALUES ($1, $2, $3, $4)
        "#,
    )
    .bind(message_id)
    .bind(bounce_type)
    .bind(bounce_code)
    .bind(raw_dsn)
    .execute(db)
    .await?;

    Ok(())
}

// ─── Correction statut sending → pending au démarrage ─────────────────────────
// ─── Suppression BDD après envoi ─────────────────────────────────────────────

/// Supprime la ligne de `mail_queue` après livraison réussie : la BDD ne
/// conserve que les jobs en attente, en erreur ou en cours de retry.
async fn delete_sent(db: &PgPool, id: Uuid) -> Result<()> {
    sqlx::query("DELETE FROM mail_queue WHERE id = $1")
        .bind(id)
        .execute(db)
        .await
        .context("DELETE mail_queue après envoi")?;
    Ok(())
}

// ─── Correction statut sending → pending au démarrage ─────────────────────────
/// Remet les jobs bloqués en `sending` à `pending` au redémarrage du service.
/// (Prévu pour les crashs en cours de traitement)
pub async fn reset_stale_sending(db: &PgPool) -> Result<u64> {
    let res = sqlx::query(
        "UPDATE mail_queue SET status = 'pending', next_attempt_at = NOW()
         WHERE status = 'sending'",
    )
    .execute(db)
    .await?;
    Ok(res.rows_affected())
}
```

</details>

<details><summary><code>src/bounce/mod.rs</code> (1 lignes)</summary>

```rust
pub mod server;
```

</details>

<details><summary><code>src/bounce/server.rs</code> (282 lignes)</summary>

```rust
//! Serveur SMTP entrant minimal pour la réception des bounces (NDR/DSN).
//!
//! RFC 5321 : séquence EHLO → MAIL FROM → RCPT TO → DATA
//! RFC 3464 : format DSN (Delivery Status Notification)
//!
//! Ce serveur accepte TOUTES les connexions et TOUS les destinataires
//! (il ne fait que recevoir des NDR envoyés à nos adresses bounce).
//! La corrélation avec le message original se fait via le header `Message-ID`
//! présent dans la partie "message/rfc822" ou dans les en-têtes DSN.

use std::{sync::Arc, time::Duration};

use anyhow::Result;
use sqlx::PgPool;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    time::timeout,
};

use crate::metrics::BOUNCES;
use crate::mta::queue_worker;

/// Timeout de lecture par commande SMTP
const SMTP_TIMEOUT: Duration = Duration::from_secs(60);
/// Taille maximale d'un message DATA accepté (10 Mo)
const MAX_MESSAGE_SIZE: usize = 10 * 1024 * 1024;

// ─── Point d'entrée ───────────────────────────────────────────────────────────

pub async fn run(addr: &str, hostname: Arc<String>, db: PgPool) -> Result<()> {
    let listener = TcpListener::bind(addr)
        .await
        .map_err(|e| anyhow::anyhow!("Bounce SMTP : bind sur {addr} échoué — {e}"))?;

    tracing::info!(addr, "Serveur SMTP bounce en écoute");

    loop {
        match listener.accept().await {
            Ok((stream, peer)) => {
                let db2 = db.clone();
                let hostname2 = hostname.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_connection(stream, hostname2, db2).await {
                        tracing::warn!(peer = %peer, error = %e, "Session bounce terminée avec erreur");
                    }
                });
            }
            Err(e) => tracing::error!(error = %e, "Bounce SMTP : accept() échoué"),
        }
    }
}

// ─── Gestion d'une connexion SMTP ─────────────────────────────────────────────

async fn handle_connection(stream: TcpStream, hostname: Arc<String>, db: PgPool) -> Result<()> {
    let peer = stream
        .peer_addr()
        .ok()
        .map(|a| a.to_string())
        .unwrap_or_default();
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);

    // ── Bannière 220 ─────────────────────────────────────────────────────────
    write_line(
        &mut writer,
        &format!("220 {} ESMTP Clairtyx Bounce Receiver\r\n", hostname),
    )
    .await?;

    let mut data_mode = false;
    let mut data_buf = Vec::<u8>::new();

    loop {
        let mut line = String::new();
        let n = timeout(SMTP_TIMEOUT, reader.read_line(&mut line))
            .await
            .map_err(|_| anyhow::anyhow!("timeout lecture"))?
            .map_err(|e| anyhow::anyhow!("lecture: {e}"))?;

        if n == 0 {
            break; // connexion fermée
        }

        if data_mode {
            // Mode DATA : accumulation jusqu'à "\r\n.\r\n"
            if data_buf.len() + line.len() > MAX_MESSAGE_SIZE {
                anyhow::bail!("message trop volumineux");
            }
            if line == ".\r\n" || line == ".\n" {
                // Fin du message
                data_mode = false;
                write_line(&mut writer, "250 Ok: message reçu\r\n").await?;

                // Traitement asynchrone pour ne pas bloquer la session SMTP
                let raw_dsn = data_buf.clone();
                let db2 = db.clone();
                let peer2 = peer.clone();
                tokio::spawn(async move {
                    if let Err(e) = process_dsn(&raw_dsn, &db2).await {
                        tracing::warn!(peer = %peer2, error = %e, "Erreur traitement DSN");
                    }
                });
                data_buf.clear();
            } else {
                // Annuler la transparence du point (RFC 5321 §4.5.2)
                let content = if line.starts_with("..") {
                    &line[1..]
                } else {
                    &line
                };
                data_buf.extend_from_slice(content.as_bytes());
            }
            continue;
        }

        let upper = line.trim().to_uppercase();

        // ── Commandes SMTP ────────────────────────────────────────────────────
        if upper.starts_with("EHLO") || upper.starts_with("HELO") {
            let domain = line
                .trim()
                .split_once(' ')
                .map(|(_, rest)| rest)
                .unwrap_or("unknown");
            tracing::debug!(peer = %peer, domain = domain.trim(), "EHLO reçu");
            write_line(
                &mut writer,
                &format!(
                    "250-{}\r\n250-PIPELINING\r\n250-MESSAGE-SIZE {}\r\n250 OK\r\n",
                    hostname, MAX_MESSAGE_SIZE
                ),
            )
            .await?;
        } else if upper.starts_with("MAIL FROM") || upper.starts_with("RCPT TO") {
            write_line(&mut writer, "250 Ok\r\n").await?;
        } else if upper.starts_with("DATA") {
            write_line(&mut writer, "354 Envoyez le message, terminez par '.'\r\n").await?;
            data_mode = true;
        } else if upper.starts_with("RSET") {
            data_buf.clear();
            data_mode = false;
            write_line(&mut writer, "250 Ok\r\n").await?;
        } else if upper.starts_with("NOOP") {
            write_line(&mut writer, "250 Ok\r\n").await?;
        } else if upper.starts_with("QUIT") {
            write_line(&mut writer, &format!("221 {} Bye\r\n", hostname)).await?;
            break;
        } else {
            write_line(&mut writer, "502 Commande non implémentée\r\n").await?;
        }
    }

    Ok(())
}

// ─── Parsing et enregistrement du DSN ────────────────────────────────────────

async fn process_dsn(raw: &[u8], db: &PgPool) -> Result<()> {
    let raw_str = String::from_utf8_lossy(raw);

    // Extraire le Message-ID original depuis les en-têtes DSN
    // Cherche "In-Reply-To:", "X-Original-Message-ID:", ou l'en-tête du message encapsulé
    let message_id = extract_original_message_id(&raw_str);

    let (bounce_type, bounce_code) = detect_bounce_type(&raw_str);

    let id_str = match &message_id {
        Some(id) => id.as_str(),
        None => {
            tracing::warn!("DSN reçu sans Message-ID original identifiable");
            // On enregistre quand même sans corrélation
            return Ok(());
        }
    };

    tracing::info!(
        message_id = id_str,
        bounce_type,
        ?bounce_code,
        "Bounce reçu"
    );

    queue_worker::record_bounce(db, id_str, bounce_type, bounce_code, &raw_str).await?;
    // `bounce_type` vient de `detect_bounce_type` : valeurs fixes, cardinalité bornée.
    BOUNCES.with_label_values(&[bounce_type]).inc();
    Ok(())
}

/// Recherche l'identifiant du message original dans un DSN.
/// Essaie plusieurs champs dans l'ordre de fiabilité.
fn extract_original_message_id(dsn: &str) -> Option<String> {
    // 1. Header "In-Reply-To:" (le plus courant dans les NDR)
    if let Some(id) = find_header(dsn, "In-Reply-To") {
        return Some(id);
    }
    // 2. Partie "message/delivery-status" : "Original-Message-ID:"
    if let Some(id) = find_header(dsn, "Original-Message-ID") {
        return Some(id);
    }
    // 3. Final-Recipient dans la partie delivery-status
    // (pas un Message-ID, mais utile en dernier recours)
    None
}

/// Détecte le type de bounce et le code SMTP depuis le corps du DSN.
fn detect_bounce_type(dsn: &str) -> (&'static str, Option<i32>) {
    // Recherche un "Status:" dans la partie delivery-status
    // Format : "Status: N.N.N" où le premier chiffre donne la sévérité
    for line in dsn.lines() {
        let l = line.trim();
        if l.to_lowercase().starts_with("status:") {
            let status = l.split_once(':').map(|(_, v)| v).unwrap_or("").trim();
            if status.starts_with('5') {
                return ("permanent", parse_smtp_code(dsn));
            } else if status.starts_with('4') {
                return ("transient", parse_smtp_code(dsn));
            }
        }
        // Recherche directe de codes SMTP dans "Diagnostic-Code:"
        if l.to_lowercase().starts_with("diagnostic-code:") {
            let diagnostic = l.to_lowercase();
            if diagnostic.contains("550")
                || diagnostic.contains("551")
                || diagnostic.contains("553")
                || diagnostic.contains("554")
            {
                return ("permanent", extract_code_from_diagnostic(l));
            }
        }
    }
    ("transient", None)
}

fn parse_smtp_code(dsn: &str) -> Option<i32> {
    for line in dsn.lines() {
        let l = line.trim().to_lowercase();
        if l.starts_with("diagnostic-code:") {
            return extract_code_from_diagnostic(line);
        }
    }
    None
}

fn extract_code_from_diagnostic(line: &str) -> Option<i32> {
    // Extrait le premier nombre à 3 chiffres commençant par 4 ou 5
    for word in line.split_whitespace() {
        if let Ok(n) = word
            .trim_matches(|c: char| !c.is_ascii_digit())
            .parse::<i32>()
        {
            if (400..600).contains(&n) {
                return Some(n);
            }
        }
    }
    None
}

fn find_header(text: &str, name: &str) -> Option<String> {
    let prefix = format!("{}:", name.to_lowercase());
    for line in text.lines() {
        if line.to_lowercase().starts_with(&prefix) {
            let value = line[prefix.len()..].trim().to_string();
            if !value.is_empty() {
                return Some(value);
            }
        }
    }
    None
}

// ─── Écriture SMTP ────────────────────────────────────────────────────────────

async fn write_line(writer: &mut tokio::net::tcp::OwnedWriteHalf, line: &str) -> Result<()> {
    timeout(SMTP_TIMEOUT, writer.write_all(line.as_bytes()))
        .await
        .map_err(|_| anyhow::anyhow!("timeout écriture SMTP"))?
        .map_err(|e| anyhow::anyhow!("écriture SMTP: {e}"))?;
    Ok(())
}
```

</details>

<details><summary><code>migrations/20260302000001_mail_queue.sql</code> (34 lignes)</summary>

```sql
-- Table de file d'attente pour tous les emails sortants
-- status:
--   pending  → en attente de traitement
--   sending  → en cours (verrou SKIP LOCKED)
--   sent     → livré avec succès
--   failed   → échecs max atteints
--   bounced  → NDR/DSN reçu du destinataire

CREATE TYPE mail_status AS ENUM ('pending', 'sending', 'sent', 'failed', 'bounced');

CREATE TABLE mail_queue (
    id              UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    event_type      TEXT        NOT NULL,
    recipient       TEXT        NOT NULL,
    payload         JSONB       NOT NULL,
    status          mail_status NOT NULL DEFAULT 'pending',
    attempts        INT         NOT NULL DEFAULT 0,
    max_attempts    INT         NOT NULL DEFAULT 5,
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    message_id      TEXT,                                           -- Message-ID RFC 5322, positionné après construction MIME
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    sent_at         TIMESTAMPTZ,
    error           TEXT                                            -- dernière erreur (SMTP ou DNS)
);

-- Index partiel : seuls les jobs pending/prochain essai sont scannés fréquemment
CREATE INDEX idx_mail_queue_pending
    ON mail_queue (next_attempt_at)
    WHERE status = 'pending';

-- Index sur message_id pour corrélation avec les bounces
CREATE INDEX idx_mail_queue_message_id
    ON mail_queue (message_id)
    WHERE message_id IS NOT NULL;
```

</details>

<details><summary><code>migrations/20260302000002_bounces.sql</code> (15 lignes)</summary>

```sql
-- Table des bounces (NDR/DSN reçus sur le serveur SMTP entrant)
-- bounce_type:
--   permanent  → 5xx (adresse inexistante, boîte pleine définitive…)
--   transient  → 4xx (temporaire, reçu après nos tentatives maximales)

CREATE TABLE bounces (
    id          UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    message_id  TEXT        NOT NULL,                   -- corrélé avec mail_queue.message_id
    bounce_type TEXT        NOT NULL CHECK (bounce_type IN ('permanent', 'transient')),
    bounce_code INT,                                    -- code SMTP (ex: 550, 421)
    raw_dsn     TEXT,                                   -- corps brut du DSN pour diagnostic
    received_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_bounces_message_id ON bounces (message_id);
```

</details>

<details><summary><code>migrations/20260614000001_dedup_key.sql</code> (9 lignes)</summary>

```sql
-- Idempotence du consumer Kafka (at-least-once) : un event rejoué ne doit pas
-- générer un mail en double. `dedup_key` porte une clé naturelle par event
-- (user_id pour la bienvenue, event_id pour les mails sécurité) ; l'INSERT du
-- consumer fait `ON CONFLICT (dedup_key) DO NOTHING`.
ALTER TABLE mail_queue ADD COLUMN dedup_key TEXT;

CREATE UNIQUE INDEX uq_mail_queue_dedup_key
    ON mail_queue (dedup_key)
    WHERE dedup_key IS NOT NULL;
```

</details>


### 6.2 Partie propre au projet (remplacée par les données du §1)

`consumer.rs` est remplacé par les règles et la colle (§1.2, §2). Dans `template/mod.rs`, seul l'enum
`EmailEvent` est propre au projet (le moteur `TemplateEngine` est fixe). Les deux migrations de templates
deviennent la migration générée du §1.3.

<details><summary><code>src/core/consumer.rs</code> (262 lignes)</summary>

```rust
use anyhow::Result;
use async_trait::async_trait;
use shared::kafka::{ConsumerConfig, EventConsumer, InboundEventHandler};
use shared::topics::{
    AUTH_EMAIL_CHANGE_REQUESTED, AUTH_EMAIL_VERIFICATION_REQUESTED, AUTH_INVITATION_CREATED,
    AUTH_PASSWORD_RESET_REQUESTED, AUTH_USER_REGISTERED,
};
use sqlx::PgPool;
use std::sync::Arc;

use crate::metrics::KAFKA_MESSAGES;

// Produit par le monolithe (bouton "Signaler un retard"). SMS pris en charge
// plus tard par un service dédié, sur ce même topic.
const CORE_CHANTIER_DELAY_REPORTED: &str = "core.chantier.delay_reported";

pub const TOPICS: &[&str] = &[
    AUTH_USER_REGISTERED,
    AUTH_EMAIL_CHANGE_REQUESTED,
    AUTH_PASSWORD_RESET_REQUESTED,
    AUTH_EMAIL_VERIFICATION_REQUESTED,
    AUTH_INVITATION_CREATED,
    CORE_CHANTIER_DELAY_REPORTED,
];

/// Lance le consumer en tâche de fond. La lecture du flux, le commit des offsets et la reprise sur
/// place sont ceux de `shared::kafka::EventConsumer` ; ici ne reste que la mise en file.
pub fn spawn(config: &ConsumerConfig, db: PgPool) -> Result<tokio::task::JoinHandle<()>> {
    let handlers: Vec<Arc<dyn InboundEventHandler>> = vec![Arc::new(MailQueueHandler { db })];
    let consumer = EventConsumer::new(config, TOPICS, handlers)?;
    tracing::info!(topics = ?TOPICS, group = %config.group_id, "consumer kafka démarré");
    Ok(tokio::spawn(async move { consumer.run().await }))
}

/// Met en file les mails demandés par les événements.
struct MailQueueHandler {
    db: PgPool,
}

#[async_trait]
impl InboundEventHandler for MailQueueHandler {
    /// Un payload « poison » (illisible, champ obligatoire absent) est logué puis ignoré : rejoué
    /// indéfiniment, il bloquerait la partition. Une panne de la base, elle, est retentée sur
    /// place, sinon le mail serait perdu.
    async fn apply_or_dead_letter(&self, topic: &str, payload: &[u8]) -> bool {
        let Ok(payload) = std::str::from_utf8(payload) else {
            tracing::warn!(topic, "message sans payload UTF-8 — ignoré");
            count_message(topic, "invalid");
            return true;
        };
        let value: serde_json::Value = match serde_json::from_str(payload) {
            Ok(v) => v,
            Err(e) => {
                tracing::error!(topic, error = %e, "payload JSON illisible — ignoré");
                count_message(topic, "invalid");
                return true;
            }
        };

        let db = &self.db;
        let result = match topic {
            AUTH_USER_REGISTERED => enqueue_welcome(db, &value).await,
            AUTH_EMAIL_CHANGE_REQUESTED => enqueue_email_change(db, &value).await,
            AUTH_PASSWORD_RESET_REQUESTED => enqueue_password_reset(db, &value).await,
            AUTH_EMAIL_VERIFICATION_REQUESTED => enqueue_email_verification(db, &value).await,
            AUTH_INVITATION_CREATED => enqueue_invitation(db, &value).await,
            CORE_CHANTIER_DELAY_REPORTED => enqueue_delay_reported(db, &value).await,
            other => {
                tracing::warn!(topic = other, "topic non géré — ignoré");
                count_message(other, "invalid");
                return true;
            }
        };

        match result {
            Ok(Enqueued::New) => count_message(topic, "enqueued"),
            Ok(Enqueued::Duplicate) => count_message(topic, "duplicate"),
            Ok(Enqueued::Skipped) => count_message(topic, "skipped"),
            Err(e) => {
                tracing::error!(topic, error = %e, "échec mise en file");
                count_message(topic, "error");
                // Base indisponible : à retenter. Toute autre erreur est celle du payload.
                return e.downcast_ref::<sqlx::Error>().is_none();
            }
        }
        true
    }
}

/// Topics hors de `TOPICS` regroupés sous `other` : cardinalité bornée.
fn count_message(topic: &str, outcome: &str) {
    let topic = if TOPICS.contains(&topic) {
        topic
    } else {
        "other"
    };
    KAFKA_MESSAGES.with_label_values(&[topic, outcome]).inc();
}

/// Issue d'une mise en file : nouveau job, event rejoué déjà en file, ou rien
/// à envoyer (pas d'adresse e-mail — voir `enqueue_delay_reported`).
enum Enqueued {
    New,
    Duplicate,
    Skipped,
}

fn recipient_of<'a>(payload: &'a serde_json::Value, event_type: &str) -> Result<&'a str> {
    payload["email"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("champ 'email' absent du payload {event_type}"))
}

/// Insère un job de mail. `dedup_key` est unique : un event rejoué (at-least-once)
/// est un no-op grâce à `ON CONFLICT DO NOTHING` → pas de mail en double.
async fn enqueue(
    db: &PgPool,
    event_type: &str,
    recipient: &str,
    payload: &serde_json::Value,
    dedup_key: &str,
) -> Result<Enqueued> {
    let inserted = sqlx::query_scalar::<_, uuid::Uuid>(
        // L'index unique sur dedup_key est PARTIEL (WHERE dedup_key IS NOT NULL) :
        // l'inférence ON CONFLICT doit répéter ce prédicat pour le matcher.
        r#"INSERT INTO mail_queue (event_type, recipient, payload, dedup_key)
           VALUES ($1, $2, $3, $4)
           ON CONFLICT (dedup_key) WHERE dedup_key IS NOT NULL DO NOTHING
           RETURNING id"#,
    )
    .bind(event_type)
    .bind(recipient)
    .bind(payload)
    .bind(dedup_key)
    .fetch_optional(db)
    .await?;

    match inserted {
        Some(_) => {
            tracing::info!(recipient, event_type, "email mis en file");
            Ok(Enqueued::New)
        }
        None => {
            tracing::debug!(recipient, event_type, "event rejoué — déjà en file, no-op");
            Ok(Enqueued::Duplicate)
        }
    }
}

async fn enqueue_welcome(db: &PgPool, payload: &serde_json::Value) -> Result<Enqueued> {
    let recipient = recipient_of(payload, AUTH_USER_REGISTERED)?;
    // Un seul mail de bienvenue par utilisateur.
    let user_id = payload["user_id"].as_str().unwrap_or(recipient);
    let dedup_key = format!("{AUTH_USER_REGISTERED}:{user_id}");
    enqueue(db, AUTH_USER_REGISTERED, recipient, payload, &dedup_key).await
}

/// Lien de confirmation d'un changement d'adresse e-mail (svc-auth).
/// `event_id` est un identifiant de demande frais à chaque fois (généré par
/// svc-auth) : contrairement au mail de bienvenue, plusieurs demandes légitimes
/// peuvent se succéder pour un même compte, chacune doit partir.
async fn enqueue_email_change(db: &PgPool, payload: &serde_json::Value) -> Result<Enqueued> {
    let recipient = recipient_of(payload, AUTH_EMAIL_CHANGE_REQUESTED)?;
    // `eventId` (camelCase) : svc-auth sérialise ses structs d'événements avec
    // `#[serde(rename_all = "camelCase")]`, `event_id` n'existe donc jamais
    // tel quel dans le payload JSON réel.
    let event_id = payload["eventId"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| format!("{AUTH_EMAIL_CHANGE_REQUESTED}:{recipient}"));
    let dedup_key = format!("{AUTH_EMAIL_CHANGE_REQUESTED}:{event_id}");
    enqueue(
        db,
        AUTH_EMAIL_CHANGE_REQUESTED,
        recipient,
        payload,
        &dedup_key,
    )
    .await
}

/// Lien de réinitialisation de mot de passe (svc-auth, flux Fortify "réinitialisation
/// du mot de passe"). Comme le changement d'e-mail, plusieurs demandes légitimes
/// peuvent se succéder pour un même compte : la dédup porte sur `event_id`,
/// généré à chaque demande par svc-auth, jamais sur le compte seul.
/// Lien de vérification d'adresse e-mail à l'inscription (svc-auth, chantier C).
/// Même patron de dédup que `enqueue_email_change` ci-dessus.
async fn enqueue_email_verification(db: &PgPool, payload: &serde_json::Value) -> Result<Enqueued> {
    let recipient = recipient_of(payload, AUTH_EMAIL_VERIFICATION_REQUESTED)?;
    let event_id = payload["eventId"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| format!("{AUTH_EMAIL_VERIFICATION_REQUESTED}:{recipient}"));
    let dedup_key = format!("{AUTH_EMAIL_VERIFICATION_REQUESTED}:{event_id}");
    enqueue(
        db,
        AUTH_EMAIL_VERIFICATION_REQUESTED,
        recipient,
        payload,
        &dedup_key,
    )
    .await
}

async fn enqueue_password_reset(db: &PgPool, payload: &serde_json::Value) -> Result<Enqueued> {
    let recipient = recipient_of(payload, AUTH_PASSWORD_RESET_REQUESTED)?;
    // `eventId` (camelCase) : voir le commentaire équivalent dans
    // `enqueue_email_change` ci-dessus.
    let event_id = payload["eventId"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| format!("{AUTH_PASSWORD_RESET_REQUESTED}:{recipient}"));
    let dedup_key = format!("{AUTH_PASSWORD_RESET_REQUESTED}:{event_id}");
    enqueue(
        db,
        AUTH_PASSWORD_RESET_REQUESTED,
        recipient,
        payload,
        &dedup_key,
    )
    .await
}

/// Invitation d'un collaborateur (lien magique). `eventId` est l'identifiant
/// de l'invitation : une nouvelle invitation (réinvitation) repart, un event
/// rejoué non.
async fn enqueue_invitation(db: &PgPool, payload: &serde_json::Value) -> Result<Enqueued> {
    let recipient = recipient_of(payload, AUTH_INVITATION_CREATED)?;
    let event_id = payload["eventId"].as_str().ok_or_else(|| {
        anyhow::anyhow!("champ 'eventId' absent du payload {AUTH_INVITATION_CREATED}")
    })?;
    let dedup_key = format!("{AUTH_INVITATION_CREATED}:{event_id}");
    enqueue(db, AUTH_INVITATION_CREATED, recipient, payload, &dedup_key).await
}

/// Retard signalé sur un chantier (core, bouton "Signaler un retard").
/// `eventId` est l'identifiant du retard : un event rejoué ne renvoie pas un
/// second mail. Le client n'a pas forcément d'e-mail (client renseigné par
/// téléphone seulement) : sans adresse, rien à mettre en file — le SMS (s'il
/// est activé) prendra le relais côté service sms, sur ce même topic.
async fn enqueue_delay_reported(db: &PgPool, payload: &serde_json::Value) -> Result<Enqueued> {
    let recipient = payload["clientEmail"].as_str().unwrap_or("");
    if recipient.is_empty() {
        tracing::debug!(
            topic = CORE_CHANTIER_DELAY_REPORTED,
            "client sans e-mail — rien à mettre en file"
        );
        return Ok(Enqueued::Skipped);
    }
    let event_id = payload["eventId"].as_str().ok_or_else(|| {
        anyhow::anyhow!("champ 'eventId' absent du payload {CORE_CHANTIER_DELAY_REPORTED}")
    })?;
    let dedup_key = format!("{CORE_CHANTIER_DELAY_REPORTED}:{event_id}");
    enqueue(
        db,
        CORE_CHANTIER_DELAY_REPORTED,
        recipient,
        payload,
        &dedup_key,
    )
    .await
}
```

</details>

<details><summary><code>src/template/mod.rs</code> (433 lignes)</summary>

```rust
use anyhow::{Context, Result};
use serde_json::Value;
use sqlx::PgPool;
use std::collections::HashMap;
use tera::{Context as TeraCtx, Tera};

/// Types d'événements Kafka gérés par ce service
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EmailEvent {
    Welcome,
    EmailChangeRequested,
    PasswordReset,
    EmailVerification,
    Invitation,
    DelayReported,
}

impl EmailEvent {
    /// Convertit le nom du topic Kafka (= `event_type` stocké dans mail_queue)
    /// en variante EmailEvent.
    pub fn from_subject(s: &str) -> Option<Self> {
        match s {
            "auth.user.registered" => Some(Self::Welcome),
            "auth.email_change.requested" => Some(Self::EmailChangeRequested),
            "auth.password_reset.requested" => Some(Self::PasswordReset),
            "auth.email_verification.requested" => Some(Self::EmailVerification),
            "auth.invitation.created" => Some(Self::Invitation),
            "core.chantier.delay_reported" => Some(Self::DelayReported),
            _ => None,
        }
    }

    /// Clé du template en base (`mail_templates.event_type`) : le sujet du topic Kafka.
    pub fn event_type(&self) -> &'static str {
        match self {
            Self::Welcome => "auth.user.registered",
            Self::EmailChangeRequested => "auth.email_change.requested",
            Self::PasswordReset => "auth.password_reset.requested",
            Self::EmailVerification => "auth.email_verification.requested",
            Self::Invitation => "auth.invitation.created",
            Self::DelayReported => "core.chantier.delay_reported",
        }
    }

    /// Nom d'expéditeur affiché (en-tête `From`). Par défaut celui de la
    /// configuration (`SMTP_FROM_NAME`, "Clairtyx") : les mails à nos propres
    /// clients (compte, invitation...) partent bien en leur nom. Un retard
    /// signalé s'adresse au client final de l'entreprise, jamais informé de
    /// Clairtyx : l'e-mail doit se présenter comme venant d'elle, pas de nous
    /// (voir `services::delay_events` côté monolithe, qui fournit `companyName`).
    pub fn sender_name(&self, vars: &HashMap<String, Value>, default_name: &str) -> String {
        match self {
            Self::DelayReported => match vars.get("companyName").and_then(|v| v.as_str()) {
                Some(company) if !company.is_empty() => format!("{company} via Clairtyx"),
                _ => default_name.to_string(),
            },
            _ => default_name.to_string(),
        }
    }

    /// Adresse `Reply-To`, si le retard doit y répondre ailleurs qu'à
    /// l'expéditeur technique (`contactEmail`, professionnelle, fournie par
    /// `services::companies` côté monolithe). `None` : pas d'en-tête `Reply-To`,
    /// les réponses vont à l'adresse d'expédition par défaut.
    pub fn reply_to<'v>(&self, vars: &'v HashMap<String, Value>) -> Option<&'v str> {
        match self {
            Self::DelayReported => vars
                .get("contactEmail")
                .and_then(|v| v.as_str())
                .filter(|email| !email.is_empty()),
            _ => None,
        }
    }
}

/// Rendu d'un email : corps HTML + corps texte brut
pub struct RenderedEmail {
    pub subject: String,
    pub html: String,
    pub text: String,
}

/// Moteur de templates Tera, construit depuis la table `mail_templates`.
pub struct TemplateEngine {
    tera: Tera,
}

#[derive(sqlx::FromRow)]
struct TemplateRow {
    event_type: String,
    subject: String,
    html_body: String,
    text_body: String,
}

impl TemplateEngine {
    /// Charge tous les templates depuis la base. Le worker l'appelle à chaque
    /// cycle : une modification en base s'applique sans redémarrer le service.
    pub async fn load(db: &PgPool) -> Result<Self> {
        let rows: Vec<TemplateRow> =
            sqlx::query_as("SELECT event_type, subject, html_body, text_body FROM mail_templates")
                .fetch_all(db)
                .await
                .context("lecture de mail_templates")?;

        // `.html` est auto-échappé par Tera ; le sujet et le texte brut ne le sont pas.
        let raw = rows.iter().flat_map(|row| {
            [
                (format!("{}.subject", row.event_type), row.subject.as_str()),
                (format!("{}.html", row.event_type), row.html_body.as_str()),
                (format!("{}.txt", row.event_type), row.text_body.as_str()),
            ]
        });
        let mut tera = Tera::default();
        tera.add_raw_templates(raw)
            .context("Tera : compilation des templates de mail_templates")?;
        Ok(Self { tera })
    }

    /// Rend le sujet et les corps HTML / texte d'un email à partir des variables du payload.
    pub fn render(
        &self,
        event: &EmailEvent,
        vars: &HashMap<String, Value>,
    ) -> Result<RenderedEmail> {
        let mut ctx = TeraCtx::new();
        for (k, v) in vars {
            ctx.insert(k, v);
        }

        let key = event.event_type();
        let render = |ext: &str| {
            self.tera
                .render(&format!("{key}.{ext}"), &ctx)
                .with_context(|| format!("Tera : rendu de '{key}.{ext}'"))
        };
        Ok(RenderedEmail {
            subject: render("subject")?,
            html: render("html")?,
            text: render("txt")?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Le lien de réinitialisation et son expiration apparaissent bien dans les
    /// deux corps, et le mail est correctement branché Clairtyx (pas de résidu
    /// "Workonva", ancien nom de ce service).
    ///
    /// Clés en camelCase : `payload_to_map` (queue_worker.rs) injecte le JSON
    /// du payload tel quel dans le contexte Tera, sans renommage — elles
    /// doivent donc reprendre exactement la forme `#[serde(rename_all =
    /// "camelCase")]` des events publiés par `svc-auth` (events.rs), pas les noms
    /// snake_case des champs Rust.
    #[sqlx::test(migrations = "./migrations")]
    async fn password_reset_renders_as_clairtyx_with_the_reset_link(db: PgPool) {
        let engine = TemplateEngine::load(&db)
            .await
            .expect("chargement des templates");
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), serde_json::json!("Camille"));
        vars.insert(
            "resetUrl".to_string(),
            serde_json::json!("https://app.clairtyx.com/reinitialiser?token=abc123"),
        );
        vars.insert(
            "expiresAt".to_string(),
            serde_json::json!("2026-09-23T18:00:00Z"),
        );
        let rendered = engine
            .render(&EmailEvent::PasswordReset, &vars)
            .expect("rendu réinitialisation de mot de passe");

        for body in [&rendered.html, &rendered.text] {
            assert!(body.contains("Clairtyx"), "le corps doit citer Clairtyx");
            assert!(!body.contains("Workonva"), "aucun résidu de l'ancien nom");
            assert!(body.contains("https://app.clairtyx.com/reinitialiser?token=abc123"));
            assert!(body.contains("2026-09-23T18:00:00Z"));
        }
        assert!(rendered.subject.contains("Clairtyx"));
    }

    /// Le mail de bienvenue est bien celui de Clairtyx (pas de résidu "Workonva",
    /// ancien nom de ce service) et reprend le nom du
    /// compte transmis par `svc-auth` (jamais l'entreprise : elle n'existe pas encore à l'inscription).
    #[sqlx::test(migrations = "./migrations")]
    async fn welcome_renders_as_clairtyx_and_is_personalized(db: PgPool) {
        let engine = TemplateEngine::load(&db)
            .await
            .expect("chargement des templates");
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), serde_json::json!("Camille"));
        vars.insert(
            "companyName".to_string(),
            serde_json::json!("Dupont Plomberie"),
        );
        let rendered = engine
            .render(&EmailEvent::Welcome, &vars)
            .expect("rendu mail de bienvenue");

        for body in [&rendered.html, &rendered.text] {
            assert!(body.contains("Clairtyx"), "le corps doit citer Clairtyx");
            assert!(!body.contains("Workonva"), "aucun résidu de l'ancien nom");
            assert!(body.contains("Camille"), "le nom du compte doit apparaître");
            assert!(
                !body.contains("Dupont Plomberie"),
                "l'entreprise n'est pas connue à l'inscription : le mail de bienvenue ne la cite pas"
            );
        }
        assert!(rendered.subject.contains("Clairtyx"));
    }

    /// Le lien de confirmation et son expiration apparaissent bien dans les
    /// deux corps, et le mail est correctement branché Clairtyx.
    ///
    /// Clés en camelCase, comme pour `password_reset` ci-dessus — c'est la
    /// forme réelle publiée par `EmailChangeRequested` (events.rs, svc-auth).
    #[sqlx::test(migrations = "./migrations")]
    async fn email_change_requested_renders_the_confirmation_link(db: PgPool) {
        let engine = TemplateEngine::load(&db)
            .await
            .expect("chargement des templates");
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), serde_json::json!("Camille"));
        vars.insert(
            "confirmUrl".to_string(),
            serde_json::json!("https://app.clairtyx.com/confirmer-email?token=abc123"),
        );
        vars.insert(
            "expiresAt".to_string(),
            serde_json::json!("2026-09-23T18:00:00Z"),
        );
        let rendered = engine
            .render(&EmailEvent::EmailChangeRequested, &vars)
            .expect("rendu confirmation de changement d'e-mail");

        for body in [&rendered.html, &rendered.text] {
            assert!(body.contains("Clairtyx"));
            assert!(body.contains("https://app.clairtyx.com/confirmer-email?token=abc123"));
            assert!(body.contains("2026-09-23T18:00:00Z"));
        }
        assert!(rendered.subject.contains("Clairtyx"));
    }

    /// Le lien de vérification et son expiration apparaissent bien dans les
    /// deux corps. Clés en camelCase, comme pour les autres tests ci-dessus —
    /// forme réelle publiée par `EmailVerificationRequested` (events.rs, svc-auth).
    #[sqlx::test(migrations = "./migrations")]
    async fn email_verification_requested_renders_the_verify_link(db: PgPool) {
        let engine = TemplateEngine::load(&db)
            .await
            .expect("chargement des templates");
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), serde_json::json!("Camille"));
        vars.insert(
            "verifyUrl".to_string(),
            serde_json::json!("https://app.clairtyx.com/verifier-email?token=abc123"),
        );
        vars.insert(
            "expiresAt".to_string(),
            serde_json::json!("2026-09-23T18:00:00Z"),
        );
        let rendered = engine
            .render(&EmailEvent::EmailVerification, &vars)
            .expect("rendu vérification d'e-mail");

        for body in [&rendered.html, &rendered.text] {
            assert!(body.contains("Clairtyx"), "le corps doit citer Clairtyx");
            assert!(!body.contains("Workonva"), "aucun résidu de l'ancien nom");
            assert!(body.contains("https://app.clairtyx.com/verifier-email?token=abc123"));
            assert!(body.contains("2026-09-23T18:00:00Z"));
        }
        assert!(rendered.subject.contains("Clairtyx"));
    }

    /// L'invitation nomme l'invitant et l'entreprise, porte le lien magique
    /// et sa date d'expiration lisible. Clés en camelCase : forme publiée par
    /// `InvitationCreated` (events.rs, svc-auth).
    #[sqlx::test(migrations = "./migrations")]
    async fn invitation_renders_the_company_and_the_magic_link(db: PgPool) {
        let engine = TemplateEngine::load(&db)
            .await
            .expect("chargement des templates");
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), serde_json::json!("Jean Martin"));
        vars.insert("inviterName".to_string(), serde_json::json!("Paul Durand"));
        vars.insert(
            "companyName".to_string(),
            serde_json::json!("Martin Électricité"),
        );
        vars.insert(
            "inviteUrl".to_string(),
            serde_json::json!("https://app.clairtyx.com/invitation?token=abc123"),
        );
        vars.insert("expiresOn".to_string(), serde_json::json!("01/10/2026"));
        let rendered = engine
            .render(&EmailEvent::Invitation, &vars)
            .expect("rendu de l'invitation");

        for body in [&rendered.html, &rendered.text] {
            assert!(body.contains("Jean Martin"));
            assert!(body.contains("Paul Durand"));
            assert!(body.contains("Martin Électricité"));
            assert!(body.contains("https://app.clairtyx.com/invitation?token=abc123"));
            assert!(body.contains("01/10/2026"));
        }
        assert!(rendered.subject.contains("Clairtyx"));
        assert_eq!(
            EmailEvent::from_subject("auth.invitation.created"),
            Some(EmailEvent::Invitation)
        );
    }

    /// Le client retrouve l'adresse, le motif et la nouvelle heure estimée
    /// dans les deux corps. Clés en camelCase : forme publiée par
    /// `ChantierDelayReported` (delay_events.rs, monolithe).
    #[sqlx::test(migrations = "./migrations")]
    async fn delay_reported_renders_the_reason_and_address(db: PgPool) {
        let engine = TemplateEngine::load(&db)
            .await
            .expect("chargement des templates");
        let mut vars = HashMap::new();
        vars.insert("clientName".to_string(), serde_json::json!("Mme Dubois"));
        vars.insert("address".to_string(), serde_json::json!("12 rue des Lilas"));
        vars.insert("workerName".to_string(), serde_json::json!("Paul Durand"));
        vars.insert(
            "reasonLabel".to_string(),
            serde_json::json!("Panne ou problème de route"),
        );
        vars.insert("note".to_string(), serde_json::json!(""));
        vars.insert(
            "newEstimatedTimeOn".to_string(),
            serde_json::json!("21/09/2026 à 10:30"),
        );
        vars.insert(
            "companyName".to_string(),
            serde_json::json!("Dupont Plomberie"),
        );
        let rendered = engine
            .render(&EmailEvent::DelayReported, &vars)
            .expect("rendu du retard signalé");

        for body in [&rendered.html, &rendered.text] {
            assert!(body.contains("Mme Dubois"));
            assert!(body.contains("12 rue des Lilas"));
            assert!(body.contains("Panne ou problème de route"));
            assert!(body.contains("21/09/2026 à 10:30"));
            // Signé au nom de l'entreprise, jamais de Clairtyx : le client
            // final ne connaît pas Clairtyx (voir services::delay_events, monolithe).
            assert!(body.contains("L'équipe Dupont Plomberie"));
            assert!(!body.contains("L'équipe Clairtyx"));
        }
        assert!(rendered.subject.contains("Retard"));
        assert_eq!(
            EmailEvent::from_subject("core.chantier.delay_reported"),
            Some(EmailEvent::DelayReported)
        );
    }

    /// Sans nom d'entreprise (course avec `auth.user.registered`, ou tenant
    /// créé avant ce mécanisme) : signature neutre, jamais "Clairtyx".
    #[sqlx::test(migrations = "./migrations")]
    async fn delay_reported_falls_back_without_a_company_name(db: PgPool) {
        let engine = TemplateEngine::load(&db)
            .await
            .expect("chargement des templates");
        let mut vars = HashMap::new();
        vars.insert("clientName".to_string(), serde_json::json!("Mme Dubois"));
        vars.insert("address".to_string(), serde_json::json!(""));
        vars.insert("workerName".to_string(), serde_json::json!("Paul Durand"));
        vars.insert("reasonLabel".to_string(), serde_json::json!("Absence"));
        vars.insert("note".to_string(), serde_json::json!(""));
        vars.insert("companyName".to_string(), serde_json::json!(""));

        let rendered = engine
            .render(&EmailEvent::DelayReported, &vars)
            .expect("rendu du retard signalé");

        for body in [&rendered.html, &rendered.text] {
            assert!(!body.contains("L'équipe Clairtyx"));
            assert!(body.contains("L'équipe"));
        }
    }

    /// Le retard se présente au nom de l'entreprise ("via Clairtyx"), jamais
    /// de Clairtyx seul ; les autres mails (comptes Clairtyx) restent signés
    /// du nom par défaut.
    #[test]
    fn delay_reported_sender_name_is_the_company() {
        let mut vars = HashMap::new();
        vars.insert(
            "companyName".to_string(),
            serde_json::json!("Dupont Plomberie"),
        );
        assert_eq!(
            EmailEvent::DelayReported.sender_name(&vars, "Clairtyx"),
            "Dupont Plomberie via Clairtyx"
        );

        let empty = HashMap::new();
        assert_eq!(
            EmailEvent::DelayReported.sender_name(&empty, "Clairtyx"),
            "Clairtyx"
        );
        assert_eq!(
            EmailEvent::Welcome.sender_name(&vars, "Clairtyx"),
            "Clairtyx"
        );
    }

    /// Les réponses du client final vont à l'adresse professionnelle du
    /// patron quand elle est connue ; sinon pas d'en-tête `Reply-To` du tout
    /// (jamais l'adresse technique d'expédition en dur).
    #[test]
    fn delay_reported_reply_to_is_the_contact_email() {
        let mut vars = HashMap::new();
        vars.insert(
            "contactEmail".to_string(),
            serde_json::json!("contact@dupont-plomberie.fr"),
        );
        assert_eq!(
            EmailEvent::DelayReported.reply_to(&vars),
            Some("contact@dupont-plomberie.fr")
        );

        let empty = HashMap::new();
        assert_eq!(EmailEvent::DelayReported.reply_to(&empty), None);
        assert_eq!(EmailEvent::Welcome.reply_to(&vars), None);
    }
}
```

</details>

<details><summary><code>migrations/20261027000001_mail_templates.sql</code> (484 lignes)</summary>

```sql
-- Templates d'e-mails transactionnels, éditables sans redéploiement.
-- Clé : `event_type` = sujet du topic Kafka (comme `mail_queue.event_type`).
-- `subject`, `html_body` et `text_body` sont des templates Tera rendus avec le
-- payload de l'événement (clés en camelCase, voir `core/consumer.rs`).

CREATE TABLE mail_templates (
    event_type  TEXT        PRIMARY KEY,
    subject     TEXT        NOT NULL,
    html_body   TEXT        NOT NULL,
    text_body   TEXT        NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

INSERT INTO mail_templates (event_type, subject, html_body, text_body) VALUES
($tpl$auth.user.registered$tpl$, $tpl$Bienvenue sur Clairtyx !$tpl$,
$tpl$<!DOCTYPE html>
<html lang="fr">

<body style="margin:0;padding:0;background-color:#f1f5f9;">

    <span style="display:none;font-size:0;line-height:0;opacity:0;">
        Bienvenue sur Clairtyx – Votre compte est prêt.
    </span>

    <table width="100%" cellpadding="0" cellspacing="0" bgcolor="#f1f5f9">
        <tr>
            <td align="center">

                <table width="600" cellpadding="0" cellspacing="0" bgcolor="#ffffff"
                    style="font-family:Arial,sans-serif;">

                    <tr>
                        <td bgcolor="#4a5568"
                            style="padding:20px;text-align:center;color:#ffffff;font-size:22px;font-weight:bold;">
                            Bienvenue sur Clairtyx
                        </td>
                    </tr>

                    <tr>
                        <td style="padding:30px;font-size:14px;color:#333333;line-height:1.6;">
                            Bonjour {{ name | default(value="") }},<br><br>

                            Votre compte {{ companyName | default(value="") }} sur Clairtyx a été créé avec succès.
                            Vos clients seront désormais prévenus automatiquement en cas de retard sur un
                            chantier.<br><br>

                            Merci de votre confiance.<br><br>
                            Cordialement,<br>
                            L'équipe Clairtyx
                        </td>
                    </tr>

                    <tr>
                        <td align="center" style="padding:15px;font-size:12px;color:#6b7280;">
                            © 2026 Clairtyx – Tous droits réservés
                        </td>
                    </tr>

                </table>

            </td>
        </tr>
    </table>

</body>

</html>$tpl$,
$tpl$Bonjour {{ name | default(value="") }},

Bienvenue sur Clairtyx !

Votre compte {{ companyName | default(value="") }} a été créé avec succès. Vos clients seront désormais
prévenus automatiquement en cas de retard sur un chantier.

Merci de nous avoir rejoint !

Cordialement,
L'équipe Clairtyx
$tpl$),
($tpl$auth.email_change.requested$tpl$, $tpl$Confirmez votre nouvelle adresse e-mail Clairtyx$tpl$,
$tpl$<!DOCTYPE html>
<html lang="fr">

<body style="margin:0;padding:0;background-color:#f1f5f9;">

    <span style="display:none;font-size:0;line-height:0;opacity:0;">
        Confirmez votre nouvelle adresse e-mail sur Clairtyx.
    </span>

    <table width="100%" cellpadding="0" cellspacing="0" bgcolor="#f1f5f9">
        <tr>
            <td align="center">

                <table width="600" cellpadding="0" cellspacing="0" bgcolor="#ffffff"
                    style="font-family:Arial,sans-serif;">

                    <tr>
                        <td bgcolor="#4a5568"
                            style="padding:20px;text-align:center;color:#ffffff;font-size:22px;font-weight:bold;">
                            Confirmez votre nouvelle adresse
                        </td>
                    </tr>

                    <tr>
                        <td style="padding:30px;font-size:14px;color:#333333;line-height:1.6;">
                            Bonjour {{ name | default(value="") }},<br><br>

                            Une demande de changement d'adresse e-mail a été effectuée pour votre compte
                            Clairtyx, vers cette adresse.<br><br>

                            Pour la confirmer, ouvrez ce lien :<br><br>

                            <a href="{{ confirmUrl | safe }}"
                                style="display:inline-block;padding:12px 24px;background-color:#4a5568;color:#ffffff;text-decoration:none;border-radius:6px;">
                                Confirmer cette adresse
                            </a><br><br>

                            Le bouton ne fonctionne pas ? Copiez ce lien :<br>
                            <span style="word-break:break-all;color:#4a5568;">{{ confirmUrl | safe }}</span><br><br>

                            Ce lien expire le {{ expiresAt }} et ne peut être utilisé qu'une seule fois.<br><br>

                            Si vous n'êtes pas à l'origine de cette demande, vous pouvez ignorer ce message :
                            votre adresse actuelle reste inchangée.<br><br>

                            Cordialement,<br>
                            L'équipe Clairtyx
                        </td>
                    </tr>

                    <tr>
                        <td align="center" style="padding:15px;font-size:12px;color:#6b7280;">
                            © 2026 Clairtyx – Tous droits réservés
                        </td>
                    </tr>

                </table>

            </td>
        </tr>
    </table>

</body>

</html>
$tpl$,
$tpl$Bonjour {{ name | default(value="") }},

Une demande de changement d'adresse e-mail a été effectuée pour votre compte Clairtyx, vers cette adresse.

Pour la confirmer, ouvrez ce lien :

{{ confirmUrl }}

Ce lien expire le {{ expiresAt }} et ne peut être utilisé qu'une seule fois.

Si vous n'êtes pas à l'origine de cette demande, vous pouvez ignorer ce message : votre adresse actuelle reste inchangée.

Cordialement,
L'équipe Clairtyx
$tpl$),
($tpl$auth.password_reset.requested$tpl$, $tpl$Réinitialisation de votre mot de passe Clairtyx$tpl$,
$tpl$<!DOCTYPE html>
<html lang="fr">

<body style="margin:0;padding:0;background-color:#f1f5f9;">

    <span style="display:none;font-size:0;line-height:0;opacity:0;">
        Réinitialisez votre mot de passe Clairtyx.
    </span>

    <table width="100%" cellpadding="0" cellspacing="0" bgcolor="#f1f5f9">
        <tr>
            <td align="center">

                <table width="600" cellpadding="0" cellspacing="0" bgcolor="#ffffff"
                    style="font-family:Arial,sans-serif;">

                    <tr>
                        <td bgcolor="#4a5568"
                            style="padding:20px;text-align:center;color:#ffffff;font-size:22px;font-weight:bold;">
                            Réinitialisation du mot de passe
                        </td>
                    </tr>

                    <tr>
                        <td style="padding:30px;font-size:14px;color:#333333;line-height:1.6;">
                            Bonjour {{ name | default(value="") }},<br><br>

                            Une demande de réinitialisation de mot de passe a été effectuée pour votre compte
                            Clairtyx. Cliquez sur le bouton ci-dessous pour choisir un nouveau mot de passe.<br><br>

                            <div style="text-align:center;">
                                <a href="{{ resetUrl | safe }}"
                                    style="display:inline-block;padding:12px 28px;background-color:#4a5568;color:#ffffff;text-decoration:none;font-size:15px;font-weight:bold;border-radius:6px;">
                                    Réinitialiser mon mot de passe
                                </a>
                            </div><br>

                            Ce lien expire le {{ expiresAt }} et ne peut être utilisé qu'une seule fois.<br><br>

                            Si le bouton ne fonctionne pas, copiez ce lien dans votre navigateur :<br>
                            <span style="word-break:break-all;color:#4a5568;">{{ resetUrl | safe }}</span><br><br>

                            Si vous n'êtes pas à l'origine de cette demande, vous pouvez ignorer ce message :
                            votre mot de passe actuel reste inchangé.<br><br>

                            Cordialement,<br>
                            L'équipe Clairtyx
                        </td>
                    </tr>

                    <tr>
                        <td align="center" style="padding:15px;font-size:12px;color:#6b7280;">
                            © 2026 Clairtyx – Tous droits réservés
                        </td>
                    </tr>

                </table>

            </td>
        </tr>
    </table>

</body>

</html>
$tpl$,
$tpl$Bonjour {{ name | default(value="") }},

Une demande de réinitialisation de mot de passe a été effectuée pour votre compte Clairtyx.

Pour choisir un nouveau mot de passe, ouvrez ce lien :

{{ resetUrl }}

Ce lien expire le {{ expiresAt }} et ne peut être utilisé qu'une seule fois.

Si vous n'êtes pas à l'origine de cette demande, vous pouvez ignorer ce message : votre mot de passe actuel reste inchangé.

Cordialement,
L'équipe Clairtyx
$tpl$),
($tpl$auth.email_verification.requested$tpl$, $tpl$Vérifiez votre adresse e-mail Clairtyx$tpl$,
$tpl$<!DOCTYPE html>
<html lang="fr">

<body style="margin:0;padding:0;background-color:#f1f5f9;">

    <span style="display:none;font-size:0;line-height:0;opacity:0;">
        Vérifiez votre adresse e-mail sur Clairtyx.
    </span>

    <table width="100%" cellpadding="0" cellspacing="0" bgcolor="#f1f5f9">
        <tr>
            <td align="center">

                <table width="600" cellpadding="0" cellspacing="0" bgcolor="#ffffff"
                    style="font-family:Arial,sans-serif;">

                    <tr>
                        <td bgcolor="#4a5568"
                            style="padding:20px;text-align:center;color:#ffffff;font-size:22px;font-weight:bold;">
                            Vérifiez votre adresse e-mail
                        </td>
                    </tr>

                    <tr>
                        <td style="padding:30px;font-size:14px;color:#333333;line-height:1.6;">
                            Bonjour {{ name | default(value="") }},<br><br>

                            Bienvenue sur Clairtyx ! Pour activer pleinement votre compte, confirmez que cette
                            adresse e-mail vous appartient.<br><br>

                            Pour la vérifier, ouvrez ce lien :<br><br>

                            <a href="{{ verifyUrl | safe }}"
                                style="display:inline-block;padding:12px 24px;background-color:#4a5568;color:#ffffff;text-decoration:none;border-radius:6px;">
                                Vérifier mon adresse
                            </a><br><br>

                            Le bouton ne fonctionne pas ? Copiez ce lien :<br>
                            <span style="word-break:break-all;color:#4a5568;">{{ verifyUrl | safe }}</span><br><br>

                            Ce lien expire le {{ expiresAt }} et ne peut être utilisé qu'une seule fois.<br><br>

                            Si vous n'êtes pas à l'origine de cette inscription, vous pouvez ignorer ce message.<br><br>

                            Cordialement,<br>
                            L'équipe Clairtyx
                        </td>
                    </tr>

                    <tr>
                        <td align="center" style="padding:15px;font-size:12px;color:#6b7280;">
                            © 2026 Clairtyx – Tous droits réservés
                        </td>
                    </tr>

                </table>

            </td>
        </tr>
    </table>

</body>

</html>
$tpl$,
$tpl$Bonjour {{ name | default(value="") }},

Bienvenue sur Clairtyx ! Pour activer pleinement votre compte, confirmez que cette adresse e-mail vous appartient.

Pour la vérifier, ouvrez ce lien :

{{ verifyUrl }}

Ce lien expire le {{ expiresAt }} et ne peut être utilisé qu'une seule fois.

Si vous n'êtes pas à l'origine de cette inscription, vous pouvez ignorer ce message.

Cordialement,
L'équipe Clairtyx
$tpl$),
($tpl$auth.invitation.created$tpl$, $tpl$Vous êtes invité à rejoindre une équipe sur Clairtyx$tpl$,
$tpl$<!DOCTYPE html>
<html lang="fr">

<body style="margin:0;padding:0;background-color:#f1f5f9;">

    <span style="display:none;font-size:0;line-height:0;opacity:0;">
        {{ inviterName }} vous invite à rejoindre {{ companyName }} sur Clairtyx.
    </span>

    <table width="100%" cellpadding="0" cellspacing="0" bgcolor="#f1f5f9">
        <tr>
            <td align="center">

                <table width="600" cellpadding="0" cellspacing="0" bgcolor="#ffffff"
                    style="font-family:Arial,sans-serif;">

                    <tr>
                        <td bgcolor="#4a5568"
                            style="padding:20px;text-align:center;color:#ffffff;font-size:22px;font-weight:bold;">
                            Rejoignez {{ companyName }}
                        </td>
                    </tr>

                    <tr>
                        <td style="padding:30px;font-size:14px;color:#333333;line-height:1.6;">
                            Bonjour {{ name | default(value="") }},<br><br>

                            {{ inviterName }} vous invite à rejoindre l'équipe de <strong>{{ companyName }}</strong>
                            sur Clairtyx, l'application qui prévient automatiquement les clients quand un
                            intervenant est en retard.<br><br>

                            <a href="{{ inviteUrl | safe }}"
                                style="display:inline-block;padding:12px 24px;background-color:#4a5568;color:#ffffff;text-decoration:none;border-radius:6px;">
                                Rejoindre l'équipe
                            </a><br><br>

                            Le bouton ne fonctionne pas ? Copiez ce lien :<br>
                            <span style="word-break:break-all;color:#4a5568;">{{ inviteUrl | safe }}</span><br><br>

                            Vous verrez vos chantiers et pourrez signaler un retard depuis votre téléphone.
                            Ce lien expire le {{ expiresOn | safe }} et ne peut être utilisé qu'une seule fois.<br><br>

                            Si vous ne connaissez pas {{ companyName }}, vous pouvez ignorer ce message.<br><br>

                            Cordialement,<br>
                            L'équipe Clairtyx
                        </td>
                    </tr>

                    <tr>
                        <td align="center" style="padding:15px;font-size:12px;color:#6b7280;">
                            © 2026 Clairtyx – Tous droits réservés
                        </td>
                    </tr>

                </table>

            </td>
        </tr>
    </table>

</body>

</html>
$tpl$,
$tpl$Bonjour {{ name | default(value="") }},

{{ inviterName }} vous invite à rejoindre l'équipe de {{ companyName }} sur Clairtyx, l'application qui prévient automatiquement les clients quand un intervenant est en retard.

Pour accepter l'invitation, ouvrez ce lien :

{{ inviteUrl }}

Vous verrez vos chantiers et pourrez signaler un retard depuis votre téléphone. Ce lien expire le {{ expiresOn }} et ne peut être utilisé qu'une seule fois.

Si vous ne connaissez pas {{ companyName }}, vous pouvez ignorer ce message.

Cordialement,
L'équipe Clairtyx
$tpl$),
($tpl$core.chantier.delay_reported$tpl$, $tpl$Retard signalé sur votre chantier$tpl$,
$tpl$<!DOCTYPE html>
<html lang="fr">

<body style="margin:0;padding:0;background-color:#f1f5f9;">

    <span style="display:none;font-size:0;line-height:0;opacity:0;">
        {{ workerName }} est en retard sur votre chantier{% if address %} au {{ address }}{% endif %}.
    </span>

    <table width="100%" cellpadding="0" cellspacing="0" bgcolor="#f1f5f9">
        <tr>
            <td align="center">

                <table width="600" cellpadding="0" cellspacing="0" bgcolor="#ffffff"
                    style="font-family:Arial,sans-serif;">

                    <tr>
                        <td bgcolor="#b45309"
                            style="padding:20px;text-align:center;color:#ffffff;font-size:22px;font-weight:bold;">
                            Retard signalé sur votre chantier
                        </td>
                    </tr>

                    <tr>
                        <td style="padding:30px;font-size:14px;color:#333333;line-height:1.6;">
                            Bonjour {{ clientName | default(value="") }},<br><br>

                            {{ workerName }} vous informe d'un retard sur votre chantier
                            {% if address %}au <strong>{{ address }}</strong>{% endif %}.<br><br>

                            <strong>Motif :</strong> {{ reasonLabel }}<br>
                            {% if note %}
                            <strong>Précisions :</strong> {{ note }}<br>
                            {% endif %}
                            {% if newEstimatedTimeOn %}
                            <strong>Nouvelle heure estimée :</strong> {{ newEstimatedTimeOn | safe }}<br>
                            {% endif %}
                            <br>

                            Nous vous prions de nous excuser pour la gêne occasionnée.<br><br>

                            Cordialement,<br>
                            {% if companyName %}L'équipe {{ companyName }}{% else %}L'équipe{% endif %}
                        </td>
                    </tr>

                    <tr>
                        <td align="center" style="padding:15px;font-size:12px;color:#6b7280;">
                            Notification envoyée via Clairtyx{% if companyName %} pour le compte de {{ companyName }}{% endif %}
                        </td>
                    </tr>

                </table>

            </td>
        </tr>
    </table>

</body>

</html>
$tpl$,
$tpl$Bonjour {{ clientName | default(value="") }},

{{ workerName }} vous informe d'un retard sur votre chantier{% if address %} au {{ address }}{% endif %}.

Motif : {{ reasonLabel }}
{% if note %}Précisions : {{ note }}
{% endif %}{% if newEstimatedTimeOn %}Nouvelle heure estimée : {{ newEstimatedTimeOn }}
{% endif %}
Nous vous prions de nous excuser pour la gêne occasionnée.

Cordialement,
{% if companyName %}L'équipe {{ companyName }}{% else %}L'équipe{% endif %}

Notification envoyée via Clairtyx{% if companyName %} pour le compte de {{ companyName }}{% endif %}
$tpl$);
```

</details>

<details><summary><code>migrations/20261027000002_welcome_without_company.sql</code> (13 lignes)</summary>

```sql
-- Le mail de bienvenue est celui de la plateforme : à l'inscription, l'entreprise n'est pas encore
-- connue, le payload de `auth.user.registered` ne porte plus `companyName`.
UPDATE mail_templates
SET html_body = replace(
        html_body,
        'Votre compte {{ companyName | default(value="") }} sur Clairtyx a été créé',
        'Votre compte sur Clairtyx a été créé'),
    text_body = replace(
        text_body,
        'Votre compte {{ companyName | default(value="") }} a été créé',
        'Votre compte a été créé'),
    updated_at = NOW()
WHERE event_type = 'auth.user.registered';
```

</details>

