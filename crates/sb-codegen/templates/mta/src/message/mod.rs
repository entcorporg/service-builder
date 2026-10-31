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
            "notifications@example.com",
            "Dupont Plomberie via Example",
            "client@exemple.fr",
            None,
            Some("contact@dupont-plomberie.fr"),
            "Retard signalé",
            "<p>html</p>",
            "text",
            "example.com",
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
            "notifications@example.com",
            "Example",
            "client@exemple.fr",
            None,
            None,
            "Bienvenue",
            "<p>html</p>",
            "text",
            "example.com",
        )
        .unwrap();
        let raw = String::from_utf8(msg.raw).unwrap();
        assert!(!raw.contains("Reply-To"));
    }
}
