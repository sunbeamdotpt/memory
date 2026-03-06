// OIDC JWT verification
//
// Fetches the provider's JWKS at startup via the OpenID Connect discovery document
// and validates RS256/ES256/PS256 bearer tokens on every request.
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
}

impl OidcVerifier {
    /// Fetch discovery doc and JWKS from the issuer. Called once at startup.
    pub async fn new(issuer: &str, audience: Option<String>) -> Result<Self, OidcError> {
        let client = reqwest::Client::new();

        // 1. Discovery document
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
            .ok_or_else(|| OidcError::Discovery("discovery doc missing jwks_uri".to_string()))?;

        // 2. JWKS
        let jwks: JwkSet = client
            .get(jwks_uri)
            .send()
            .await
            .map_err(|e| OidcError::Jwks(e.to_string()))?
            .error_for_status()
            .map_err(|e| OidcError::Jwks(e.to_string()))?
            .json()
            .await
            .map_err(|e| OidcError::Jwks(e.to_string()))?;

        Ok(Self {
            issuer: issuer.to_string(),
            audience,
            jwks,
        })
    }

    /// Verify a raw Bearer token string. Returns `Ok(())` if valid.
    pub fn verify(&self, token: &str) -> Result<(), OidcError> {
        // Decode header to get kid + algorithm
        let header = decode_header(token).map_err(|e| OidcError::Token(e.to_string()))?;

        let kid = header.kid.as_deref().unwrap_or("");

        // Find the matching JWK
        let jwk = self
            .jwks
            .find(kid)
            .ok_or_else(|| OidcError::Token(format!("no JWK found for kid={kid:?}")))?;

        // Build the decoding key from the JWK
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

        // Determine algorithm from header (fall back to RS256)
        let alg = header.alg;
        // Only allow standard OIDC signing algorithms
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
