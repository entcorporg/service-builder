# Service Builder : ce qu'il doit couvrir pour produire Clairtyx

Cible : générer, sans `code` (l'échappement en Rust brut), des services comme `svc-auth`.
Ce document liste les éléments de logique manquants, avec pour chacun une **étape proposée**
et un **code de démo exécuté**.

## Légende et état des preuves

| Marque | Sens |
|---|---|
| ✅ | Code compilé et testé (`cargo test`), voir « Comment rejouer » |
| 📝 | SQL non exécuté : aucun accès PostgreSQL dans la session (les `.env` sont interdits). À rejouer avec `sqlx::test` |
| 💡 | Proposition de schéma JSON pour le builder, **non validée** : ces types d'étape n'existent pas encore |

Résultat des démos : **65 tests** (64 exécutés d'office, plus 1 vérifié à part sur la vraie migration du MTA),
`cargo clippy --all-targets -- -D warnings` et `cargo fmt --check` propres, avec les versions du workspace
`backend/Cargo.toml` (axum 0.8, argon2 0.6, sha2 0.11, hmac 0.13, rand 0.10, tokio 1.53, jsonwebtoken 11,
webauthn-rs 0.5, aws-sdk-s3 1). Le test « le runtime ne se fige pas pendant le hachage » a été vérifié par un
**contrôle négatif** : avec `hash` exécuté directement dans la tâche async, il échoue.

Le **MTA** a son propre document, [`SERVICE_BUILDER_MTA.md`](SERVICE_BUILDER_MTA.md) (code complet du service).

Étapes existantes du builder : `log`, `let`, `publish`, `callRpc`, `branch`, `respond`, `sql`, `code`.
`respond` est réservé aux RPC ; `Expr` n'a que `path`, `literal`, `raw` ; `FieldType` n'a ni `uuid`,
ni `Option`, ni `Vec`.

## Vue d'ensemble

| # | Besoin | Étape proposée | Source Clairtyx | Démo |
|---|---|---|---|---|
| 1 | Sortie anticipée avec statut HTTP (400, 401, 403, 404, 409, 429, 500) | `fail` | `AppError` | ✅ §1 |
| 2 | Validation du mot de passe, des e-mails, du SIREN | `Field.rules` | `password::validate` | ✅ §2 |
| 3 | Hachage argon2id hors du runtime async | `hashPassword` (blocking) | `password.rs:40` | ✅ §3 |
| 4 | Vérification en temps constant, hash factice | `verifyPassword` | `password.rs:53` | ✅ §3 |
| 5 | Jetons aléatoires, SHA-256, comparaison constante | `randomToken`, `sha256` | `tokens.rs` | ✅ §4 |
| 6 | Limitation de débit par clé (429 + `Retry-After`) | `rateLimit` | login, 2FA, reset | ✅ §5 |
| 7 | Réponse HTTP avec cookies et en-têtes | `respond` étendu | `open_session`, `cookies.rs` | ✅ §6 |
| 8 | Chiffrement au repos (secret TOTP) | `encrypt`, `decrypt` | `totp_encryption_key` | ✅ §7 |
| 9 | Webhook entrant signé HMAC, sans session | `verifyHmac` | `/webhook/stripe`, `/connect/webhook` | ✅ §8 |
| 10 | Tâche périodique avec arrêt propre | `schedule` (niveau service) | `cleaner.rs` | ✅ §9 |
| 11 | Consommateur idempotent (at-least-once) | `Consumer.idempotent` | topics `auth.*` | ✅ logique, 📝 SQL §10 |
| 12 | Jeton à usage unique, consommation atomique | `sql` avec `RETURNING` | e-mail, reset, invitation | 📝 §11 |
| 13 | Soft delete, FK composites multi-tenant, outbox | `sql` enrichi | `clairtyx.md` | 📝 §12 |
| 14 | Assemblage : le login complet | handler généré | `login.rs` | ✅ §6 |
| 15 | Access token Ed25519, JWKS public, rotation par `kid` | `signJwt`, `verifyJwt`, route `jwks` | `jwt.rs`, `jwks.rs` | ✅ §13 |
| 16 | 2FA TOTP : secret chiffré, tolérance, anti-rejeu, codes de récupération | `totp*`, `recoveryCodes` | `totp/`, `two_factor/` | ✅ logique, 📝 SQL §14 |
| 17 | Passkeys WebAuthn en deux temps, état en base | `passkeyStart`, `passkeyFinish` | `passkeys/` | ✅ §15 |
| 18 | Stockage objet S3 (photos), type reconnu aux octets | `objectPut`, `objectGet`, `objectDelete` | `svc-user` `avatar.rs` | ✅ §16 |
| 19 | Service MTA préfabriqué : événements et templates en données | `kind: "mta"` | `svc-mta` | ✅ voir `SERVICE_BUILDER_MTA.md` |

---

## 0. Comment rejouer

Crée un crate avec ce `Cargo.toml`, puis un fichier par bloc de code ci-dessous, nommé comme son
commentaire de tête (`src/error.rs`, etc.) :

```toml
[package]
name = "sb-demos"
version = "0.1.0"
edition = "2024"

[dependencies]
axum = "~0.8.9"
tokio = { version = "1.53.1", features = ["full", "test-util"] }
argon2 = "0.6.0"
sha2 = "0.11.0"
hmac = "0.13"
subtle = "2.6"
rand = "0.10"
hex = "0.4"
aes-gcm = "0.10"
thiserror = "2.0.19"
serde = { version = "1.0.229", features = ["derive"] }
serde_json = "1.0.151"
tracing = "0.1.44"
jsonwebtoken = { version = "11.1.0", default-features = false, features = ["rust_crypto"] }
ed25519-dalek = "2.1.1"
base64 = "0.22"
chrono = { version = "0.4.45", features = ["serde"] }
uuid = { version = "1.26.1", features = ["v4", "serde"] }
totp-rs = { version = "5", features = ["gen_secret", "otpauth"] }
webauthn-rs = { version = "0.5", features = ["danger-allow-state-serialisation", "conditional-ui"] }
aws-config = "1"
aws-sdk-s3 = "1"
async-trait = "0.1.92"
tera = "1"
anyhow = "1.0.104"

[dev-dependencies]
webauthn-authenticator-rs = { version = "0.5", default-features = false, features = ["softtoken"] }
tower = { version = "0.5", features = ["util"] }
http-body-util = "0.1"

[profile.dev.package."*"]
opt-level = 3
```

`src/lib.rs` :

```rust
pub mod consumer;
pub mod crypto_box;
pub mod error;
pub mod jobs;
pub mod jwt;
pub mod login;
pub mod mta_rules;
pub mod password;
pub mod rate_limit;
pub mod s3;
pub mod tokens;
pub mod totp;
pub mod validation;
pub mod webauthn;
pub mod webhook;
```

```bash
cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check
```

Le `[profile.dev.package."*"] opt-level = 3` évite qu'argon2 prenne plusieurs secondes par test en debug.

**Copie aussi `backend/Cargo.lock` à côté du `Cargo.toml`.** Sans lui, une résolution neuve peut mélanger deux
versions de `aws-smithy-schema` et `aws-config` ne compile plus (constaté pendant ces essais). Le `Cargo.lock`
du workspace fige un ensemble cohérent.

---

## 1. `fail` : sortie anticipée avec statut HTTP ✅

**Manque.** `branch` ne peut pas arrêter le handler avec un 4xx, et `respond` ne sert que les RPC.
Aujourd'hui, tout refus passe par `code`.

💡 **Étape proposée**

```json
{ "type": "fail", "id": "l3", "status": 401, "message": "identifiants invalides" }
{ "type": "fail", "id": "l4", "status": 429, "message": "trop de tentatives",
   "retryAfter": { "kind": "path", "path": "wait_secs" } }
```

Règles à générer : message générique côté client pour les 500, détail dans `tracing::error!`.

```rust
// src/error.rs
//! Sortie anticipée avec statut HTTP : l'étape `fail` du builder.

use axum::Json;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum Fail {
    #[error("{0}")]
    BadRequest(String),
    #[error("identifiants invalides")]
    Unauthorized,
    #[error("accès refusé")]
    Forbidden,
    #[error("introuvable")]
    NotFound,
    #[error("{0}")]
    Conflict(String),
    #[error("trop de tentatives")]
    TooManyRequests { retry_after_secs: u64 },
    /// Dépendance externe indisponible (stockage objet...).
    #[error("{0}")]
    Unavailable(String),
    /// Le détail part dans les logs, jamais chez le client.
    #[error("{0}")]
    Internal(String),
}

pub type FailResult<T> = Result<T, Fail>;

impl IntoResponse for Fail {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            Fail::BadRequest(m) => (StatusCode::BAD_REQUEST, m.clone()),
            Fail::Unauthorized => (StatusCode::UNAUTHORIZED, self.to_string()),
            Fail::Forbidden => (StatusCode::FORBIDDEN, self.to_string()),
            Fail::NotFound => (StatusCode::NOT_FOUND, self.to_string()),
            Fail::Conflict(m) => (StatusCode::CONFLICT, m.clone()),
            Fail::TooManyRequests { .. } => (StatusCode::TOO_MANY_REQUESTS, self.to_string()),
            Fail::Unavailable(what) => {
                tracing::warn!(%what, "dépendance indisponible");
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "Service momentanément indisponible".to_owned(),
                )
            }
            Fail::Internal(detail) => {
                tracing::error!(%detail, "erreur interne");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Erreur interne".to_owned(),
                )
            }
        };
        let mut response = (status, Json(json!({ "error": message }))).into_response();
        if let Fail::TooManyRequests { retry_after_secs } = self {
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from(retry_after_secs));
        }
        response
    }
}
```

<details><summary>Tests de <code>error.rs</code></summary>

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_status_and_hides_internal_detail() {
        assert_eq!(Fail::Unauthorized.into_response().status(), 401);
        assert_eq!(
            Fail::Conflict("déjà pris".into()).into_response().status(),
            409
        );
        let res = Fail::Internal("connexion pg: secret".into()).into_response();
        assert_eq!(res.status(), 500);
    }

    #[test]
    fn too_many_requests_carries_retry_after() {
        let res = Fail::TooManyRequests {
            retry_after_secs: 42,
        }
        .into_response();
        assert_eq!(res.status(), 429);
        assert_eq!(res.headers()[header::RETRY_AFTER], "42");
    }
}
```

</details>


---

## 2. Validation déclarative ✅

**Manque.** `Field` ne porte qu'un type. Chaque règle (longueur, e-mail, SIREN) est écrite à la main.
Les longueurs se comptent en **caractères**, pas en octets, et la borne haute du mot de passe protège
d'un déni de service par argon2.

💡 **Proposition** : `Field.rules`

```json
{ "name": "password", "type": { "kind": "string" },
   "rules": [{ "rule": "length", "min": 10, "max": 128 }] }
{ "name": "email", "type": { "kind": "string" }, "rules": [{ "rule": "email" }] }
{ "name": "siren", "type": { "kind": "string" }, "rules": [{ "rule": "siren" }] }
```

```rust
// src/validation.rs
//! Validation déclarative des champs du body (`Field.rules` dans le builder).

use crate::error::{Fail, FailResult};

pub fn email(value: &str) -> FailResult<()> {
    let value = value.trim();
    let valid = value.len() <= 254
        && value.split_once('@').is_some_and(|(local, domain)| {
            !local.is_empty() && domain.contains('.') && !domain.ends_with('.')
        });
    if valid {
        Ok(())
    } else {
        Err(Fail::BadRequest("Adresse e-mail invalide".into()))
    }
}

pub fn length(field: &str, value: &str, min: usize, max: usize) -> FailResult<()> {
    let len = value.trim().chars().count();
    if (min..=max).contains(&len) {
        Ok(())
    } else {
        Err(Fail::BadRequest(format!(
            "{field} : entre {min} et {max} caractères"
        )))
    }
}

/// SIREN : 9 chiffres et clé de Luhn.
pub fn siren(value: &str) -> FailResult<()> {
    let digits: Vec<u32> = value.chars().filter_map(|c| c.to_digit(10)).collect();
    if value.len() != 9 || digits.len() != 9 {
        return Err(Fail::BadRequest("SIREN : 9 chiffres attendus".into()));
    }
    let sum: u32 = digits
        .iter()
        .rev()
        .enumerate()
        .map(|(i, &d)| {
            if i % 2 == 1 {
                let x = d * 2;
                if x > 9 { x - 9 } else { x }
            } else {
                d
            }
        })
        .sum();
    if sum.is_multiple_of(10) {
        Ok(())
    } else {
        Err(Fail::BadRequest("SIREN : clé de contrôle invalide".into()))
    }
}
```

<details><summary>Tests de <code>validation.rs</code></summary>

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_rules() {
        assert!(email("a@b.fr").is_ok());
        assert!(email(" a@b.fr ").is_ok());
        for bad in [
            "",
            "a",
            "@b.fr",
            "a@b",
            "a@b.",
            &format!("{}@b.fr", "a".repeat(260)),
        ] {
            assert!(email(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn length_rules() {
        assert!(length("nom", "Al", 2, 5).is_ok());
        assert!(length("nom", " A ", 2, 5).is_err());
        assert!(length("nom", "Alexandre", 2, 5).is_err());
    }

    #[test]
    fn siren_luhn() {
        assert!(siren("732829320").is_ok()); // SIREN public valide
        assert!(siren("732829321").is_err());
        assert!(siren("12345678").is_err());
        assert!(siren("73282932a").is_err());
    }
}
```

</details>


---

## 3. `hashPassword` et `verifyPassword` : argon2id dans `spawn_blocking` ✅

**Manque.** Rien dans le builder ne sort un calcul CPU du runtime async. Deux pièges à encoder une fois
pour toutes :

1. argon2 (CPU + mémoire) doit tourner dans `spawn_blocking`, sinon il fige les autres requêtes.
2. Un e-mail inconnu doit coûter **le même temps** qu'un mauvais mot de passe (hash factice), sinon le
   délai révèle quels comptes existent.

💡 **Étapes proposées**

```json
{ "type": "hashPassword", "id": "r5", "input": { "kind": "path", "path": "input.password" }, "bind": "password_hash" }
{ "type": "verifyPassword", "id": "l2",
   "password": { "kind": "path", "path": "input.password" },
   "stored":   { "kind": "path", "path": "account_hash" },
   "bind": "password_ok" }
```

`stored` accepte `Option<String>` : `None` déclenche la vérification contre le hash factice.
Une étape générique `blocking` (bloc de Rust dans `spawn_blocking`, résultat lié à `bind`) couvrirait
les autres cas CPU.

```rust
// src/password.rs
//! Étapes `validatePassword`, `hashPassword`, `verifyPassword`.

use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use std::sync::LazyLock;

use crate::error::{Fail, FailResult};

pub const MIN_LEN: usize = 10;
/// Borne haute : argon2 coûte cher, on refuse les mots de passe géants (DoS).
pub const MAX_LEN: usize = 128;

/// Vérifié quand l'e-mail est inconnu, pour que la réponse prenne le même temps
/// qu'un vrai refus.
static DUMMY_HASH: LazyLock<String> = LazyLock::new(|| {
    Argon2::default()
        .hash_password(b"mot de passe factice")
        .expect("hash factice")
        .to_string()
});

pub fn validate(password: &str) -> FailResult<()> {
    let len = password.chars().count();
    if len < MIN_LEN {
        return Err(Fail::BadRequest(format!(
            "Le mot de passe doit contenir au moins {MIN_LEN} caractères"
        )));
    }
    if len > MAX_LEN {
        return Err(Fail::BadRequest(format!(
            "Le mot de passe ne doit pas dépasser {MAX_LEN} caractères"
        )));
    }
    Ok(())
}

/// Étape `blocking` : argon2 est volontairement lent, hors du runtime async.
pub async fn hash(password: String) -> FailResult<String> {
    tokio::task::spawn_blocking(move || {
        Argon2::default()
            .hash_password(password.as_bytes())
            .map(|h| h.to_string())
            .map_err(|e| Fail::Internal(format!("hachage du mot de passe: {e}")))
    })
    .await
    .map_err(|e| Fail::Internal(e.to_string()))?
}

/// `stored = None` : compte inconnu, on vérifie contre le hash factice et on
/// répond faux.
pub async fn verify(password: String, stored: Option<String>) -> bool {
    tokio::task::spawn_blocking(move || {
        let known = stored.is_some();
        let hash = stored.unwrap_or_else(|| DUMMY_HASH.clone());
        let Ok(parsed) = PasswordHash::new(&hash) else {
            return false;
        };
        let ok = Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok();
        ok && known
    })
    .await
    .unwrap_or(false)
}
```

<details><summary>Tests de <code>password.rs</code></summary>

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_short_and_huge_passwords() {
        assert!(validate("court").is_err());
        assert!(validate(&"a".repeat(MAX_LEN + 1)).is_err());
        assert!(validate("un mot de passe correct").is_ok());
    }

    #[test]
    fn length_counts_chars_not_bytes() {
        // 10 caractères, 20 octets
        assert!(validate("éééééééééé").is_ok());
    }

    #[tokio::test]
    async fn hash_then_verify() {
        let hash = hash("un mot de passe correct".into()).await.unwrap();
        assert!(hash.starts_with("$argon2id$"));
        assert!(verify("un mot de passe correct".into(), Some(hash.clone())).await);
        assert!(!verify("autre mot de passe".into(), Some(hash)).await);
    }

    #[tokio::test]
    async fn unknown_account_never_verifies() {
        assert!(!verify("mot de passe factice".into(), None).await);
    }

    #[tokio::test]
    async fn corrupted_hash_never_verifies() {
        assert!(!verify("x".into(), Some("pas un hash".into())).await);
    }

    #[tokio::test]
    async fn blocking_work_does_not_starve_the_runtime() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::time::Duration;

        // Runtime à 1 thread : pendant le hachage, le minuteur doit continuer de
        // tourner. Un argon2 exécuté directement dans la tâche async le figerait.
        let ticks = Arc::new(AtomicU32::new(0));
        let counter = ticks.clone();
        let ticker = tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(1)).await;
                counter.fetch_add(1, Ordering::SeqCst);
            }
        });
        tokio::time::sleep(Duration::from_millis(5)).await;
        let before = ticks.load(Ordering::SeqCst);
        hash("un mot de passe correct".into()).await.unwrap();
        let during = ticks.load(Ordering::SeqCst) - before;
        ticker.abort();
        assert!(during > 0, "le runtime est resté figé pendant le hachage");
    }
}
```

</details>


---

## 4. `randomToken`, `sha256`, comparaison constante ✅

**Manque.** Les jetons de session, de vérification d'e-mail, de reset et d'invitation suivent tous le
même schéma : 256 bits aléatoires, **seul le hash est stocké**, comparaison en temps constant. Le jeton
est déjà à haute entropie : SHA-256 suffit, inutile d'employer argon2.

💡 **Étapes proposées**

```json
{ "type": "randomToken", "id": "s2", "bind": "token" }
{ "type": "sha256", "id": "s3", "input": { "kind": "path", "path": "token" }, "bind": "token_hash" }
```

Prérequis de schéma : `auth_tokens` n'a pas de colonne `token_hash` (`bytea`), il faut l'ajouter.

```rust
// src/tokens.rs
//! Étapes `randomToken`, `sha256`, `constantTimeEq`.

use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

/// 32 octets aléatoires (256 bits) en hexadécimal.
pub fn generate() -> String {
    let mut bytes = [0u8; 32];
    rand::fill(&mut bytes);
    hex::encode(bytes)
}

/// Empreinte stockée en base : le jeton est déjà à haute entropie, un hash
/// rapide suffit.
pub fn hash(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

/// Comparaison en temps constant.
pub fn matches(token: &str, stored_hash: &[u8]) -> bool {
    hash(token).ct_eq(stored_hash).into()
}
```

<details><summary>Tests de <code>tokens.rs</code></summary>

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_unique_and_long() {
        let (a, b) = (generate(), generate());
        assert_ne!(a, b);
        assert_eq!(a.len(), 64);
    }

    #[test]
    fn hash_is_deterministic() {
        assert_eq!(hash("abc"), hash("abc"));
        assert_ne!(hash("abc"), hash("abd"));
    }

    #[test]
    fn matches_only_the_right_token() {
        let token = generate();
        let stored = hash(&token);
        assert!(matches(&token, &stored));
        assert!(!matches("autre", &stored));
        assert!(!matches(&token, &stored[..16]), "longueur différente");
    }
}
```

</details>


---

## 5. `rateLimit` ✅

**Manque.** Login, 2FA, demande de reset et renvoi de vérification sont limités par clé (e-mail) avec
un `429` et un `Retry-After`. Fenêtre glissante, clés indépendantes.

💡 **Étape proposée**

```json
{ "type": "rateLimit", "id": "l1",
   "key": { "kind": "raw", "code": "input.email.trim().to_lowercase()" },
   "max": 5, "windowSecs": 900 }
```

Dépassement : `429` automatique avec `Retry-After`. Le magasin (mémoire, Valkey, table purgée par le
cleaner) est un paramètre du service. La démo est en mémoire ; en prod, Valkey.

```rust
// src/rate_limit.rs
//! Étape `rateLimit` : fenêtre glissante par clé (ici en mémoire ; en prod,
//! Valkey ou une table purgée par le cleaner).

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub struct RateLimiter {
    max: usize,
    window: Duration,
    hits: Mutex<HashMap<String, VecDeque<Instant>>>,
}

impl RateLimiter {
    pub fn new(max: usize, window: Duration) -> Self {
        Self {
            max,
            window,
            hits: Mutex::new(HashMap::new()),
        }
    }

    /// Enregistre une tentative. `Err(délai)` : combien attendre avant la suivante.
    pub fn check(&self, key: &str, now: Instant) -> Result<(), Duration> {
        let mut hits = self.hits.lock().unwrap_or_else(|e| e.into_inner());
        let queue = hits.entry(key.to_owned()).or_default();
        while queue
            .front()
            .is_some_and(|&t| now.duration_since(t) >= self.window)
        {
            queue.pop_front();
        }
        if queue.len() >= self.max {
            let oldest = queue.front().copied().unwrap_or(now);
            return Err(self.window.saturating_sub(now.duration_since(oldest)));
        }
        queue.push_back(now);
        Ok(())
    }
}
```

<details><summary>Tests de <code>rate_limit.rs</code></summary>

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_after_max_then_reopens() {
        let limiter = RateLimiter::new(3, Duration::from_secs(60));
        let t0 = Instant::now();
        for i in 0..3 {
            assert!(limiter.check("a@b.fr", t0 + Duration::from_secs(i)).is_ok());
        }
        let wait = limiter
            .check("a@b.fr", t0 + Duration::from_secs(10))
            .unwrap_err();
        assert_eq!(wait, Duration::from_secs(50));
        assert!(
            limiter
                .check("a@b.fr", t0 + Duration::from_secs(61))
                .is_ok()
        );
    }

    #[test]
    fn keys_are_independent() {
        let limiter = RateLimiter::new(1, Duration::from_secs(60));
        let t0 = Instant::now();
        assert!(limiter.check("a", t0).is_ok());
        assert!(limiter.check("b", t0).is_ok());
        assert!(limiter.check("a", t0).is_err());
    }
}
```

</details>


---

## 6. Réponse HTTP complète et login assemblé ✅

**Manque.** `respond` n'a ni statut, ni en-têtes, ni cookies. Le front exige un cookie de session
`HttpOnly; Secure; SameSite=Lax`.

💡 **Extension proposée de `respond` pour les routes**

```json
{ "type": "respond", "id": "l6", "status": 200,
   "cookies": [{ "name": "session", "value": { "kind": "path", "path": "token" },
                 "maxAgeSecs": 900, "httpOnly": true, "secure": true, "sameSite": "Lax" }],
   "fields": [{ "field": "account_id", "value": { "kind": "path", "path": "account.id" } }] }
```

**Le login complet** (`POST /v1/auth/login`, aujourd'hui `new_route2 []` dans le projet) devrait
pouvoir s'écrire ainsi :

💡 `validate` → `rateLimit` → `sql select` → `verifyPassword` → `branch` + `fail 401` → `randomToken`
→ `sha256` → `sql insert sessions` → `respond` avec cookie.

La démo ci-dessous est ce que ce flux doit générer. Ses 4 tests couvrent : bon identifiant avec cookie
sécurisé, mauvais mot de passe et e-mail inconnu **indiscernables** (même statut, même corps), e-mail
invalide en 400 avant tout hachage, et la 6ᵉ tentative en 429 avec `Retry-After`.

```rust
// src/login.rs
//! Ce que le builder devrait générer pour `POST /v1/auth/login`, en assemblant
//! les étapes : validation, limitation, vérification du mot de passe (temps
//! constant), session, cookie `HttpOnly`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::http::header::SET_COOKIE;
use axum::response::{AppendHeaders, IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;

use crate::error::{Fail, FailResult};
use crate::rate_limit::RateLimiter;
use crate::{password, tokens, validation};

pub const LOGIN_MAX_ATTEMPTS: usize = 5;
pub const LOGIN_WINDOW: Duration = Duration::from_secs(15 * 60);
pub const SESSION_TTL_SECS: u64 = 15 * 60;

pub struct Account {
    pub id: u32,
    pub password_hash: String,
}

pub struct Demo {
    pub accounts: HashMap<String, Account>,
    pub limiter: RateLimiter,
}

#[derive(Deserialize)]
pub struct LoginBody {
    pub email: String,
    pub password: String,
}

/// Cookie de session : inaccessible au JS, jamais envoyé en cross-site.
pub fn session_cookie(value: &str, max_age_secs: u64) -> String {
    format!("session={value}; Max-Age={max_age_secs}; Path=/; HttpOnly; Secure; SameSite=Lax")
}

pub async fn login(
    State(state): State<Arc<Demo>>,
    Json(body): Json<LoginBody>,
) -> FailResult<Response> {
    validation::email(&body.email)?;
    let email = body.email.trim().to_lowercase();

    state
        .limiter
        .check(&email, Instant::now())
        .map_err(|wait| Fail::TooManyRequests {
            retry_after_secs: wait.as_secs().max(1),
        })?;

    let account = state.accounts.get(&email);
    let stored = account.map(|a| a.password_hash.clone());
    if !password::verify(body.password, stored).await {
        // Même message et même durée que le compte soit inconnu ou non.
        return Err(Fail::Unauthorized);
    }
    let account = account.ok_or(Fail::Unauthorized)?;

    let token = tokens::generate();
    // En prod : INSERT sessions (tokens::hash(&token)), jamais le jeton en clair.
    let cookie = session_cookie(&token, SESSION_TTL_SECS);
    Ok((
        AppendHeaders([(SET_COOKIE, cookie)]),
        Json(json!({ "account_id": account.id })),
    )
        .into_response())
}

pub fn router(state: Arc<Demo>) -> Router {
    Router::new()
        .route("/v1/auth/login", post(login))
        .with_state(state)
}
```

<details><summary>Tests de <code>login.rs</code></summary>

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode, header};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    async fn app() -> Router {
        let hash = password::hash("un mot de passe correct".into())
            .await
            .unwrap();
        let accounts = HashMap::from([(
            "alice@exemple.fr".to_owned(),
            Account {
                id: 1,
                password_hash: hash,
            },
        )]);
        router(Arc::new(Demo {
            accounts,
            limiter: RateLimiter::new(LOGIN_MAX_ATTEMPTS, LOGIN_WINDOW),
        }))
    }

    async fn post_login(app: &Router, email: &str, password: &str) -> Response {
        let req = Request::post("/v1/auth/login")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                json!({ "email": email, "password": password }).to_string(),
            ))
            .unwrap();
        app.clone().oneshot(req).await.unwrap()
    }

    #[tokio::test]
    async fn good_credentials_open_a_session_with_a_secure_cookie() {
        let res = post_login(&app().await, "Alice@Exemple.fr", "un mot de passe correct").await;
        assert_eq!(res.status(), StatusCode::OK);
        let cookie = res.headers()[SET_COOKIE].to_str().unwrap().to_owned();
        assert!(cookie.starts_with("session="));
        for attr in ["HttpOnly", "Secure", "SameSite=Lax", "Max-Age=900"] {
            assert!(cookie.contains(attr), "{attr} manquant : {cookie}");
        }
        let body = res.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap()["account_id"],
            1
        );
    }

    #[tokio::test]
    async fn wrong_password_and_unknown_email_look_identical() {
        let app = app().await;
        let wrong = post_login(&app, "alice@exemple.fr", "mauvais mot de passe").await;
        let unknown = post_login(&app, "inconnu@exemple.fr", "un mot de passe correct").await;
        assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(unknown.status(), StatusCode::UNAUTHORIZED);
        let a = wrong.into_body().collect().await.unwrap().to_bytes();
        let b = unknown.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(a, b, "même corps : pas d'énumération de comptes");
    }

    #[tokio::test]
    async fn invalid_email_is_a_400_before_any_hashing() {
        let res = post_login(&app().await, "pas-un-email", "x").await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn sixth_attempt_is_a_429_with_retry_after() {
        let app = app().await;
        for _ in 0..LOGIN_MAX_ATTEMPTS {
            let res = post_login(&app, "alice@exemple.fr", "mauvais mot de passe").await;
            assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        }
        let res = post_login(&app, "alice@exemple.fr", "un mot de passe correct").await;
        assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
        let retry: u64 = res.headers()[header::RETRY_AFTER]
            .to_str()
            .unwrap()
            .parse()
            .unwrap();
        assert!((1..=LOGIN_WINDOW.as_secs()).contains(&retry));
    }
}
```

</details>


---

## 7. `encrypt` et `decrypt` ✅

**Manque.** Le secret TOTP est chiffré au repos en AES-256-GCM. Le nonce est aléatoire à chaque appel
et préfixe le message ; toute altération, mauvaise clé ou troncature est refusée.

💡 **Étapes proposées**

```json
{ "type": "encrypt", "id": "t2", "key": { "kind": "raw", "code": "state.totp_key" },
   "input": { "kind": "path", "path": "secret" }, "bind": "sealed" }
{ "type": "decrypt", "id": "t3", "key": { "kind": "raw", "code": "state.totp_key" },
   "input": { "kind": "path", "path": "row.secret_enc" }, "bind": "secret" }
```

```rust
// src/crypto_box.rs
//! Étapes `encrypt` / `decrypt` : AES-256-GCM, nonce aléatoire en préfixe
//! (secrets TOTP chiffrés au repos).

use aes_gcm::aead::{Aead, AeadCore, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Nonce};

use crate::error::{Fail, FailResult};

const NONCE_LEN: usize = 12;

pub fn encrypt(key: &[u8; 32], plaintext: &[u8]) -> FailResult<Vec<u8>> {
    let cipher =
        Aes256Gcm::new_from_slice(key).map_err(|_| Fail::Internal("clé AES invalide".into()))?;
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let mut out = nonce.to_vec();
    out.extend(
        cipher
            .encrypt(&nonce, plaintext)
            .map_err(|_| Fail::Internal("chiffrement".into()))?,
    );
    Ok(out)
}

pub fn decrypt(key: &[u8; 32], sealed: &[u8]) -> FailResult<Vec<u8>> {
    if sealed.len() <= NONCE_LEN {
        return Err(Fail::Internal("donnée chiffrée trop courte".into()));
    }
    let (nonce, ciphertext) = sealed.split_at(NONCE_LEN);
    let nonce: [u8; NONCE_LEN] = nonce
        .try_into()
        .map_err(|_| Fail::Internal("nonce invalide".into()))?;
    Aes256Gcm::new_from_slice(key)
        .map_err(|_| Fail::Internal("clé AES invalide".into()))?
        .decrypt(&Nonce::from(nonce), ciphertext)
        .map_err(|_| Fail::Internal("déchiffrement (clé ou donnée invalide)".into()))
}
```

<details><summary>Tests de <code>crypto_box.rs</code></summary>

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; 32] = [7; 32];

    #[test]
    fn roundtrip() {
        let sealed = encrypt(&KEY, b"JBSWY3DPEHPK3PXP").unwrap();
        assert_eq!(decrypt(&KEY, &sealed).unwrap(), b"JBSWY3DPEHPK3PXP");
    }

    #[test]
    fn nonce_makes_each_encryption_different() {
        assert_ne!(encrypt(&KEY, b"x").unwrap(), encrypt(&KEY, b"x").unwrap());
    }

    #[test]
    fn rejects_wrong_key_tampering_and_truncation() {
        let mut sealed = encrypt(&KEY, b"secret").unwrap();
        assert!(decrypt(&[8; 32], &sealed).is_err());
        assert!(decrypt(&KEY, &sealed[..5]).is_err());
        *sealed.last_mut().unwrap() ^= 1;
        assert!(decrypt(&KEY, &sealed).is_err());
    }
}
```

</details>


---

## 8. `verifyHmac` : webhooks entrants ✅

**Manque.** `POST /webhook/stripe` et `POST /connect/webhook` sont hors `/v1`, sans session,
authentifiés par signature (un secret chacune). Le builder doit signer/vérifier sur le **corps brut**,
avec une tolérance d'horloge contre le rejeu, et comparer en temps constant.

💡 **Étape proposée** (route de type `webhook`)

```json
{ "type": "verifyHmac", "id": "w1",
   "header": "Stripe-Signature", "secretEnv": "STRIPE_WEBHOOK_SECRET", "toleranceSecs": 300 }
```

```rust
// src/webhook.rs
//! Webhook entrant signé (HMAC-SHA256), sans session : `t=<ts>,v1=<hex>`
//! calculé sur `"{ts}.{corps}"`, avec une tolérance d'horloge.

use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

use crate::error::{Fail, FailResult};

type HmacSha256 = Hmac<Sha256>;

pub const TOLERANCE_SECS: i64 = 300;

pub fn sign(secret: &[u8], timestamp: i64, body: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret).expect("clé HMAC de toute taille");
    mac.update(timestamp.to_string().as_bytes());
    mac.update(b".");
    mac.update(body);
    format!(
        "t={timestamp},v1={}",
        hex::encode(mac.finalize().into_bytes())
    )
}

pub fn verify(secret: &[u8], header: &str, body: &[u8], now: i64) -> FailResult<()> {
    let invalid = || Fail::Unauthorized;
    let mut timestamp = None;
    let mut signature = None;
    for part in header.split(',') {
        match part.split_once('=') {
            Some(("t", v)) => timestamp = v.parse::<i64>().ok(),
            Some(("v1", v)) => signature = hex::decode(v).ok(),
            _ => {}
        }
    }
    let (timestamp, signature) = timestamp.zip(signature).ok_or_else(invalid)?;
    if (now - timestamp).abs() > TOLERANCE_SECS {
        return Err(invalid());
    }
    let mut mac = HmacSha256::new_from_slice(secret).map_err(|_| invalid())?;
    mac.update(timestamp.to_string().as_bytes());
    mac.update(b".");
    mac.update(body);
    // `verify_slice` compare en temps constant.
    mac.verify_slice(&signature).map_err(|_| invalid())
}
```

<details><summary>Tests de <code>webhook.rs</code></summary>

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &[u8] = b"whsec_test";
    const BODY: &[u8] = br#"{"type":"payment_intent.succeeded"}"#;

    #[test]
    fn accepts_a_valid_signature() {
        let header = sign(SECRET, 1_000, BODY);
        assert!(verify(SECRET, &header, BODY, 1_100).is_ok());
    }

    #[test]
    fn rejects_tampered_body_wrong_secret_and_replay() {
        let header = sign(SECRET, 1_000, BODY);
        assert!(verify(SECRET, &header, b"{}", 1_000).is_err());
        assert!(verify(b"autre", &header, BODY, 1_000).is_err());
        assert!(verify(SECRET, &header, BODY, 1_000 + TOLERANCE_SECS + 1).is_err());
    }

    #[test]
    fn rejects_malformed_headers() {
        for header in ["", "t=1", "v1=zz", "t=abc,v1=00", "n'importe quoi"] {
            assert!(verify(SECRET, header, BODY, 1).is_err(), "{header}");
        }
    }
}
```

</details>


---

## 9. `schedule` : tâches périodiques ✅

**Manque.** Le builder n'a que des routes, des RPC et des consommateurs. Le `cleaner` de `svc-auth`
(purge toutes les 7 minutes des jetons, sessions, challenges 2FA et états WebAuthn expirés) n'est pas
exprimable. Contraintes encodées dans la démo : pas de rafale de rattrapage après un passage lent, une
erreur est journalisée sans tuer la boucle, arrêt propre. Les `DELETE` sont idempotents, plusieurs
instances peuvent donc tourner.

💡 **Proposition** : `Service.jobs`

```json
{ "name": "cleaner", "everySecs": 420,
   "steps": [{ "type": "sql", "id": "c1", "op": "delete", "table": "auth_tokens",
               "where": [{ "column": "expires_at", "op": "le", "value": { "kind": "raw", "code": "now()" } }] }] }
```

```rust
// src/jobs.rs
//! Étape `schedule` : tâche périodique avec arrêt propre (le cleaner).

use std::future::Future;
use std::time::Duration;

use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::{MissedTickBehavior, interval};

/// Lance `job` toutes les `every`, sans rafale de rattrapage si un passage a
/// été long ; s'arrête quand `shutdown` passe à `true`. Une erreur du job est
/// journalisée, jamais fatale.
pub fn spawn_periodic<F, Fut, E>(
    every: Duration,
    mut shutdown: watch::Receiver<bool>,
    job: F,
) -> JoinHandle<()>
where
    F: Fn() -> Fut + Send + 'static,
    Fut: Future<Output = Result<u64, E>> + Send,
    E: std::fmt::Display,
{
    tokio::spawn(async move {
        let mut ticker = interval(every);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = ticker.tick() => match job().await {
                    Ok(rows) => tracing::debug!(rows, "purge terminée"),
                    Err(e) => tracing::warn!(error = %e, "purge en échec"),
                },
                _ = shutdown.changed() => break,
            }
        }
    })
}
```

<details><summary>Tests de <code>jobs.rs</code></summary>

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[tokio::test(start_paused = true)]
    async fn runs_every_interval_and_stops_on_shutdown() {
        let runs = Arc::new(AtomicU32::new(0));
        let (stop, rx) = watch::channel(false);
        let counter = runs.clone();
        let handle = spawn_periodic(Duration::from_secs(60), rx, move || {
            let counter = counter.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok::<u64, String>(0)
            }
        });

        tokio::time::sleep(Duration::from_secs(150)).await;
        // t=0, 60, 120
        assert_eq!(runs.load(Ordering::SeqCst), 3);

        stop.send(true).unwrap();
        handle.await.unwrap();
        tokio::time::sleep(Duration::from_secs(300)).await;
        assert_eq!(
            runs.load(Ordering::SeqCst),
            3,
            "plus aucun passage après l'arrêt"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_failing_job_does_not_kill_the_loop() {
        let runs = Arc::new(AtomicU32::new(0));
        let (stop, rx) = watch::channel(false);
        let counter = runs.clone();
        let handle = spawn_periodic(Duration::from_secs(10), rx, move || {
            let counter = counter.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Err::<u64, String>("base indisponible".into())
            }
        });
        tokio::time::sleep(Duration::from_secs(25)).await;
        assert_eq!(runs.load(Ordering::SeqCst), 3);
        stop.send(true).unwrap();
        handle.await.unwrap();
    }
}
```

</details>


---

## 10. Consommateur idempotent ✅ (logique) 📝 (SQL)

**Manque.** Kafka livre au moins une fois : un `UserRegistered` peut arriver deux fois, et
l'e-mail de bienvenue partirait deux fois. Le builder doit offrir un drapeau sur le consommateur et
générer le dédoublonnage dans la **même transaction** que les effets du handler.

💡 **Proposition** : `Consumer.idempotent: true`

📝 SQL (non exécuté)

```sql
CREATE TABLE processed_events (
    consumer     text        NOT NULL,
    event_id     uuid        NOT NULL,
    processed_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (consumer, event_id)
);

-- Dans la transaction du handler ; rows_affected() = 1 → première livraison.
INSERT INTO processed_events (consumer, event_id)
VALUES ($1, $2)
ON CONFLICT DO NOTHING;
```

```rust
// src/consumer.rs
//! Consommateur idempotent : un événement livré deux fois (at-least-once) n'est
//! traité qu'une fois. En prod, `Dedup` = table `processed_events(event_id PK)`
//! et `first_time` = `INSERT ... ON CONFLICT DO NOTHING`, `rows_affected() == 1`
//! (voir le SQL dans le document).

use std::collections::HashSet;
use std::sync::Mutex;

pub trait Dedup {
    /// `true` la première fois qu'on voit cet `event_id`.
    fn first_time(&self, event_id: &str) -> bool;
}

#[derive(Default)]
pub struct MemoryDedup(Mutex<HashSet<String>>);

impl Dedup for MemoryDedup {
    fn first_time(&self, event_id: &str) -> bool {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(event_id.to_owned())
    }
}

/// Exécute `handler` seulement si l'événement est nouveau.
/// Renvoie `true` s'il a été traité.
pub async fn handle_once<D, F, Fut>(dedup: &D, event_id: &str, handler: F) -> bool
where
    D: Dedup,
    F: FnOnce() -> Fut,
    Fut: Future<Output = ()>,
{
    if !dedup.first_time(event_id) {
        return false;
    }
    handler().await;
    true
}
```

<details><summary>Tests de <code>consumer.rs</code></summary>

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[tokio::test]
    async fn processes_each_event_id_once() {
        let dedup = MemoryDedup::default();
        let done = AtomicU32::new(0);
        for id in ["e1", "e1", "e2", "e1"] {
            handle_once(&dedup, id, || async {
                done.fetch_add(1, Ordering::SeqCst);
            })
            .await;
        }
        assert_eq!(done.load(Ordering::SeqCst), 2);
    }
}
```

</details>


---

## 11. Jeton à usage unique, consommation atomique 📝

**Manque.** Vérification d'e-mail, reset de mot de passe, changement d'e-mail et invitation consomment
un jeton une seule fois. Faire `SELECT` puis `UPDATE` laisse une fenêtre de rejeu : deux requêtes
simultanées passent toutes les deux. Il faut **une seule instruction** (`UPDATE ... RETURNING`) qui
vérifie l'état, l'expiration et marque `used`.

💡 Le `sql` du builder doit supporter `RETURNING` (liaison du résultat) et un `where` avec `used = false`.

📝 SQL (non exécuté)

```sql
-- Aucune ligne renvoyée : jeton inconnu, expiré ou déjà utilisé → fail 400.
UPDATE auth_tokens
   SET used = true
 WHERE token_hash = $1
   AND kind       = $2
   AND used       = false
   AND expires_at > now()
RETURNING account_id, new_email;
```

Test à écrire avec `#[sqlx::test]` : lancer deux consommations concurrentes du même jeton
(`tokio::join!`), vérifier qu'exactement **une** renvoie une ligne.

---

## 12. Données multi-tenant, soft delete, outbox 📝

**Manque.** Règles de `.claude/rules/clairtyx.md` que le builder doit appliquer par défaut.

💡 **Propositions**

- Table métier : `tenant_id` ajouté et filtré dans **tous** les `sql`, sans que l'auteur l'écrive.
- `Table.softDelete: true` : `select` ajoute `deleted_at IS NULL`, `delete` devient `UPDATE`.
- `Table.references` : FK composite `(tenant_id, id)`.
- `publish` : toujours via l'outbox, dans la transaction, jamais directement sur Kafka.

📝 SQL (non exécuté)

```sql
-- Soft delete : jamais de DELETE sur une donnée partagée entre membres.
UPDATE clients SET deleted_at = now()
 WHERE tenant_id = $1 AND client_id = $2 AND deleted_at IS NULL;

-- FK composite : une facture ne peut pointer que vers un client du même tenant.
ALTER TABLE clients  ADD CONSTRAINT clients_tenant_id_uq UNIQUE (tenant_id, client_id);
ALTER TABLE invoices ADD CONSTRAINT invoices_client_fk
    FOREIGN KEY (tenant_id, client_id) REFERENCES clients (tenant_id, client_id);

-- Relais de l'outbox : plusieurs instances sans doublon grâce à SKIP LOCKED.
SELECT id, topic, payload FROM outbox
 WHERE published_at IS NULL
 ORDER BY id
 LIMIT 100
 FOR UPDATE SKIP LOCKED;
```

---

## 13. JWT et JWKS ✅

**Manque.** Le builder n'a ni signature ni vérification de jeton. `svc-auth` émet un access token **Ed25519
(EdDSA) de 5 minutes**, vérifiable sans base ; les autres services (realtime) le valident avec la clé publique
publiée sur `GET /.well-known/jwks.json`, sans jamais voir la graine. Règles à générer une fois pour toutes :

- l'algorithme est **imposé** (EdDSA) : ni `none`, ni HS256 signé avec la clé publique ;
- émetteur, `exp` (tolérance 5 s) et claims obligatoires vérifiés ; toute erreur donne le même 401 ;
- le `kid` (dérivé de la clé publique) choisit la clé de vérification, ce qui permet la **rotation** : l'ancienne
  clé reste publiée 5 minutes, le temps que ses jetons expirent ;
- le JWKS ne contient jamais de paramètre privé (`d`).

💡 **Étapes proposées**

```json
{ "type": "signJwt", "id": "j1", "claims": [
    { "field": "sub", "value": { "kind": "path", "path": "account.id" } },
    { "field": "tid", "value": { "kind": "path", "path": "tenant.id" } },
    { "field": "roles", "value": { "kind": "path", "path": "grants.roles" } }],
  "ttlSecs": 300, "bind": "access_token" }
{ "type": "verifyJwt", "id": "j2", "token": { "kind": "path", "path": "bearer" }, "jwks": "auth", "bind": "claims" }
```

et une route `{ "kind": "jwks" }` qui publie la clé du service. Le projet Service Builder a déjà le bon
découpage : l'émetteur (`JwtKeys`) porte la clé privée, le consommateur (`JwksVerifier`) ne connaît que le JWKS.

```rust
// src/jwt.rs
//! Étapes `signJwt` / `verifyJwt` et route `GET /.well-known/jwks.json` :
//! access token Ed25519 (EdDSA) de 5 minutes, vérifiable sans base, avec
//! publication de la clé publique (JWKS) pour les services qui le valident seuls
//! (realtime) et prise en charge de la rotation par `kid`.

use std::collections::HashMap;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{Duration, Utc};
use ed25519_dalek::SigningKey;
use jsonwebtoken::{
    Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, decode_header, encode,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::error::{Fail, FailResult};

/// Durée de vie d'un access token ; c'est aussi le délai maximal pendant lequel
/// un jeton reste accepté après une révocation.
pub const ACCESS_TTL: Duration = Duration::minutes(5);
pub const ISSUER: &str = "clairtyx";
/// Tolérance sur `exp` face aux horloges légèrement décalées.
const LEEWAY_SECS: u64 = 5;

/// Préfixe PKCS#8 d'une clé privée Ed25519 (RFC 8410), suivi des 32 octets de graine.
const PKCS8_ED25519_PREFIX: [u8; 16] = [
    0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20,
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessClaims {
    pub iss: String,
    /// Compte
    pub sub: Uuid,
    /// Entreprise active
    pub tid: Uuid,
    /// Session : permet de révoquer côté serveur
    pub sid: Uuid,
    pub iat: i64,
    pub exp: i64,
    #[serde(default)]
    pub roles: Vec<String>,
    #[serde(default)]
    pub permissions: Vec<String>,
}

impl AccessClaims {
    pub fn new(
        tenant: Uuid,
        account: Uuid,
        session: Uuid,
        roles: &[&str],
        permissions: &[&str],
    ) -> Self {
        let now = Utc::now();
        Self {
            iss: ISSUER.to_owned(),
            sub: account,
            tid: tenant,
            sid: session,
            iat: now.timestamp(),
            exp: (now + ACCESS_TTL).timestamp(),
            roles: roles.iter().map(|r| (*r).to_owned()).collect(),
            permissions: permissions.iter().map(|p| (*p).to_owned()).collect(),
        }
    }
}

/// Identifiant de clé dérivé de la clé publique : prépare la rotation.
fn kid_of(public: &[u8; 32]) -> String {
    URL_SAFE_NO_PAD.encode(&Sha256::digest(public)[..8])
}

/// Côté émetteur : porte la clé privée.
pub struct JwtKeys {
    encoding: EncodingKey,
    public: [u8; 32],
    kid: String,
}

impl JwtKeys {
    pub fn from_seed(seed: &[u8; 32]) -> Self {
        let public = SigningKey::from_bytes(seed).verifying_key().to_bytes();
        let mut pkcs8 = PKCS8_ED25519_PREFIX.to_vec();
        pkcs8.extend_from_slice(seed);
        Self {
            encoding: EncodingKey::from_ed_der(&pkcs8),
            kid: kid_of(&public),
            public,
        }
    }

    /// JWKS (RFC 7517) : uniquement la clé publique, qui ne permet pas de forger.
    pub fn jwks(&self) -> serde_json::Value {
        serde_json::json!({ "keys": [self.jwk()] })
    }

    fn jwk(&self) -> serde_json::Value {
        serde_json::json!({
            "kty": "OKP", "crv": "Ed25519", "use": "sig", "alg": "EdDSA",
            "kid": self.kid, "x": URL_SAFE_NO_PAD.encode(self.public),
        })
    }

    pub fn sign(&self, claims: &AccessClaims) -> FailResult<String> {
        let mut header = Header::new(Algorithm::EdDSA);
        header.kid = Some(self.kid.clone());
        encode(&header, claims, &self.encoding)
            .map_err(|e| Fail::Internal(format!("signature du JWT: {e}")))
    }
}

/// Côté consommateur : ne connaît que le JWKS, jamais la graine.
pub struct JwksVerifier {
    keys: HashMap<String, DecodingKey>,
}

impl JwksVerifier {
    /// Lit un JWKS ; ignore les clés qui ne sont pas `OKP`/`Ed25519`.
    pub fn from_jwks(jwks: &serde_json::Value) -> FailResult<Self> {
        let mut keys = HashMap::new();
        for jwk in jwks["keys"].as_array().into_iter().flatten() {
            if jwk["kty"] != "OKP" || jwk["crv"] != "Ed25519" {
                continue;
            }
            let (Some(kid), Some(x)) = (jwk["kid"].as_str(), jwk["x"].as_str()) else {
                continue;
            };
            let public = URL_SAFE_NO_PAD
                .decode(x)
                .map_err(|_| Fail::Internal("JWKS : paramètre x illisible".into()))?;
            keys.insert(kid.to_owned(), DecodingKey::from_ed_der(&public));
        }
        if keys.is_empty() {
            return Err(Fail::Internal("JWKS sans clé Ed25519 exploitable".into()));
        }
        Ok(Self { keys })
    }

    /// Vérifie signature, algorithme (EdDSA uniquement : pas de confusion
    /// d'algorithme), émetteur et expiration. Toute erreur donne le même 401.
    pub fn verify(&self, token: &str) -> FailResult<AccessClaims> {
        let header = decode_header(token).map_err(|_| Fail::Unauthorized)?;
        let key = header
            .kid
            .as_deref()
            .and_then(|kid| self.keys.get(kid))
            .ok_or(Fail::Unauthorized)?;

        let mut validation = Validation::new(Algorithm::EdDSA);
        validation.leeway = LEEWAY_SECS;
        validation.set_issuer(&[ISSUER]);
        validation.set_required_spec_claims(&["exp", "iss", "sub"]);
        decode::<AccessClaims>(token, key, &validation)
            .map(|data| data.claims)
            .map_err(|_| Fail::Unauthorized)
    }
}

/// Fusionne les JWKS de plusieurs clés : pendant une rotation, l'ancienne clé
/// reste publiée tant que ses jetons (5 min) peuvent circuler.
pub fn merge_jwks(keys: &[&JwtKeys]) -> serde_json::Value {
    serde_json::json!({ "keys": keys.iter().map(|k| k.jwk()).collect::<Vec<_>>() })
}
```

<details><summary>Tests de <code>jwt.rs</code></summary>

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::EncodingKey as Hs;

    fn claims() -> AccessClaims {
        AccessClaims::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            &["staff"],
            &["clients.view"],
        )
    }

    fn verifier(keys: &JwtKeys) -> JwksVerifier {
        JwksVerifier::from_jwks(&keys.jwks()).unwrap()
    }

    #[test]
    fn roundtrip_through_the_published_jwks_only() {
        let keys = JwtKeys::from_seed(&[1; 32]);
        let claims = claims();
        let token = keys.sign(&claims).unwrap();
        // Le vérificateur n'a jamais vu la graine, seulement le JWKS.
        let got = verifier(&keys).verify(&token).unwrap();
        assert_eq!(
            (got.tid, got.sub, got.sid),
            (claims.tid, claims.sub, claims.sid)
        );
        assert_eq!(got.roles, ["staff"]);
        assert_eq!(got.permissions, ["clients.view"]);
        assert_eq!(got.exp - got.iat, ACCESS_TTL.num_seconds());
    }

    #[test]
    fn jwks_has_no_private_parameter_and_matches_the_token_kid() {
        let keys = JwtKeys::from_seed(&[7; 32]);
        let jwks = keys.jwks();
        let key = &jwks["keys"][0];
        assert_eq!(
            (
                key["kty"].as_str(),
                key["crv"].as_str(),
                key["alg"].as_str()
            ),
            (Some("OKP"), Some("Ed25519"), Some("EdDSA"))
        );
        assert!(key.get("d").is_none(), "jamais de paramètre privé");
        let public = SigningKey::from_bytes(&[7; 32]).verifying_key().to_bytes();
        assert_eq!(
            key["x"].as_str(),
            Some(URL_SAFE_NO_PAD.encode(public).as_str())
        );
        let header = decode_header(keys.sign(&claims()).unwrap()).unwrap();
        assert_eq!(header.kid.as_deref(), key["kid"].as_str());
    }

    #[test]
    fn token_signed_by_another_key_is_refused() {
        let forged = JwtKeys::from_seed(&[2; 32]).sign(&claims()).unwrap();
        assert!(matches!(
            verifier(&JwtKeys::from_seed(&[1; 32])).verify(&forged),
            Err(Fail::Unauthorized)
        ));
    }

    #[test]
    fn expired_wrong_issuer_and_tampered_tokens_are_refused() {
        let keys = JwtKeys::from_seed(&[1; 32]);
        let verifier = verifier(&keys);

        let mut expired = claims();
        expired.exp = (Utc::now() - Duration::minutes(1)).timestamp();
        assert!(verifier.verify(&keys.sign(&expired).unwrap()).is_err());

        let mut other_issuer = claims();
        other_issuer.iss = "autre".into();
        assert!(verifier.verify(&keys.sign(&other_issuer).unwrap()).is_err());

        // Charge utile d'une autre entreprise, signature d'origine.
        let token = keys.sign(&claims()).unwrap();
        let mut parts: Vec<String> = token.split('.').map(str::to_owned).collect();
        parts[1] = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims()).unwrap());
        assert!(verifier.verify(&parts.join(".")).is_err());
    }

    #[test]
    fn none_and_hmac_algorithms_are_refused() {
        let keys = JwtKeys::from_seed(&[1; 32]);
        let verifier = verifier(&keys);
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"none","typ":"JWT","kid":"x"}"#);
        let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims()).unwrap());
        assert!(verifier.verify(&format!("{header}.{payload}.")).is_err());

        // HS256 signé avec la clé publique comme secret (attaque classique),
        // en reprenant le bon `kid` pour atteindre la vérification.
        let mut hs_header = Header::new(Algorithm::HS256);
        hs_header.kid = keys.jwk()["kid"].as_str().map(str::to_owned);
        let hs = encode(&hs_header, &claims(), &Hs::from_secret(&keys.public)).unwrap();
        assert!(verifier.verify(&hs).is_err());
    }

    #[test]
    fn unknown_kid_and_garbage_are_refused() {
        let token = JwtKeys::from_seed(&[2; 32]).sign(&claims()).unwrap();
        let verifier = verifier(&JwtKeys::from_seed(&[1; 32]));
        assert!(verifier.verify(&token).is_err(), "kid inconnu");
        for garbage in ["", "a.b.c", "pas un jwt"] {
            assert!(verifier.verify(garbage).is_err(), "{garbage}");
        }
    }

    #[test]
    fn rotation_keeps_old_tokens_valid_until_the_old_key_is_removed() {
        let old = JwtKeys::from_seed(&[1; 32]);
        let new = JwtKeys::from_seed(&[2; 32]);
        let old_token = old.sign(&claims()).unwrap();
        let new_token = new.sign(&claims()).unwrap();

        let during = JwksVerifier::from_jwks(&merge_jwks(&[&new, &old])).unwrap();
        assert!(during.verify(&old_token).is_ok());
        assert!(during.verify(&new_token).is_ok());

        // Plus de jeton de l'ancienne clé en circulation (5 min) : on la retire.
        let after = JwksVerifier::from_jwks(&new.jwks()).unwrap();
        assert!(after.verify(&old_token).is_err());
        assert!(after.verify(&new_token).is_ok());
    }

    #[test]
    fn empty_or_non_ed25519_jwks_is_rejected_at_load() {
        assert!(JwksVerifier::from_jwks(&serde_json::json!({ "keys": [] })).is_err());
        let rsa =
            serde_json::json!({ "keys": [{ "kty": "RSA", "kid": "a", "n": "x", "e": "AQAB" }] });
        assert!(JwksVerifier::from_jwks(&rsa).is_err());
    }
}
```

</details>


---

## 14. TOTP, anti-rejeu et codes de récupération ✅ (logique) 📝 (SQL)

**Manque.** Le 2FA TOTP (RFC 6238) réunit cinq règles que le builder doit connaître :

1. le secret est **déchiffrable** (il faut pouvoir vérifier un code), donc chiffré au repos (§7), jamais haché ;
2. l'URI `otpauth://` est renvoyée au client, qui fabrique le QR code ; le secret base32 n'existe qu'à ce moment ;
3. la vérification accepte ±1 pas de 30 s et compare **tous** les pas en temps constant ;
4. **anti-rejeu** : un code n'est accepté que pour un pas strictement plus récent que le dernier accepté ;
5. les codes de récupération (`XXXXX-XXXXX`) sont à usage unique, stockés hachés, saisis avec ou sans tiret.

La limite de tentatives (5 par 15 min, §5) **réserve la tentative avant** de vérifier et la **libère en cas de
succès** : un échec reste donc compté même si la requête plante en route.

💡 **Étapes proposées**

```json
{ "type": "totpGenerate", "id": "t1", "keyEnv": "TOTP_ENCRYPTION_KEY", "account": { "kind": "path", "path": "input.email" }, "bind": "totp" }
{ "type": "totpVerify", "id": "t2", "keyEnv": "TOTP_ENCRYPTION_KEY", "sealed": { "kind": "path", "path": "account.totp_secret_encrypted" },
  "code": { "kind": "path", "path": "input.code" }, "bind": "step" }
{ "type": "recoveryCodes", "id": "t3", "count": 8, "bind": "codes" }
```

📝 SQL (non exécuté) : l'anti-rejeu et les codes de récupération sont **une seule instruction atomique**,
comme les jetons du §11.

```sql
-- Anti-rejeu : rows_affected() > 0 → accepté.
UPDATE accounts SET totp_last_used_step = $1
 WHERE id = $2 AND (totp_last_used_step IS NULL OR totp_last_used_step < $1);

-- Code de récupération à usage unique : rows_affected() > 0 → valable, et consommé.
UPDATE recovery_codes SET used_at = now()
 WHERE account_id = $1 AND code_hash = $2 AND used_at IS NULL;
```

```rust
// src/totp.rs
//! 2FA par TOTP (RFC 6238) : secret chiffré au repos, vérification avec
//! tolérance, anti-rejeu, codes de récupération à usage unique.

use std::collections::HashSet;
use std::sync::Mutex;

use subtle::ConstantTimeEq;
use totp_rs::{Algorithm, Secret, TOTP};

use crate::crypto_box;
use crate::error::{Fail, FailResult};
use crate::tokens;

const ISSUER: &str = "Clairtyx";
const DIGITS: usize = 6;
/// Pas de temps (30 s) de part et d'autre du pas courant encore acceptés.
const SKEW: u8 = 1;
pub const STEP_SECS: u64 = 30;

pub struct NewTotp {
    /// Secret chiffré (nonce inclus), à stocker dans `accounts.totp_secret_encrypted`.
    pub encrypted_secret: Vec<u8>,
    /// URI `otpauth://` : le QR code est généré côté client.
    pub otpauth_url: String,
    /// Secret en base32 pour la saisie manuelle ; jamais stocké en clair.
    pub secret_base32: String,
}

fn build(secret: Vec<u8>, account: Option<String>) -> FailResult<TOTP> {
    TOTP::new(
        Algorithm::SHA1,
        DIGITS,
        SKEW,
        STEP_SECS,
        secret,
        Some(ISSUER.to_owned()),
        account.unwrap_or_default(),
    )
    .map_err(|e| Fail::Internal(format!("secret TOTP invalide: {e}")))
}

pub fn generate(key: &[u8; 32], email: &str) -> FailResult<NewTotp> {
    let secret = Secret::generate_secret()
        .to_bytes()
        .map_err(|_| Fail::Internal("génération du secret TOTP échouée".into()))?;
    let totp = build(secret.clone(), Some(email.to_owned()))?;
    Ok(NewTotp {
        otpauth_url: totp.get_url(),
        secret_base32: totp.get_secret_base32(),
        encrypted_secret: crypto_box::encrypt(key, &secret)?,
    })
}

/// Pas le plus récent de la fenêtre dont le code correspond. Toutes les
/// comparaisons sont faites (temps constant), sans arrêt au premier trouvé.
fn matching_step(totp: &TOTP, code: &str, now: u64) -> Option<i64> {
    let current = now / STEP_SECS;
    let mut matched = None;
    for step in current.saturating_sub(SKEW as u64)..=current + SKEW as u64 {
        let expected = totp.generate(step * STEP_SECS);
        if bool::from(expected.as_bytes().ct_eq(code.as_bytes())) {
            matched = Some(step as i64);
        }
    }
    matched
}

/// Vérifie un code contre le secret chiffré ; renvoie le pas auquel il
/// correspond, que l'appelant passe à l'anti-rejeu. `now` : secondes Unix.
pub fn verify_code(
    key: &[u8; 32],
    encrypted_secret: &[u8],
    code: &str,
    now: u64,
) -> FailResult<Option<i64>> {
    let secret = crypto_box::decrypt(key, encrypted_secret)?;
    Ok(matching_step(&build(secret, None)?, code, now))
}

/// Anti-rejeu : un pas n'est accepté que s'il est strictement plus récent que
/// le dernier accepté. En base, c'est une seule requête atomique :
///
/// ```sql
/// UPDATE accounts SET totp_last_used_step = $1
///  WHERE id = $2 AND (totp_last_used_step IS NULL OR totp_last_used_step < $1)
/// -- rows_affected() > 0 → accepté
/// ```
pub fn is_newer_step(last_used: Option<i64>, step: i64) -> bool {
    last_used.is_none_or(|last| last < step)
}

/// Un code de récupération : forme lisible (affichée une fois) et empreinte stockée.
pub struct RecoveryCode {
    pub display: String,
    pub hash: [u8; 32],
}

/// `count` codes `XXXXX-XXXXX` (40 bits chacun, suffisant avec la limite de
/// tentatives TOTP : 5 par 15 min).
pub fn generate_recovery_codes(count: usize) -> Vec<RecoveryCode> {
    (0..count)
        .map(|_| {
            let mut bytes = [0u8; 5];
            rand::fill(&mut bytes);
            let canonical = hex::encode(bytes).to_uppercase();
            RecoveryCode {
                display: format!("{}-{}", &canonical[0..5], &canonical[5..10]),
                hash: tokens::hash(&canonical),
            }
        })
        .collect()
}

/// Forme canonique d'un code saisi (sans tiret, en majuscules).
pub fn normalize_recovery_code(input: &str) -> String {
    input
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_uppercase()
}

/// Magasin en mémoire des codes consommés, pour montrer l'usage unique. En base :
///
/// ```sql
/// UPDATE recovery_codes SET used_at = now()
///  WHERE account_id = $1 AND code_hash = $2 AND used_at IS NULL
/// -- rows_affected() > 0 → code valable, et désormais consommé
/// ```
#[derive(Default)]
pub struct RecoveryStore {
    unused: Mutex<HashSet<[u8; 32]>>,
}

impl RecoveryStore {
    pub fn with_codes(codes: &[RecoveryCode]) -> Self {
        Self {
            unused: Mutex::new(codes.iter().map(|c| c.hash).collect()),
        }
    }

    pub fn consume(&self, input: &str) -> bool {
        let hash = tokens::hash(&normalize_recovery_code(input));
        self.unused
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&hash)
    }
}
```

<details><summary>Tests de <code>totp.rs</code></summary>

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; 32] = [7; 32];
    const NOW: u64 = 1_700_000_000;

    /// Code que produirait l'application d'authentification à l'instant `at`.
    fn code_at(new: &NewTotp, at: u64) -> String {
        let secret = crypto_box::decrypt(&KEY, &new.encrypted_secret).unwrap();
        build(secret, None).unwrap().generate(at)
    }

    #[test]
    fn otpauth_url_and_manual_secret_are_consistent() {
        let new = generate(&KEY, "jean@dupont.fr").unwrap();
        assert!(
            new.otpauth_url
                .starts_with("otpauth://totp/Clairtyx:jean%40dupont.fr?")
        );
        assert!(new.otpauth_url.contains("issuer=Clairtyx"));
        assert!(
            new.otpauth_url
                .contains(&format!("secret={}", new.secret_base32))
        );
        assert_ne!(
            new.encrypted_secret,
            new.secret_base32.as_bytes(),
            "jamais en clair"
        );
    }

    #[test]
    fn secret_is_only_readable_with_the_right_key() {
        let new = generate(&KEY, "a@b.fr").unwrap();
        assert!(verify_code(&[8; 32], &new.encrypted_secret, "000000", NOW).is_err());
    }

    #[test]
    fn accepts_the_skew_window_and_nothing_else() {
        let new = generate(&KEY, "a@b.fr").unwrap();
        let current = (NOW / STEP_SECS) as i64;
        for offset in -1..=1i64 {
            let code = code_at(&new, ((current + offset) as u64) * STEP_SECS);
            assert_eq!(
                verify_code(&KEY, &new.encrypted_secret, &code, NOW).unwrap(),
                Some(current + offset)
            );
        }
        for offset in [-2i64, 2] {
            let code = code_at(&new, ((current + offset) as u64) * STEP_SECS);
            let in_window =
                (-1..=1i64).any(|o| code_at(&new, ((current + o) as u64) * STEP_SECS) == code);
            if !in_window {
                assert_eq!(
                    verify_code(&KEY, &new.encrypted_secret, &code, NOW).unwrap(),
                    None,
                    "offset {offset}"
                );
            }
        }
        assert_eq!(
            verify_code(&KEY, &new.encrypted_secret, "abcdef", NOW).unwrap(),
            None
        );
        assert_eq!(
            verify_code(&KEY, &new.encrypted_secret, "", NOW).unwrap(),
            None
        );
    }

    #[test]
    fn replay_is_refused_but_a_newer_code_passes() {
        let new = generate(&KEY, "a@b.fr").unwrap();
        let code = code_at(&new, NOW);
        let step = verify_code(&KEY, &new.encrypted_secret, &code, NOW)
            .unwrap()
            .unwrap();

        let mut last_used = None;
        assert!(is_newer_step(last_used, step), "première utilisation");
        last_used = Some(step);
        assert!(!is_newer_step(last_used, step), "même code rejoué");
        assert!(
            !is_newer_step(last_used, step - 1),
            "code plus ancien que le dernier utilisé"
        );
        assert!(is_newer_step(last_used, step + 1), "code du pas suivant");
    }

    #[test]
    fn recovery_codes_are_unique_readable_and_single_use() {
        let codes = generate_recovery_codes(8);
        let displays: HashSet<_> = codes.iter().map(|c| &c.display).collect();
        assert_eq!(displays.len(), 8);
        assert!(
            codes
                .iter()
                .all(|c| c.display.len() == 11 && c.display.as_bytes()[5] == b'-')
        );

        let store = RecoveryStore::with_codes(&codes);
        // Saisie avec bruit : minuscules, espaces, sans tiret.
        let typed = format!(" {} ", codes[0].display.to_lowercase().replace('-', ""));
        assert!(store.consume(&typed));
        assert!(!store.consume(&codes[0].display), "déjà consommé");
        assert!(
            store.consume(&codes[1].display),
            "les autres restent valables"
        );
        assert!(!store.consume("ZZZZZ-ZZZZZ"));
    }
}
```

</details>


---

## 15. WebAuthn (passkeys) ✅

**Manque.** Une passkey se fait en **deux temps** : `start` (le serveur émet un challenge et un état) puis
`finish` (le navigateur répond). L'état doit survivre entre les deux requêtes : il est sérialisé en JSON et
stocké en base avec une expiration, puis consommé en une instruction (`DELETE ... RETURNING`, comme au §11),
ce qui interdit de terminer deux fois le même `start`. Le domaine (RP ID) et l'origine viennent de `APP_URL`, sans
configuration dédiée.

Ces démos tournent contre un **authentificateur logiciel** (`SoftToken`) : de vraies signatures, aucun navigateur.
Elles couvrent : l'enregistrement puis la connexion avec l'état passé par JSON, le rejeu d'une réponse contre un
autre challenge, une réponse produite pour une autre origine (refusée par l'appareil, et, pour un sous-domaine
compromis de même RP ID, refusée par le serveur), et l'exclusion d'une clé déjà enregistrée.

> **Non couvert ici** : la connexion *découvrable* sans e-mail (`start_discoverable_authentication` +
> `identify_discoverable_authentication`) utilisée par `svc-auth`, qui demande un authentificateur à clé résidente,
> et la mise à jour du compteur de signature. À tester avec un vrai navigateur ou un jeton CTAP2.

💡 **Étapes proposées**

```json
{ "type": "passkeyStart", "id": "w1", "mode": "register", "user": { "kind": "path", "path": "account.id" },
  "excludeExisting": true, "stateTable": "webauthn_registration_states", "bind": "challenge" }
{ "type": "passkeyFinish", "id": "w2", "mode": "register", "stateId": { "kind": "path", "path": "input.state_id" },
  "credential": { "kind": "path", "path": "input.credential" }, "store": "webauthn_credentials" }
```

📝 Schéma (non exécuté) : `credential_id` **unique globalement** (une même clé physique ne peut pas être sur deux
comptes) ; une table d'états par mode, avec `expires_at`, purgée par le job du §9.

```rust
// src/webauthn.rs
//! Passkeys (WebAuthn) : construction de l'instance depuis `APP_URL`, et flux
//! d'enregistrement et de connexion en deux temps. L'état de chaque étape est
//! sérialisé en JSON pour être stocké en base entre le `start` et le `finish`.

use webauthn_rs::prelude::{Url, Webauthn, WebauthnBuilder};

use crate::error::{Fail, FailResult};

/// Durée de vie d'un état `start` non terminé (le cleaner purge au-delà).
pub const STATE_TTL_MINUTES: i64 = 5;

/// Domaine (RP ID) et origine viennent de `APP_URL` : pas de configuration
/// dédiée, donc pas de désynchronisation avec les liens des e-mails.
pub fn build_webauthn(app_url: &str, app_name: &str) -> FailResult<Webauthn> {
    let origin =
        Url::parse(app_url).map_err(|e| Fail::Internal(format!("APP_URL invalide: {e}")))?;
    let rp_id = origin
        .host_str()
        .ok_or_else(|| Fail::Internal("APP_URL sans hôte : RP ID WebAuthn impossible".into()))?;
    WebauthnBuilder::new(rp_id, &origin)
        .map_err(|e| Fail::Internal(format!("WebAuthn: {e}")))?
        .rp_name(app_name)
        .build()
        .map_err(|e| Fail::Internal(format!("WebAuthn: {e}")))
}
```

<details><summary>Tests de <code>webauthn.rs</code></summary>

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;
    use webauthn_authenticator_rs::{WebauthnAuthenticator, softtoken::SoftToken};
    use webauthn_rs::prelude::{PasskeyAuthentication, PasskeyRegistration};

    const APP_URL: &str = "https://app.clairtyx.com";

    fn authenticator() -> WebauthnAuthenticator<SoftToken> {
        let (soft, _) = SoftToken::new(true).expect("soft token");
        WebauthnAuthenticator::new(soft)
    }

    fn origin() -> Url {
        Url::parse(APP_URL).unwrap()
    }

    #[test]
    fn rp_id_is_derived_from_app_url() {
        assert!(build_webauthn(APP_URL, "Clairtyx").is_ok());
        assert!(build_webauthn("pas une url", "Clairtyx").is_err());
        assert!(
            build_webauthn("data:text/plain,x", "Clairtyx").is_err(),
            "pas d'hôte"
        );
    }

    #[test]
    fn register_then_log_in_with_state_stored_as_json() {
        let webauthn = build_webauthn(APP_URL, "Clairtyx").unwrap();
        let mut device = authenticator();
        let account = Uuid::new_v4();

        // ── Enregistrement : start → (navigateur) → finish ──
        let (challenge, state) = webauthn
            .start_passkey_registration(account, "jean@dupont.fr", "Jean", None)
            .unwrap();
        let state_json = serde_json::to_string(&state).unwrap(); // INSERT ... state
        let response = device.do_registration(origin(), challenge).unwrap();
        let state: PasskeyRegistration = serde_json::from_str(&state_json).unwrap(); // SELECT
        let passkey = webauthn
            .finish_passkey_registration(&response, &state)
            .unwrap();

        // ── Connexion ──
        let (challenge, state) = webauthn
            .start_passkey_authentication(std::slice::from_ref(&passkey))
            .unwrap();
        let state_json = serde_json::to_string(&state).unwrap();
        let assertion = device.do_authentication(origin(), challenge).unwrap();
        let state: PasskeyAuthentication = serde_json::from_str(&state_json).unwrap();
        let result = webauthn
            .finish_passkey_authentication(&assertion, &state)
            .unwrap();
        assert_eq!(result.cred_id(), passkey.cred_id());
    }

    #[test]
    fn an_assertion_cannot_be_replayed_against_another_challenge() {
        let webauthn = build_webauthn(APP_URL, "Clairtyx").unwrap();
        let mut device = authenticator();
        let (challenge, state) = webauthn
            .start_passkey_registration(Uuid::new_v4(), "a@b.fr", "A", None)
            .unwrap();
        let passkey = webauthn
            .finish_passkey_registration(
                &device.do_registration(origin(), challenge).unwrap(),
                &state,
            )
            .unwrap();

        let (first, _first_state) = webauthn
            .start_passkey_authentication(std::slice::from_ref(&passkey))
            .unwrap();
        let old_assertion = device.do_authentication(origin(), first).unwrap();

        // Nouvelle tentative = nouveau challenge : l'ancienne réponse ne vaut plus rien.
        let (_second, second_state) = webauthn.start_passkey_authentication(&[passkey]).unwrap();
        assert!(
            webauthn
                .finish_passkey_authentication(&old_assertion, &second_state)
                .is_err()
        );
    }

    #[test]
    fn a_response_made_for_another_origin_is_refused() {
        let user = Uuid::new_v4();
        let mut device = authenticator();

        // 1. Hameçonnage classique : l'authentificateur lui-même refuse de signer
        //    pour une origine qui n'est pas celle du RP ID du challenge.
        let webauthn = build_webauthn(APP_URL, "Clairtyx").unwrap();
        let (challenge, _) = webauthn
            .start_passkey_registration(user, "a@b.fr", "A", None)
            .unwrap();
        let phishing = Url::parse("https://app.clairtyx.evil.example").unwrap();
        assert!(device.do_registration(phishing, challenge).is_err());

        // 2. Même RP ID mais autre origine (sous-domaine compromis) : l'appareil
        //    signe, c'est le serveur qui refuse car `clientData.origin` ne correspond pas.
        let rp_id = "clairtyx.com";
        let real = WebauthnBuilder::new(rp_id, &origin())
            .unwrap()
            .build()
            .unwrap();
        let evil_origin = Url::parse("https://evil.clairtyx.com").unwrap();
        let evil = WebauthnBuilder::new(rp_id, &evil_origin)
            .unwrap()
            .build()
            .unwrap();
        let (challenge, state) = evil
            .start_passkey_registration(user, "a@b.fr", "A", None)
            .unwrap();
        let response = device.do_registration(evil_origin, challenge).unwrap();
        assert!(real.finish_passkey_registration(&response, &state).is_err());
    }

    #[test]
    fn an_already_registered_key_is_excluded_from_a_second_registration() {
        let webauthn = build_webauthn(APP_URL, "Clairtyx").unwrap();
        let mut device = authenticator();
        let account = Uuid::new_v4();
        let (challenge, state) = webauthn
            .start_passkey_registration(account, "a@b.fr", "A", None)
            .unwrap();
        let passkey = webauthn
            .finish_passkey_registration(
                &device.do_registration(origin(), challenge).unwrap(),
                &state,
            )
            .unwrap();

        let (challenge, _) = webauthn
            .start_passkey_registration(
                account,
                "a@b.fr",
                "A",
                Some(vec![passkey.cred_id().clone()]),
            )
            .unwrap();
        assert_eq!(
            challenge
                .public_key
                .exclude_credentials
                .as_ref()
                .map(Vec::len),
            Some(1)
        );
    }
}
```

</details>


---

## 16. Stockage objet S3 ✅

**Manque.** Les photos de profil vivent dans un stockage objet privé (Garage en dev, AWS en prod). Règles à encoder :

- le type de l'image est reconnu à ses **premiers octets** (PNG, JPEG) ; l'en-tête `Content-Type` du client n'est
  jamais cru, car il décide du type servi ensuite ; **pas de SVG** (il peut porter du script) ; 2 Mo maximum ;
- avec un `endpoint` (Garage, MinIO), adressage **par chemin** ; sans lui, AWS ;
- identifiants par la chaîne standard du SDK (`AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_REGION`),
  jamais dans le code ; aucune requête à la construction du client ;
- objet absent (`NoSuchKey`) = `None`, pas une erreur ; stockage injoignable = **503**, pas 500 ;
- **ordre des écritures** : à l'envoi, l'image d'abord puis la base (au pire une image que la prochaine écriture
  remplace) ; à la suppression, la base d'abord puis l'objet (un objet orphelin n'est plus lisible, une ligne qui
  le pointerait le serait).

Le S3 réel est remplacé par un **faux S3 sur un port local** : le SDK parle vraiment HTTP, la requête est signée
(SigV4) et le test le vérifie. Garage et AWS eux-mêmes ne sont pas testés ici. Un `MemoryStore` à panne
commandable sert aux tests des services.

💡 **Étapes proposées**

```json
{ "type": "imageSniff", "id": "a1", "input": { "kind": "path", "path": "input.bytes" }, "allow": ["png", "jpeg"], "maxBytes": 2097152, "bind": "content_type" }
{ "type": "objectPut", "id": "a2", "bucketEnv": "AVATAR_BUCKET", "key": { "kind": "path", "path": "account.id" },
  "body": { "kind": "path", "path": "input.bytes" }, "contentType": { "kind": "path", "path": "content_type" } }
```

```rust
// src/s3.rs
//! Stockage objet privé (S3 : Garage en dev, AWS en prod) derrière un trait,
//! pour les photos de profil. Le type de l'image est reconnu à ses premiers
//! octets : l'en-tête `Content-Type` du client n'est jamais cru. Pas de SVG
//! (il peut porter du script).

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use aws_sdk_s3::{Client, error::DisplayErrorContext, primitives::ByteStream};

use crate::error::{Fail, FailResult};

/// 2 Mo, comme le sélecteur du front.
pub const MAX_BYTES: usize = 2 * 1024 * 1024;

const PNG: &str = "image/png";
const JPEG: &str = "image/jpeg";
const PNG_MAGIC: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
const JPEG_MAGIC: &[u8] = &[0xFF, 0xD8, 0xFF];

/// Le type d'une image d'après ses premiers octets ; `None` si ni PNG ni JPEG.
pub fn sniff(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(PNG_MAGIC) {
        Some(PNG)
    } else if bytes.starts_with(JPEG_MAGIC) {
        Some(JPEG)
    } else {
        None
    }
}

/// Valide une image reçue : non vide, bornée, PNG ou JPEG. Renvoie son type.
pub fn validate_image(bytes: &[u8]) -> FailResult<&'static str> {
    if bytes.is_empty() {
        return Err(Fail::BadRequest("La photo est vide.".into()));
    }
    if bytes.len() > MAX_BYTES {
        return Err(Fail::BadRequest("La photo dépasse 2 Mo.".into()));
    }
    sniff(bytes).ok_or_else(|| Fail::BadRequest("La photo doit être une image PNG ou JPEG.".into()))
}

#[async_trait]
pub trait ObjectStore: Send + Sync {
    async fn put(&self, key: &str, bytes: Vec<u8>, content_type: &str) -> FailResult<()>;
    /// `None` si l'objet n'existe pas.
    async fn get(&self, key: &str) -> FailResult<Option<Vec<u8>>>;
    async fn delete(&self, key: &str) -> FailResult<()>;
}

fn unavailable(detail: impl std::fmt::Display) -> Fail {
    tracing::warn!(error = %detail, "stockage objet en échec");
    Fail::Unavailable("stockage des photos".into())
}

pub struct S3Store {
    client: Client,
    bucket: String,
}

impl S3Store {
    pub fn new(client: Client, bucket: impl Into<String>) -> Self {
        Self {
            client,
            bucket: bucket.into(),
        }
    }

    /// Client AWS réel, ou un stockage compatible quand `endpoint` est renseigné
    /// (adressage par chemin, attendu par Garage). Identifiants : chaîne standard
    /// du SDK (`AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_REGION`).
    /// Aucune requête ici : le bucket est joint à l'usage.
    pub async fn connect(endpoint: Option<&str>, bucket: impl Into<String>) -> Self {
        let mut loader = aws_config::defaults(aws_config::BehaviorVersion::latest());
        if let Some(endpoint) = endpoint {
            loader = loader.endpoint_url(endpoint);
        }
        let sdk = loader.load().await;
        let mut config = aws_sdk_s3::config::Builder::from(&sdk);
        if endpoint.is_some() {
            config = config.force_path_style(true);
        }
        Self::new(Client::from_conf(config.build()), bucket)
    }
}

#[async_trait]
impl ObjectStore for S3Store {
    async fn put(&self, key: &str, bytes: Vec<u8>, content_type: &str) -> FailResult<()> {
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .body(ByteStream::from(bytes))
            .content_type(content_type)
            .send()
            .await
            .map_err(|e| unavailable(DisplayErrorContext(&e)))?;
        Ok(())
    }

    async fn get(&self, key: &str) -> FailResult<Option<Vec<u8>>> {
        match self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
        {
            Ok(output) => {
                let bytes = output.body.collect().await.map_err(unavailable)?;
                Ok(Some(bytes.into_bytes().to_vec()))
            }
            Err(e) if e.as_service_error().is_some_and(|s| s.is_no_such_key()) => Ok(None),
            Err(e) => Err(unavailable(DisplayErrorContext(&e))),
        }
    }

    async fn delete(&self, key: &str) -> FailResult<()> {
        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| unavailable(DisplayErrorContext(&e)))?;
        Ok(())
    }
}

/// Stockage en mémoire pour les tests des services : peut tomber en panne à la demande.
#[derive(Default)]
pub struct MemoryStore {
    objects: Mutex<HashMap<String, (Vec<u8>, String)>>,
    failing: AtomicBool,
}

impl MemoryStore {
    pub fn failing(&self, failing: bool) {
        self.failing.store(failing, Ordering::SeqCst);
    }

    pub fn object(&self, key: &str) -> Option<(Vec<u8>, String)> {
        self.lock().get(key).cloned()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, (Vec<u8>, String)>> {
        self.objects.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn check(&self) -> FailResult<()> {
        if self.failing.load(Ordering::SeqCst) {
            return Err(unavailable("panne simulée"));
        }
        Ok(())
    }
}

#[async_trait]
impl ObjectStore for MemoryStore {
    async fn put(&self, key: &str, bytes: Vec<u8>, content_type: &str) -> FailResult<()> {
        self.check()?;
        self.lock()
            .insert(key.to_owned(), (bytes, content_type.to_owned()));
        Ok(())
    }

    async fn get(&self, key: &str) -> FailResult<Option<Vec<u8>>> {
        self.check()?;
        Ok(self.lock().get(key).map(|(bytes, _)| bytes.clone()))
    }

    async fn delete(&self, key: &str) -> FailResult<()> {
        self.check()?;
        self.lock().remove(key);
        Ok(())
    }
}
```

<details><summary>Tests de <code>s3.rs</code></summary>

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use aws_sdk_s3::config::retry::RetryConfig;
    use aws_sdk_s3::config::{BehaviorVersion, Credentials, Region};
    use axum::Router;
    use axum::body::Bytes;
    use axum::extract::{Path, State};
    use axum::http::{HeaderMap, StatusCode, header};
    use axum::response::IntoResponse;
    use axum::routing::put;

    const PNG_BYTES: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 1, 2, 3];
    const JPEG_BYTES: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 3];

    type Objects = Arc<Mutex<HashMap<String, (Vec<u8>, String)>>>;

    /// Faux S3 sur un port local : stocke ce qu'on lui envoie, répond `NoSuchKey`
    /// (404 + XML) sur un objet absent, et note si la requête était signée.
    #[derive(Clone, Default)]
    struct FakeS3 {
        objects: Objects,
        signed: Arc<Mutex<Vec<bool>>>,
    }

    async fn s3_put(
        State(s): State<FakeS3>,
        Path((bucket, key)): Path<(String, String)>,
        headers: HeaderMap,
        body: Bytes,
    ) -> StatusCode {
        s.signed
            .lock()
            .unwrap()
            .push(headers.contains_key(header::AUTHORIZATION));
        let content_type = headers
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_owned();
        s.objects
            .lock()
            .unwrap()
            .insert(format!("{bucket}/{key}"), (body.to_vec(), content_type));
        StatusCode::OK
    }

    async fn s3_get(
        State(s): State<FakeS3>,
        Path((bucket, key)): Path<(String, String)>,
    ) -> axum::response::Response {
        match s.objects.lock().unwrap().get(&format!("{bucket}/{key}")) {
            Some((bytes, content_type)) => ([(header::CONTENT_TYPE, content_type.clone())], bytes.clone()).into_response(),
            None => (
                StatusCode::NOT_FOUND,
                [(header::CONTENT_TYPE, "application/xml")],
                "<?xml version=\"1.0\"?><Error><Code>NoSuchKey</Code><Message>absent</Message></Error>",
            )
                .into_response(),
        }
    }

    async fn s3_delete(
        State(s): State<FakeS3>,
        Path((bucket, key)): Path<(String, String)>,
    ) -> StatusCode {
        s.objects.lock().unwrap().remove(&format!("{bucket}/{key}"));
        StatusCode::NO_CONTENT
    }

    async fn spawn_fake() -> (FakeS3, String) {
        let fake = FakeS3::default();
        let app = Router::new()
            .route("/{bucket}/{key}", put(s3_put).get(s3_get).delete(s3_delete))
            .with_state(fake.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await });
        (fake, url)
    }

    /// Client réglé comme `S3Store::connect` avec un `endpoint` (chemin forcé),
    /// identifiants explicites et sans nouvelle tentative pour garder les tests rapides.
    fn store_for(endpoint: &str) -> S3Store {
        let config = aws_sdk_s3::Config::builder()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new("garage"))
            .endpoint_url(endpoint)
            .force_path_style(true)
            .credentials_provider(Credentials::new("AKTEST", "secret", None, None, "test"))
            .retry_config(RetryConfig::disabled())
            .build();
        S3Store::new(Client::from_conf(config), "avatars")
    }

    #[test]
    fn sniffing_trusts_the_bytes_not_the_name() {
        assert_eq!(sniff(PNG_BYTES), Some("image/png"));
        assert_eq!(sniff(JPEG_BYTES), Some("image/jpeg"));
        assert_eq!(sniff(b"<svg onload=alert(1)>"), None, "pas de SVG");
        assert_eq!(sniff(b"GIF89a"), None);
        assert_eq!(sniff(b""), None);
    }

    #[test]
    fn validation_rejects_empty_oversized_and_unknown_images() {
        assert_eq!(validate_image(PNG_BYTES).unwrap(), "image/png");
        assert!(validate_image(&[]).is_err());
        assert!(validate_image(b"texte").is_err());
        let mut huge = PNG_BYTES.to_vec();
        huge.resize(MAX_BYTES + 1, 0);
        assert!(validate_image(&huge).is_err());
        huge.truncate(MAX_BYTES);
        assert!(validate_image(&huge).is_ok(), "la borne est incluse");
    }

    #[tokio::test]
    async fn s3_store_puts_gets_and_deletes_over_http() {
        let (fake, url) = spawn_fake().await;
        let store = store_for(&url);
        let key = "7d9f0c1e";

        store
            .put(key, PNG_BYTES.to_vec(), "image/png")
            .await
            .unwrap();
        let (stored, content_type) = fake
            .objects
            .lock()
            .unwrap()
            .get("avatars/7d9f0c1e")
            .cloned()
            .unwrap();
        assert_eq!(stored, PNG_BYTES);
        assert_eq!(content_type, "image/png");
        assert_eq!(
            fake.signed.lock().unwrap().as_slice(),
            [true],
            "requête signée SigV4"
        );

        assert_eq!(store.get(key).await.unwrap().as_deref(), Some(PNG_BYTES));

        store.delete(key).await.unwrap();
        assert_eq!(
            store.get(key).await.unwrap(),
            None,
            "NoSuchKey → None, pas une erreur"
        );
    }

    #[tokio::test]
    async fn a_missing_object_is_none_but_an_unreachable_store_is_503() {
        let (_fake, url) = spawn_fake().await;
        assert_eq!(store_for(&url).get("inconnu").await.unwrap(), None);

        // Port fermé : le stockage est tombé.
        let closed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let dead_url = format!("http://{}", closed.local_addr().unwrap());
        drop(closed);
        let err = store_for(&dead_url)
            .put("k", PNG_BYTES.to_vec(), "image/png")
            .await
            .unwrap_err();
        assert!(matches!(err, Fail::Unavailable(_)));
        assert!(matches!(
            store_for(&dead_url).get("k").await,
            Err(Fail::Unavailable(_))
        ));
    }

    #[tokio::test]
    async fn memory_store_behaves_like_the_trait_and_can_fail_on_demand() {
        let store = MemoryStore::default();
        store
            .put("a", JPEG_BYTES.to_vec(), "image/jpeg")
            .await
            .unwrap();
        assert_eq!(store.object("a").unwrap().1, "image/jpeg");
        assert_eq!(store.get("a").await.unwrap().as_deref(), Some(JPEG_BYTES));
        store.delete("a").await.unwrap();
        assert_eq!(store.get("a").await.unwrap(), None);

        store.failing(true);
        assert!(matches!(
            store.put("a", vec![1], "image/png").await,
            Err(Fail::Unavailable(_))
        ));
    }
}
```

</details>


---

## 17. MTA : service préfabriqué

Le MTA n'est pas du code personnalisable : c'est **du code fixe** (file `mail_queue`, worker, DKIM, SMTP direct
ou relais, serveur de bounces, santé, métriques) plus **deux entrées de données** : les événements écoutés et les
templates, semés par une migration. Le builder le fournit donc tel quel (`kind: "mta"`), sans étapes.

Tout est dans [`SERVICE_BUILDER_MTA.md`](SERVICE_BUILDER_MTA.md) : le code complet du service, les règles
d'événements en données (qui remplacent les six `enqueue_*`), le générateur de migration, 23 tests
complémentaires sur le code existant, et un défaut trouvé dans l'envoi SMTP.

---

## Ordre de mise en œuvre conseillé

1. **`fail`** (§1) et les types `uuid`, `Option`, `Vec` : sans eux, aucun handler réel n'est exprimable.
2. **`hashPassword`, `verifyPassword`, `randomToken`, `sha256`** (§3, §4) : débloquent register et login.
3. **`respond` étendu** (statut, cookies, en-têtes) et **`rateLimit`** (§5, §6) : login complet.
4. **`sql` avec `RETURNING`** et `Table.softDelete` / multi-tenant (§11, §12) : tous les jetons à usage unique.
5. **`verifyHmac`**, **`schedule`**, **`Consumer.idempotent`** (§8, §9, §10) : Connect, cleaner, notifications.
6. **`encrypt` / `decrypt`** et **validation déclarative** (§2, §7) : 2FA et confort d'écriture.
7. **`signJwt` / `verifyJwt` / `jwks`** (§13), puis **TOTP** (§14) et **passkeys** (§15) : le reste de `svc-auth`.
8. **`objectPut` / `objectGet`** (§16) : photos de profil de `svc-user`.
9. **`kind: "mta"`** : le service est déjà écrit, il reste à lire `mta.json` et à générer sa migration.

## Ce qui n'est pas prouvé

- **Le SQL** (📝 §10 à §12, §14, §15) : pas d'accès PostgreSQL dans ces essais.
- **Les services réels** : Garage/AWS pour S3, un navigateur ou un jeton CTAP2 pour la connexion découvrable,
  un vrai MX sur Internet pour le MTA, Stripe.
- **Les schémas JSON proposés** (💡) : ils décrivent l'intention, le builder ne les connaît pas encore.
