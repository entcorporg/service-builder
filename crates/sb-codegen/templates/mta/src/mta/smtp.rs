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
/// - `from`       : adresse enveloppe MAIL FROM (ex: "noreply@example.dev")
/// - `to`         : adresse enveloppe RCPT TO (ex: "jean@gmail.com")
/// - `raw_message`: message RFC 5322 complet (avec DKIM-Signature en tête)
/// - `our_domain` : notre domaine pour le EHLO (ex: "example.dev")
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
    // Le message se termine déjà par CRLF : pas de ligne vide avant le point final.
    let terminator: &[u8] = if body.ends_with(b"\r\n") {
        b".\r\n"
    } else {
        b"\r\n.\r\n"
    };
    transport.write_all(terminator).await?;

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
    for line in message.split_inclusive(|&b| b == b'\n') {
        if line.starts_with(b".") {
            out.push(b'.');
        }
        out.extend_from_slice(line);
    }
    out
}

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
            "noreply@example.dev",
            "client@exemple.fr",
            raw,
            "example.dev",
        )
        .await
        .unwrap();
        let received = server.await.unwrap();
        assert_eq!(received[0], "EHLO example.dev\r\n");
        assert_eq!(received[1], "MAIL FROM:<noreply@example.dev>\r\n");
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
            "a@example.dev",
            "x@exemple.fr",
            b"Subject: x\r\n\r\ny\r\n",
            "example.dev",
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
            "a@example.dev",
            "x@exemple.fr",
            b"Subject: x\r\n\r\ny\r\n",
            "example.dev",
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
            "a@example.dev",
            "x@exemple.fr",
            raw,
            "example.dev",
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
