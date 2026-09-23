//! Inbound bearer verification for the remote engine listener.
//!
//! Upstream zeron routes remote clients through the edge Worker, which
//! verifies WorkOS AuthKit access-token JWTs against the WorkOS JWKS
//! (`edge/src/auth.ts`). The remote listener serves clients directly, so it
//! performs the same verification itself:
//!
//! - **WorkOS mode** (a WorkOS client id is configured): the bearer is an
//!   AuthKit access token, verified against the WorkOS JWKS with the issuer
//!   pinned to the client — the same checks the edge applies to relay
//!   clients. `WORKOS_ISSUER`/`WORKOS_JWKS_URL` override the endpoints,
//!   matching the edge's env.
//! - **Dev mode** (no client id): the bearer IS the user id — optionally
//!   `userId@orgId` to carry a fake org claim — mirroring the edge's dev
//!   verification so local development and tests run offline.

use std::time::{Duration, Instant};

use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use serde::Deserialize;
use tokio::sync::RwLock;

/// A verified remote caller.
#[derive(Debug, Clone)]
pub struct RemoteIdentity {
    pub user_id: String,
    /// WorkOS `org_id` claim when the caller's session is org-scoped.
    pub org_id: Option<String>,
}

/// How long a fetched JWKS stays fresh before the verifier refetches it.
const JWKS_TTL: Duration = Duration::from_secs(3600);
const HTTP_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Deserialize)]
struct Claims {
    sub: String,
    #[serde(default)]
    org_id: Option<String>,
}

enum Mode {
    /// The bearer is the user id; `userId@orgId` carries a fake org claim.
    Dev,
    /// The bearer is a WorkOS AuthKit access token.
    Workos { issuer: String, jwks_url: String },
}

/// Verifies inbound remote-listener bearers. Shared as one `Arc` per engine.
pub struct RemoteAuthorizer {
    mode: Mode,
    http: reqwest::Client,
    jwks: RwLock<Option<(Instant, JwkSet)>>,
}

impl RemoteAuthorizer {
    /// Build from the engine's WorkOS client id: `None` = dev mode.
    pub fn new(workos_client_id: Option<&str>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(HTTP_TIMEOUT)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        let mode = match workos_client_id {
            Some(client_id) => Mode::Workos {
                issuer: std::env::var("WORKOS_ISSUER").unwrap_or_else(|_| {
                    format!("https://api.workos.com/user_management/{client_id}")
                }),
                jwks_url: std::env::var("WORKOS_JWKS_URL")
                    .unwrap_or_else(|_| format!("https://api.workos.com/sso/jwks/{client_id}")),
            },
            None => Mode::Dev,
        };
        Self {
            mode,
            http,
            jwks: RwLock::new(None),
        }
    }

    /// `Some(identity)` when the credential is valid; `None` rejects.
    pub async fn verify(&self, credential: &str) -> Option<RemoteIdentity> {
        match &self.mode {
            Mode::Dev => {
                let (user_id, org_id) = match credential.split_once('@') {
                    Some((user_id, org_id)) => (user_id, Some(org_id)),
                    None => (credential, None),
                };
                (!user_id.is_empty()).then(|| RemoteIdentity {
                    user_id: user_id.to_owned(),
                    org_id: org_id.map(str::to_owned),
                })
            }
            Mode::Workos { issuer, jwks_url } => {
                self.verify_jwt(credential, issuer, jwks_url).await
            }
        }
    }

    async fn verify_jwt(
        &self,
        credential: &str,
        issuer: &str,
        jwks_url: &str,
    ) -> Option<RemoteIdentity> {
        let header = decode_header(credential).ok()?;
        let kid = header.kid.as_deref()?;
        // Cached JWKS, refetched once when the kid is unknown (key rotation).
        let key = match self.cached_key(kid).await {
            Some(key) => key,
            None => {
                self.refresh_jwks(jwks_url).await;
                self.cached_key(kid).await?
            }
        };
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_issuer(&[issuer]);
        let data = decode::<Claims>(credential, &key, &validation).ok()?;
        (!data.claims.sub.is_empty()).then(|| RemoteIdentity {
            user_id: data.claims.sub,
            org_id: data.claims.org_id,
        })
    }

    async fn cached_key(&self, kid: &str) -> Option<DecodingKey> {
        let cache = self.jwks.read().await;
        let Some((fetched_at, set)) = cache.as_ref() else {
            return None;
        };
        if fetched_at.elapsed() > JWKS_TTL {
            return None;
        }
        DecodingKey::from_jwk(set.find(kid)?).ok()
    }

    async fn refresh_jwks(&self, jwks_url: &str) {
        let fetched = match self.http.get(jwks_url).send().await {
            Ok(response) => match response.error_for_status() {
                Ok(response) => response.json::<JwkSet>().await.ok(),
                Err(error) => {
                    tracing::warn!(%error, "remote auth: JWKS fetch rejected");
                    None
                }
            },
            Err(error) => {
                tracing::warn!(%error, "remote auth: JWKS fetch failed");
                None
            }
        };
        if let Some(set) = fetched {
            *self.jwks.write().await = Some((Instant::now(), set));
        }
    }
}
