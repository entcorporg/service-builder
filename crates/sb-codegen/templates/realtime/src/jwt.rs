//! Verification of the access tokens (EdDSA).
//!
//! The service holds the public key only, read from the JWKS: it validates a request by itself, without calling the API
//! each time, and cannot forge a token. The claims are `sub` (account), `tid` (company, optional: a system without companies
//! leaves it out), `sid` (session), `exp`, and `permissions` (the rights of the account: `module.view`, `module.manage`; optional).

use std::{
    collections::HashMap,
    sync::RwLock,
    time::{Duration, Instant},
};

use jsonwebtoken::{
    Algorithm, DecodingKey, Validation, decode, decode_header,
    jwk::{Jwk, JwkSet},
};
use serde::Deserialize;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::{
    contract::ISSUER,
    errors::{AppError, AppResult},
};

/// Tolerance on `exp` for clocks that differ a little between services.
const LEEWAY_SECS: u64 = 5;
/// Least time between two reads of the JWKS: a token with an unknown `kid` (forged, or a key being rotated) must not make
/// the API be hammered.
const MIN_REFRESH_INTERVAL: Duration = Duration::from_secs(30);
const FETCH_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Deserialize)]
pub struct AccessClaims {
    pub sub: Uuid,
    /// The company. Absent in a system that has none: the stream is then in the scope of the streams without a company.
    #[serde(default)]
    pub tid: Option<Uuid>,
    pub sid: Uuid,
    /// Unix seconds.
    pub exp: i64,
    #[serde(default)]
    pub permissions: Vec<String>,
}

#[derive(Default)]
struct Keys {
    by_kid: HashMap<String, DecodingKey>,
    fetched_at: Option<Instant>,
}

pub struct AccessVerifier {
    jwks_url: String,
    http: reqwest::Client,
    /// Read on every stream that opens, written on a rotation only: a plain lock, never held across a wait.
    keys: RwLock<Keys>,
    /// One read of the JWKS at a time: a thousand tokens with a key that is not known yet make one request, not a thousand.
    refresh: Mutex<()>,
}

impl std::fmt::Debug for AccessVerifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccessVerifier").field("jwks_url", &self.jwks_url).finish_non_exhaustive()
    }
}

impl AccessVerifier {
    /// Reads nothing at start-up: the service starts even if the API is not ready yet.
    pub fn new(jwks_url: String) -> Self {
        Self::with_keys(jwks_url, Keys::default())
    }

    /// Keys known in advance, as if the JWKS had just been read (tests, which sign their own tokens).
    pub fn with_jwks(jwks_url: String, set: &JwkSet) -> Self {
        Self::with_keys(jwks_url, Keys { by_kid: keys_by_kid(set), fetched_at: Some(Instant::now()) })
    }

    fn with_keys(jwks_url: String, keys: Keys) -> Self {
        Self {
            jwks_url,
            http: reqwest::Client::builder().timeout(FETCH_TIMEOUT).build().unwrap_or_default(),
            keys: RwLock::new(keys),
            refresh: Mutex::new(()),
        }
    }

    /// Checks signature, algorithm (EdDSA only), issuer and expiry. Every refused token gives the same 401; a 503 only
    /// when no key could ever be read.
    pub async fn verify(&self, token: &str) -> AppResult<AccessClaims> {
        let header = decode_header(token).map_err(|_| AppError::Unauthorized)?;
        if header.alg != Algorithm::EdDSA {
            return Err(AppError::Unauthorized);
        }
        let kid = header.kid.ok_or(AppError::Unauthorized)?;
        if let Some(claims) = self.verify_with_known_key(token, &kid) {
            return claims;
        }
        self.refresh_if_stale().await?;
        self.verify_with_known_key(token, &kid).unwrap_or(Err(AppError::Unauthorized))
    }

    /// `None` when no known key has this `kid`.
    fn verify_with_known_key(&self, token: &str, kid: &str) -> Option<AppResult<AccessClaims>> {
        let keys = self.keys.read().unwrap_or_else(|p| p.into_inner());
        let key = keys.by_kid.get(kid)?;
        Some(decode::<AccessClaims>(token, key, &validation()).map(|data| data.claims).map_err(|_| AppError::Unauthorized))
    }

    async fn refresh_if_stale(&self) -> AppResult<()> {
        let _one_at_a_time = self.refresh.lock().await;
        {
            // Another stream may have read the JWKS while this one waited.
            let keys = self.keys.read().unwrap_or_else(|p| p.into_inner());
            if keys.fetched_at.is_some_and(|at| at.elapsed() < MIN_REFRESH_INTERVAL) {
                return Ok(());
            }
        }
        // Counted even when it fails: an API that is down must not be asked again for every stream.
        self.keys.write().unwrap_or_else(|p| p.into_inner()).fetched_at = Some(Instant::now());
        match self.fetch().await {
            Ok(by_kid) => {
                self.keys.write().unwrap_or_else(|p| p.into_inner()).by_kid = by_kid;
                Ok(())
            }
            Err(err) => {
                tracing::warn!(error = ?err, url = %self.jwks_url, "jwks: cannot be read");
                if self.keys.read().unwrap_or_else(|p| p.into_inner()).by_kid.is_empty() {
                    Err(AppError::Unavailable("jwks".into()))
                } else {
                    // The keys already known stay valid: only the `kid` asked for is unknown, the token is refused (401).
                    Ok(())
                }
            }
        }
    }

    async fn fetch(&self) -> anyhow::Result<HashMap<String, DecodingKey>> {
        let set: JwkSet = self.http.get(&self.jwks_url).send().await?.error_for_status()?.json().await?;
        Ok(keys_by_kid(&set))
    }
}

fn keys_by_kid(set: &JwkSet) -> HashMap<String, DecodingKey> {
    set.keys
        .iter()
        .filter_map(|jwk: &Jwk| {
            let kid = jwk.common.key_id.clone()?;
            let key = DecodingKey::from_jwk(jwk).ok()?;
            Some((kid, key))
        })
        .collect()
}

fn validation() -> Validation {
    let mut validation = Validation::new(Algorithm::EdDSA);
    validation.leeway = LEEWAY_SECS;
    validation.set_issuer(&[ISSUER]);
    validation.set_required_spec_claims(&["exp", "iss", "sub"]);
    validation
}

#[cfg(test)]
pub mod testing {
    //! Tokens and keys for the tests of this crate and of the load tool.
    use super::*;
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    use ed25519_dalek::SigningKey;
    use jsonwebtoken::{EncodingKey, Header, encode};
    use serde_json::json;

    /// PKCS#8 prefix of an Ed25519 private key (RFC 8410).
    const PKCS8_ED25519_PREFIX: [u8; 16] = [0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20];

    pub fn encoding_key(seed: u8) -> EncodingKey {
        let mut pkcs8 = PKCS8_ED25519_PREFIX.to_vec();
        pkcs8.extend_from_slice(&[seed; 32]);
        EncodingKey::from_ed_der(&pkcs8)
    }

    pub fn jwks(seed: u8, kid: &str) -> JwkSet {
        let public = SigningKey::from_bytes(&[seed; 32]).verifying_key().to_bytes();
        serde_json::from_value(json!({
            "keys": [{"kty": "OKP", "crv": "Ed25519", "use": "sig", "alg": "EdDSA", "kid": kid, "x": URL_SAFE_NO_PAD.encode(public)}]
        }))
        .unwrap()
    }

    pub fn token(seed: u8, kid: &str, issuer: &str, claims: serde_json::Value) -> String {
        let mut header = Header::new(Algorithm::EdDSA);
        header.kid = Some(kid.to_string());
        let mut body = claims;
        body["iss"] = issuer.into();
        encode(&header, &body, &encoding_key(seed)).unwrap()
    }

    pub fn claims(exp_offset: i64, permissions: &[&str]) -> serde_json::Value {
        json!({
            "sub": Uuid::new_v4(), "tid": Uuid::new_v4(), "sid": Uuid::new_v4(),
            "exp": chrono::Utc::now().timestamp() + exp_offset, "permissions": permissions,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;

    const DEAD_URL: &str = "http://127.0.0.1:9/jwks";

    #[tokio::test]
    async fn a_valid_token_gives_its_claims() {
        let verifier = AccessVerifier::with_jwks(DEAD_URL.into(), &jwks(1, "k1"));
        let claims = verifier.verify(&token(1, "k1", ISSUER, self::claims(300, &["contacts.view"]))).await.unwrap();
        assert_eq!(claims.permissions, vec!["contacts.view"]);
    }

    #[tokio::test]
    async fn a_token_without_a_company_or_permissions_is_valid() {
        let verifier = AccessVerifier::with_jwks(DEAD_URL.into(), &jwks(1, "k1"));
        let bare = serde_json::json!({"sub": Uuid::new_v4(), "sid": Uuid::new_v4(), "exp": chrono::Utc::now().timestamp() + 300});
        let claims = verifier.verify(&token(1, "k1", ISSUER, bare)).await.unwrap();
        assert_eq!((claims.tid, claims.permissions), (None, vec![]));
    }

    #[tokio::test]
    async fn a_wrong_key_issuer_expiry_or_algorithm_is_a_401() {
        let verifier = AccessVerifier::with_jwks(DEAD_URL.into(), &jwks(1, "k1"));
        for (what, token) in [
            ("other key", token(2, "k1", ISSUER, self::claims(300, &[]))),
            ("other issuer", token(1, "k1", "someone-else", self::claims(300, &[]))),
            ("expired", token(1, "k1", ISSUER, self::claims(-300, &[]))),
            ("garbage", "not.a.token".to_string()),
        ] {
            assert!(matches!(verifier.verify(&token).await, Err(AppError::Unauthorized)), "{what}");
        }
    }

    #[tokio::test]
    async fn an_unknown_key_with_the_api_down_is_a_401_once_a_key_is_known_and_a_503_before() {
        let verifier = AccessVerifier::with_jwks(DEAD_URL.into(), &jwks(1, "k1"));
        // A key is known: the unknown `kid` is refused, not a failure of the service.
        assert!(matches!(verifier.verify(&token(1, "other", ISSUER, self::claims(300, &[]))).await, Err(AppError::Unauthorized)));
        let empty = AccessVerifier::new(DEAD_URL.into());
        assert!(matches!(empty.verify(&token(1, "k1", ISSUER, self::claims(300, &[]))).await, Err(AppError::Unavailable(_))));
    }
}
