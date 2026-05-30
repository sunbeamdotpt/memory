// OIDC JWT verification
//
// Fetches the provider's JWKS at startup via the OpenID Connect discovery document
// and validates RS256/ES256/PS256 bearer tokens on every request.
// Refreshes JWKS automatically on kid miss or TTL expiry.
//
// Environment variables:
//   MCP_OIDC_ISSUER   — issuer URL, e.g. https://auth.example.com
//   MCP_OIDC_AUDIENCE — (optional) expected `aud` claim

use jsonwebtoken::{
    decode, decode_header,
    jwk::{AlgorithmParameters, JwkSet},
    Algorithm, DecodingKey, Validation,
};
use serde_json::Value;
use std::time::{Duration, Instant};

#[derive(Debug)]
pub enum OidcError {
    Discovery(String),
    Jwks(String),
    Token(String),
}

impl std::fmt::Display for OidcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Discovery(s) => write!(f, "OIDC discovery: {s}"),
            Self::Jwks(s) => write!(f, "OIDC JWKS: {s}"),
            Self::Token(s) => write!(f, "OIDC token: {s}"),
        }
    }
}

pub struct OidcVerifier {
    issuer: String,
    audience: Option<String>,
    jwks: JwkSet,
    _jwks_uri: String,
    last_fetched: Instant,
    ttl: Duration,
}

impl OidcVerifier {
    /// Fetch discovery doc and JWKS from the issuer. Called once at startup.
    pub async fn new(issuer: &str, audience: Option<String>) -> Result<Self, OidcError> {
        let (jwks, jwks_uri) = Self::fetch_jwks(issuer).await?;
        Ok(Self {
            issuer: issuer.to_string(),
            audience,
            jwks,
            _jwks_uri: jwks_uri,
            last_fetched: Instant::now(),
            ttl: Duration::from_secs(3600),
        })
    }

    #[doc(hidden)]
    pub fn test_new(issuer: &str, audience: Option<String>, jwks: JwkSet) -> Self {
        Self {
            issuer: issuer.to_string(),
            audience,
            jwks,
            _jwks_uri: String::new(),
            last_fetched: Instant::now(),
            ttl: Duration::from_secs(3600),
        }
    }

    async fn fetch_jwks(issuer: &str) -> Result<(JwkSet, String), OidcError> {
        let client = reqwest::Client::new();

        let disc_url = format!(
            "{}/.well-known/openid-configuration",
            issuer.trim_end_matches('/')
        );
        let disc: Value = client
            .get(&disc_url)
            .send()
            .await
            .map_err(|e| OidcError::Discovery(e.to_string()))?
            .error_for_status()
            .map_err(|e| OidcError::Discovery(e.to_string()))?
            .json()
            .await
            .map_err(|e| OidcError::Discovery(e.to_string()))?;

        let jwks_uri = disc["jwks_uri"]
            .as_str()
            .ok_or_else(|| OidcError::Discovery("discovery doc missing jwks_uri".to_string()))?
            .to_string();

        let jwks: JwkSet = client
            .get(&jwks_uri)
            .send()
            .await
            .map_err(|e| OidcError::Jwks(e.to_string()))?
            .error_for_status()
            .map_err(|e| OidcError::Jwks(e.to_string()))?
            .json()
            .await
            .map_err(|e| OidcError::Jwks(e.to_string()))?;

        Ok((jwks, jwks_uri))
    }

    /// Verify a raw Bearer token string. Returns `Ok(())` if valid.
    pub fn verify(&mut self, token: &str) -> Result<(), OidcError> {
        let header = decode_header(token).map_err(|e| OidcError::Token(e.to_string()))?;
        let kid = header.kid.as_deref().unwrap_or("");

        let jwk = self.jwks.find(kid);
        let jwk = match jwk {
            Some(j) => j,
            None => {
                // Try refreshing JWKS if kid not found or TTL expired
                if self.last_fetched.elapsed() > self.ttl {
                    let rt = tokio::runtime::Handle::try_current()
                        .map_err(|e| OidcError::Jwks(e.to_string()))?;
                    let (new_jwks, _) = rt.block_on(Self::fetch_jwks(&self.issuer))?;
                    self.jwks = new_jwks;
                    self.last_fetched = Instant::now();
                    self.jwks.find(kid)
                        .ok_or_else(|| OidcError::Token(format!("no JWK found for kid={kid:?} after refresh")))?
                } else {
                    return Err(OidcError::Token(format!("no JWK found for kid={kid:?}")));
                }
            }
        };

        let key = match &jwk.algorithm {
            AlgorithmParameters::RSA(rsa) => {
                DecodingKey::from_rsa_components(&rsa.n, &rsa.e)
                    .map_err(|e| OidcError::Token(e.to_string()))?
            }
            AlgorithmParameters::EllipticCurve(ec) => {
                DecodingKey::from_ec_components(&ec.x, &ec.y)
                    .map_err(|e| OidcError::Token(e.to_string()))?
            }
            other => {
                return Err(OidcError::Token(format!(
                    "unsupported JWK algorithm: {other:?}"
                )))
            }
        };

        let alg = header.alg;
        match alg {
            Algorithm::RS256
            | Algorithm::RS384
            | Algorithm::RS512
            | Algorithm::PS256
            | Algorithm::PS384
            | Algorithm::PS512
            | Algorithm::ES256
            | Algorithm::ES384 => {}
            other => {
                return Err(OidcError::Token(format!(
                    "algorithm {other:?} not permitted"
                )))
            }
        }

        let mut validation = Validation::new(alg);
        validation.set_issuer(&[&self.issuer]);

        if let Some(aud) = &self.audience {
            validation.set_audience(&[aud]);
        } else {
            validation.validate_aud = false;
        }

        decode::<Value>(token, &key, &validation)
            .map(|_| ())
            .map_err(|e| OidcError::Token(e.to_string()))
    }
}
