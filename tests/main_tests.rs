use mcp_server::{
    config::MemoryConfig,
    memory::service::MemoryService,
    mcp::server::process_mcp_line,
    api::mcp_http::AuthConfig,
    indexer::{IndexService, IndexWatcher},
};

async fn setup_memory() -> (MemoryService, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let config = MemoryConfig {
        base_dir: dir.path().to_str().unwrap().to_string(),
        ..Default::default()
    };
    let memory = MemoryService::new(&config).await.unwrap();
    (memory, dir)
}

fn setup_indexer(memory: &MemoryService) -> IndexService {
    let (tx, rx) = crossbeam_channel::bounded(1);
    let watcher = IndexWatcher::new(tx).unwrap();
    IndexService::new(memory.clone(), rx, watcher)
}

// ── process_mcp_line tests ────────────────────────────────────────────────────

#[tokio::test]
async fn test_process_mcp_line_initialize() {
    let (memory, _dir) = setup_memory().await;
    let indexer = setup_indexer(&memory);
    let line = r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#;
    let result = process_mcp_line(line, &memory, &indexer).await;
    assert!(result.is_some());
    let resp: serde_json::Value = serde_json::from_str(&result.unwrap()).unwrap();
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 1);
    assert_eq!(resp["result"]["protocolVersion"], "2025-06-18");
}

#[tokio::test]
async fn test_process_mcp_line_invalid_json() {
    let (memory, _dir) = setup_memory().await;
    let indexer = setup_indexer(&memory);
    let line = "not json at all";
    let result = process_mcp_line(line, &memory, &indexer).await;
    assert!(result.is_some());
    let resp: serde_json::Value = serde_json::from_str(&result.unwrap()).unwrap();
    assert!(resp["error"].is_object());
    assert_eq!(resp["error"]["code"], -32700);
}

#[tokio::test]
async fn test_process_mcp_line_notification() {
    let (memory, _dir) = setup_memory().await;
    let indexer = setup_indexer(&memory);
    let line = r#"{"jsonrpc":"2.0","method":"$/progress"}"#;
    let result = process_mcp_line(line, &memory, &indexer).await;
    assert!(result.is_none());
}

#[tokio::test]
async fn test_process_mcp_line_tools_call() {
    let (memory, _dir) = setup_memory().await;
    let indexer = setup_indexer(&memory);
    let line = r#"{"jsonrpc":"2.0","id":42,"method":"tools/call","params":{"name":"store_fact","arguments":{"content":"hello"}}}"#;
    let result = process_mcp_line(line, &memory, &indexer).await;
    assert!(result.is_some());
    let resp: serde_json::Value = serde_json::from_str(&result.unwrap()).unwrap();
    assert_eq!(resp["id"], 42);
    assert!(!resp["result"]["isError"].as_bool().unwrap_or(true));
}

#[tokio::test]
async fn test_process_mcp_line_empty_line() {
    let (memory, _dir) = setup_memory().await;
    let indexer = setup_indexer(&memory);
    // Empty string is invalid JSON, so it returns an error response
    let result = process_mcp_line("", &memory, &indexer).await;
    assert!(result.is_some());
    let resp: serde_json::Value = serde_json::from_str(&result.unwrap()).unwrap();
    assert!(resp["error"].is_object());
}

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
