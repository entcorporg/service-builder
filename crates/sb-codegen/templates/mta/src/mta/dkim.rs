//! Signature DKIM des messages RFC 5322 (RFC 6376).
//!
//! Utilise le crate `mail-auth` pour signer avec RSA-2048/SHA-256
//! ou ED25519 selon la clé fournie.
//!
//! La signature DKIM-Signature: est **préfixée** au message brut
//! car certains serveurs vérifient l'ordre des en-têtes.

use anyhow::Result;
use mail_auth::common::headers::HeaderWriter;
use rustls_pki_types::{PrivateKeyDer, pem::PemObject};
use mail_auth::{
    common::crypto::{RsaKey, Sha256},
    dkim::DkimSigner,
};

/// Signe un message RFC 5322 et retourne le message avec le header
/// `DKIM-Signature:` correctement positionné en tête.
///
/// # Paramètres
/// - `raw`         : bytes du message RFC 5322 (sans DKIM, produit par `message::build`)
/// - `pem`         : clé privée RSA au format PEM (PKCS#1 ou PKCS#8)
/// - `domain`      : domaine signataire (ex: "example.dev")
/// - `selector`    : sélecteur DNS (ex: "mail" → TXT mail._domainkey.example.dev)
pub fn sign(raw: &[u8], pem: &str, domain: &str, selector: &str) -> Result<Vec<u8>> {
    // Une clé PEM est « RSA PRIVATE KEY » (PKCS#1) ou « PRIVATE KEY » (PKCS#8, ce que produit `openssl genrsa` depuis
    // OpenSSL 3) : les deux sont acceptées. `from_rsa_pem` ne lit que la première.
    let key_der = PrivateKeyDer::from_pem_slice(pem.as_bytes())
        .map_err(|e| anyhow::anyhow!("DKIM : chargement clé RSA échoué — {e}"))?;
    let rsa_key = RsaKey::<Sha256>::from_key_der(key_der)
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
            "mail._domainkey.example.dev.".to_string(),
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
            "noreply@example.dev",
            "Example",
            "client@exemple.fr",
            Some("Camille"),
            None,
            "Bonjour",
            "<p>html</p>",
            "texte",
            "example.dev",
        )
        .unwrap()
    }

    /// `openssl genrsa` écrit du PKCS#8 (« BEGIN PRIVATE KEY ») : la clé d'un opérateur n'est pas toujours du PKCS#1.
    #[tokio::test]
    async fn a_pkcs8_key_signs_as_well_as_a_pkcs1_key() {
        use rsa::pkcs8::EncodePrivateKey;
        let key = rsa::RsaPrivateKey::new(&mut rsa::rand_core::OsRng, 2048).unwrap();
        let pem = key.to_pkcs8_pem(rsa::pkcs8::LineEnding::LF).unwrap().to_string();
        assert!(pem.starts_with("-----BEGIN PRIVATE KEY-----"));
        let public = STANDARD.encode(key.to_public_key().to_public_key_der().unwrap().as_bytes());
        let signed = sign(&message().raw, &pem, "example.dev", "mail").unwrap();
        assert_eq!(
            verdict(&signed, &zone_with_key(&public)).await,
            format!("{:?}", DkimResult::Pass)
        );
        assert!(sign(&message().raw, "pas une clé", "example.dev", "mail").is_err());
    }

    #[test]
    fn the_signature_is_prepended_and_the_message_is_untouched() {
        let (pem, _) = throwaway_key();
        let msg = message();
        let signed =
            String::from_utf8(sign(&msg.raw, &pem, "example.dev", "mail").unwrap()).unwrap();
        let raw = String::from_utf8(msg.raw).unwrap();
        assert!(signed.starts_with("DKIM-Signature:"));
        assert!(
            signed.ends_with(&raw),
            "le message d'origine suit, octet pour octet"
        );
        let header: String = signed[..signed.len() - raw.len()]
            .replace("\r\n ", " ")
            .replace("\r\n\t", " ");
        for expected in ["d=example.dev", "s=mail", "a=rsa-sha256", "bh="] {
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
        let signed = sign(&message().raw, &pem, "example.dev", "mail").unwrap();
        assert_eq!(
            verdict(&signed, &zone_with_key(&public)).await,
            format!("{:?}", DkimResult::Pass)
        );
    }

    #[tokio::test]
    async fn tampering_after_signing_breaks_verification() {
        let (pem, public) = throwaway_key();
        let mut signed = sign(&message().raw, &pem, "example.dev", "mail").unwrap();
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
        let signed = sign(&message().raw, &pem, "example.dev", "mail").unwrap();
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
                "example.dev",
                "mail"
            )
            .is_err()
        );
    }
}
