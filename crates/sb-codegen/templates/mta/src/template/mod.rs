use anyhow::{Context, Result};
use serde_json::Value;
use sqlx::PgPool;
use std::collections::HashMap;
use tera::{Context as TeraCtx, Tera};

use crate::mta_rules::{EventRule, rule_for};

/// Un événement écouté, vu du worker : la règle (`mta_rules::EventRule`) qui le décrit.
///
/// `event_type` est le topic, qui est aussi la clé du template en base
/// (`mail_templates.event_type`) et le `event_type` stocké dans `mail_queue`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmailEvent(&'static EventRule);

impl EmailEvent {
    /// Convertit le nom du topic (= `event_type` stocké dans mail_queue) en événement connu.
    pub fn from_subject(s: &str) -> Option<Self> {
        rule_for(s).map(Self)
    }

    pub fn event_type(&self) -> &'static str {
        self.0.topic
    }

    /// Nom d'expéditeur affiché (en-tête `From`) : celui de la configuration, ou
    /// `<entreprise> via <nom>` quand la règle le demande et que le champ est renseigné.
    pub fn sender_name(&self, vars: &HashMap<String, Value>, default_name: &str) -> String {
        self.0.sender_name(vars, default_name)
    }

    /// Adresse `Reply-To`, si la règle en désigne une et qu'elle est renseignée.
    pub fn reply_to<'v>(&self, vars: &'v HashMap<String, Value>) -> Option<&'v str> {
        self.0.reply_to(vars)
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
