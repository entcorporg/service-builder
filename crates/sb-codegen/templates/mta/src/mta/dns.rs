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
