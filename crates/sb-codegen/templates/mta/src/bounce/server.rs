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
        &format!("220 {} ESMTP Bounce Receiver\r\n", hostname),
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

#[cfg(test)]
mod extra_tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const PERMANENT: &str = "In-Reply-To: <abc@example.dev>\r\n\r\nFinal-Recipient: rfc822; x@exemple.fr\r\nAction: failed\r\nStatus: 5.1.1\r\nDiagnostic-Code: smtp; 550 5.1.1 User unknown\r\n";
    const TRANSIENT: &str = "Original-Message-ID: <def@example.dev>\r\nStatus: 4.2.2\r\nDiagnostic-Code: smtp; 452 mailbox full\r\n";

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
            Some("<abc@example.dev>")
        );
        assert_eq!(
            extract_original_message_id(TRANSIENT).as_deref(),
            Some("<def@example.dev>")
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
            handle_connection(stream, Arc::new("mx.example.dev".to_string()), db).await
        });

        let mut client = tokio::net::TcpStream::connect(addr).await.unwrap();
        let script = format!(
            "EHLO relais.exemple.fr\r\nMAIL FROM:<>\r\nRCPT TO:<bounce@example.dev>\r\nDATA\r\n{PERMANENT}..ligne avec point\r\n.\r\nVRFY x\r\nQUIT\r\n"
        );
        client.write_all(script.as_bytes()).await.unwrap();
        let mut transcript = String::new();
        client.read_to_string(&mut transcript).await.unwrap();
        server.await.unwrap().unwrap();

        assert!(
            transcript.starts_with("220 mx.example.dev ESMTP Bounce Receiver"),
            "{transcript}"
        );
        for expected in [
            "250-mx.example.dev",
            "250 OK",
            "354 ",
            "250 Ok: message reçu",
            "502 Commande non implémentée",
            "221 mx.example.dev Bye",
        ] {
            assert!(
                transcript.contains(expected),
                "{expected} absent de :\n{transcript}"
            );
        }
    }
}
