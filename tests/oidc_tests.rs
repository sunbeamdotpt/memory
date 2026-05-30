use mcp_server::api::oidc::OidcVerifier;
use jsonwebtoken::jwk::{JwkSet, Jwk, AlgorithmParameters, RSAKeyParameters, CommonParameters, PublicKeyUse, KeyAlgorithm, OctetKeyType, RSAKeyType};

fn empty_jwks() -> JwkSet {
    JwkSet { keys: vec![] }
}

fn rsa_jwk(kid: &str) -> Jwk {
    Jwk {
        common: CommonParameters {
            public_key_use: Some(PublicKeyUse::Signature),
            key_operations: None,
            key_algorithm: Some(KeyAlgorithm::RS256),
            key_id: Some(kid.to_string()),
            x509_url: None,
            x509_chain: None,
            x509_sha1_fingerprint: None,
            x509_sha256_fingerprint: None,
        },
        algorithm: AlgorithmParameters::RSA(RSAKeyParameters {
            key_type: RSAKeyType::RSA,
            n: "test".to_string(),
            e: "AQAB".to_string(),
        }),
    }
}

#[test]
fn test_verify_invalid_jwt_format() {
    let mut verifier = OidcVerifier::test_new("https://example.com", None, empty_jwks());
    let result = verifier.verify("not-a-jwt");
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("token") || err.contains("decode"));
}

#[test]
fn test_verify_malformed_jwt() {
    let mut verifier = OidcVerifier::test_new("https://example.com", None, empty_jwks());
    let result = verifier.verify("eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxIn0.signature");
    assert!(result.is_err());
}

#[test]
fn test_verify_unknown_kid() {
    let jwks = JwkSet { keys: vec![rsa_jwk("known-kid")] };
    let mut verifier = OidcVerifier::test_new("https://example.com", None, jwks);

    // JWT with kid="unknown-kid" but valid structure
    // Header: {"alg":"RS256","kid":"unknown-kid","typ":"JWT"}
    let header = "eyJhbGciOiJSUzI1NiIsImtpZCI6InVua25vd24ta2lkIiwidHlwIjoiSldUIn0";
    let payload = "eyJzdWIiOiIxMjM0NTY3ODkwIn0";
    let token = format!("{}.{}.signature", header, payload);

    let result = verifier.verify(&token);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("kid"));
}

#[test]
fn test_verify_unsupported_algorithm() {
    let jwks = JwkSet { keys: vec![rsa_jwk("kid1")] };
    let mut verifier = OidcVerifier::test_new("https://example.com", None, jwks);

    // Header with HS256 algorithm
    let header = "eyJhbGciOiJIUzI1NiIsImtpZCI6ImtpZDEiLCJ0eXAiOiJKV1QifQ";
    let payload = "eyJzdWIiOiIxMjM0NTY3ODkwIn0";
    let token = format!("{}.{}.signature", header, payload);

    let result = verifier.verify(&token);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("algorithm") || err.contains("not permitted"));
}

#[test]
fn test_verify_unsupported_jwk_type() {
    let jwk = Jwk {
        common: CommonParameters {
            public_key_use: Some(PublicKeyUse::Signature),
            key_operations: None,
            key_algorithm: Some(KeyAlgorithm::RS256),
            key_id: Some("kid1".to_string()),
            x509_url: None,
            x509_chain: None,
            x509_sha1_fingerprint: None,
            x509_sha256_fingerprint: None,
        },
        algorithm: AlgorithmParameters::OctetKey(jsonwebtoken::jwk::OctetKeyParameters {
            key_type: OctetKeyType::Octet,
            value: "test".to_string(),
        }),
    };
    let jwks = JwkSet { keys: vec![jwk] };
    let mut verifier = OidcVerifier::test_new("https://example.com", None, jwks);

    let header = "eyJhbGciOiJSUzI1NiIsImtpZCI6ImtpZDEiLCJ0eXAiOiJKV1QifQ";
    let payload = "eyJzdWIiOiIxMjM0NTY3ODkwIn0";
    let token = format!("{}.{}.signature", header, payload);

    let result = verifier.verify(&token);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("unsupported"));
}

#[test]
fn test_verify_with_audience() {
    let jwks = JwkSet { keys: vec![rsa_jwk("kid1")] };
    let mut verifier = OidcVerifier::test_new("https://example.com", Some("my-app".to_string()), jwks);

    let header = "eyJhbGciOiJSUzI1NiIsImtpZCI6ImtpZDEiLCJ0eXAiOiJKV1QifQ";
    let payload = "eyJzdWIiOiIxMjM0NTY3ODkwIn0";
    let token = format!("{}.{}.signature", header, payload);

    let result = verifier.verify(&token);
    assert!(result.is_err());
    // Should fail because signature is invalid, but audience validation path is exercised
}

#[test]
fn test_oidc_error_display() {
    let e = mcp_server::api::oidc::OidcError::Discovery("net error".to_string());
    assert!(e.to_string().contains("OIDC discovery"));

    let e = mcp_server::api::oidc::OidcError::Jwks("bad jwks".to_string());
    assert!(e.to_string().contains("OIDC JWKS"));

    let e = mcp_server::api::oidc::OidcError::Token("bad token".to_string());
    assert!(e.to_string().contains("OIDC token"));
}
