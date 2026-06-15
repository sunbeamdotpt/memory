use sunbeam_memory::config::MemoryConfig;
use sunbeam_memory::embedding::service::EmbeddingError;
use sunbeam_memory::error::ServerError;

#[test]
fn test_config_default() {
    let config = MemoryConfig::default();
    assert!(!config.base_dir.is_empty());
    assert_eq!(config.auth_token, None);
    assert_eq!(config.oidc_issuer, None);
    assert_eq!(config.oidc_audience, None);
    assert_eq!(config.session_ttl_hours, 24);
}

#[test]
fn test_config_from_env_picks_up_base_dir() {
    let config = MemoryConfig::from_env();
    // Should not panic and should produce a valid path
    assert!(!config.base_dir.is_empty());
}

#[test]
fn test_config_from_env_with_token() {
    // We can't safely test env vars in parallel tests, but we can verify
    // the struct accepts values correctly by constructing it directly.
    let config = MemoryConfig {
        auth_token: Some("secret".to_string()),
        ..MemoryConfig::default()
    };
    assert_eq!(config.auth_token, Some("secret".to_string()));
}

#[test]
fn test_config_clone() {
    let config = MemoryConfig::default();
    let cloned = config.clone();
    assert_eq!(config.base_dir, cloned.base_dir);
}

// ── Error conversions ─────────────────────────────────────────────────────────

#[test]
fn test_server_error_display() {
    let e = ServerError::ConfigError("bad config".to_string());
    assert!(e.to_string().contains("bad config"));

    let e = ServerError::MemoryError("oom".to_string());
    assert!(e.to_string().contains("oom"));

    let e = ServerError::DatabaseError("locked".to_string());
    assert!(e.to_string().contains("locked"));

    let e = ServerError::NotFound("fact 123".to_string());
    assert!(e.to_string().contains("fact 123"));

    let e = ServerError::InvalidArgument("bad arg".to_string());
    assert!(e.to_string().contains("bad arg"));
}

#[test]
fn test_embedding_error_into_server_error() {
    let e: ServerError = EmbeddingError::UnsupportedModel("x".to_string()).into();
    assert!(e.to_string().contains("x"));

    let e: ServerError = EmbeddingError::LoadError("fail".to_string()).into();
    assert!(e.to_string().contains("fail"));

    let e: ServerError = EmbeddingError::GenerationError("bad".to_string()).into();
    assert!(e.to_string().contains("bad"));
}

#[test]
fn test_io_error_into_server_error() {
    let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file missing");
    let e: ServerError = io_err.into();
    assert!(e.to_string().contains("file missing"));
}

#[test]
fn test_rusqlite_error_into_server_error() {
    let sqlite_err = rusqlite::Error::InvalidQuery;
    let e: ServerError = sqlite_err.into();
    assert!(e.to_string().contains("Query is not read-only"));
}
