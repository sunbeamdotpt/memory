use mcp_server::{
    config::MemoryConfig,
    api::mcp_http::AuthConfig,
};

// ── AuthConfig::from_config tests ─────────────────────────────────────────────

#[test]
fn test_auth_config_local_only() {
    let config = MemoryConfig::default();
    let auth = AuthConfig::from_config(&config).unwrap();
    assert!(matches!(auth, AuthConfig::LocalOnly));
    assert!(!auth.is_remote());
}

#[test]
fn test_auth_config_bearer() {
    let config = MemoryConfig {
        auth_token: Some("my-token".to_string()),
        ..Default::default()
    };
    let auth = AuthConfig::from_config(&config).unwrap();
    assert!(matches!(auth, AuthConfig::Bearer(ref t) if t == "my-token"));
    assert!(auth.is_remote());
}

#[test]
fn test_auth_config_oidc_requires_async() {
    let config = MemoryConfig {
        oidc_issuer: Some("https://example.com".to_string()),
        ..Default::default()
    };
    let result = AuthConfig::from_config(&config);
    assert!(result.is_err());
}

#[test]
fn test_auth_config_oidc_takes_priority_over_bearer() {
    let config = MemoryConfig {
        oidc_issuer: Some("https://example.com".to_string()),
        auth_token: Some("token".to_string()),
        ..Default::default()
    };
    let result = AuthConfig::from_config(&config);
    assert!(result.is_err());
}
