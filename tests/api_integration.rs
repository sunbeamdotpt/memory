use actix_web::{test, web, App, http::StatusCode};
use mcp_server::api::config::configure_api;
use mcp_server::api::mcp_http::{AuthConfig, build_mcp_service};
use mcp_server::indexer::{IndexService, IndexWatcher};
use mcp_server::memory::service::MemoryService;
use mcp_server::config::MemoryConfig;
use serde_json::json;

macro_rules! setup_app {
    () => {{
        let dir = tempfile::tempdir().unwrap();
        let config = MemoryConfig {
            base_dir: dir.path().to_str().unwrap().to_string(),
            ..Default::default()
        };
        let memory = MemoryService::new(&config).await.unwrap();
        let (dummy_tx, dummy_rx) = crossbeam_channel::bounded(1);
        let watcher = IndexWatcher::new(dummy_tx).unwrap();
        let indexer = IndexService::new(memory.clone(), dummy_rx, watcher);
        let mcp_service = build_mcp_service(memory.clone(), indexer.clone());
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(memory))
                .app_data(web::Data::new(indexer))
                .app_data(web::Data::new(AuthConfig::LocalOnly))
                .configure(configure_api(mcp_service)),
        ).await;
        (app, dir)
    }};
}

macro_rules! setup_app_with_auth {
    ($token:expr) => {{
        let dir = tempfile::tempdir().unwrap();
        let config = MemoryConfig {
            base_dir: dir.path().to_str().unwrap().to_string(),
            auth_token: Some($token.to_string()),
            ..Default::default()
        };
        let memory = MemoryService::new(&config).await.unwrap();
        let (dummy_tx, dummy_rx) = crossbeam_channel::bounded(1);
        let watcher = IndexWatcher::new(dummy_tx).unwrap();
        let indexer = IndexService::new(memory.clone(), dummy_rx, watcher);
        let mcp_service = build_mcp_service(memory.clone(), indexer.clone());
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(memory))
                .app_data(web::Data::new(indexer))
                .app_data(web::Data::new(AuthConfig::Bearer($token.to_string())))
                .configure(configure_api(mcp_service)),
        ).await;
        (app, dir)
    }};
}

// ── SSE helpers ───────────────────────────────────────────────────────────────

async fn sse_events<B>(resp: actix_web::dev::ServiceResponse<B>) -> Vec<serde_json::Value>
where
    B: actix_web::body::MessageBody,
{
    let bytes = test::read_body(resp).await;
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    text.lines()
        .filter(|l| l.starts_with("data: "))
        .map(|l| {
            let json_str = l.strip_prefix("data: ").unwrap();
            serde_json::from_str(json_str).unwrap_or(serde_json::Value::Null)
        })
        .collect()
}

macro_rules! mcp_init {
    ($app:expr) => {{
        let req = test::TestRequest::post()
            .uri("/mcp")
            .insert_header(("Accept", "application/json, text/event-stream"))
            .insert_header(("Content-Type", "application/json"))
            .set_json(json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "clientInfo": { "name": "test", "version": "1.0" }
                }
            }))
            .to_request();
        let resp = test::call_service($app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
        resp.headers()
            .get("Mcp-Session-Id")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
    }};
}

// ── REST API tests ────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_health_check() {
    let (app, _dir) = setup_app!();
    let req = test::TestRequest::get().uri("/api/health").to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body["status"], "healthy");
    assert!(!body["version"].as_str().unwrap().is_empty());
}

#[tokio::test]
async fn test_store_fact() {
    let (app, _dir) = setup_app!();
    let req = test::TestRequest::post()
        .uri("/api/facts")
        .set_json(json!({ "content": "Hello world" }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert!(!body["id"].as_str().unwrap().is_empty());
    assert_eq!(body["namespace"], "default");
    assert_eq!(body["content"], "Hello world");
}

#[tokio::test]
async fn test_store_fact_with_namespace_and_source() {
    let (app, _dir) = setup_app!();
    let req = test::TestRequest::post()
        .uri("/api/facts")
        .set_json(json!({
            "namespace": "docs",
            "content": "Rust book chapter 1",
            "source": "urn:smem:doc:fs:/home/user/book.md"
        }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body["namespace"], "docs");
    assert_eq!(body["source"], "urn:smem:doc:fs:/home/user/book.md");
}

#[tokio::test]
async fn test_search_facts() {
    let (app, _dir) = setup_app!();

    let store_req = test::TestRequest::post()
        .uri("/api/facts")
        .set_json(json!({ "content": "The quick brown fox" }))
        .to_request();
    test::call_service(&app, store_req).await;

    let req = test::TestRequest::get()
        .uri("/api/facts/search?q=fox&limit=5")
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert!(body["total"].as_u64().unwrap() > 0);
    assert!(!body["results"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn test_search_facts_empty_query() {
    let (app, _dir) = setup_app!();
    let req = test::TestRequest::get()
        .uri("/api/facts/search?q=")
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_search_facts_limit_zero() {
    let (app, _dir) = setup_app!();
    let req = test::TestRequest::get()
        .uri("/api/facts/search?q=hello&limit=0")
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_search_facts_limit_clamped() {
    let (app, _dir) = setup_app!();
    let req = test::TestRequest::get()
        .uri("/api/facts/search?q=hello&limit=2000")
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_list_facts() {
    let (app, _dir) = setup_app!();

    let store_req = test::TestRequest::post()
        .uri("/api/facts")
        .set_json(json!({ "namespace": "testns", "content": "test content" }))
        .to_request();
    test::call_service(&app, store_req).await;

    let req = test::TestRequest::get()
        .uri("/api/facts/testns?limit=10")
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = test::read_body_json(resp).await;
    assert!(body["total"].as_u64().unwrap() > 0);
}

#[tokio::test]
async fn test_list_facts_limit_zero() {
    let (app, _dir) = setup_app!();
    let req = test::TestRequest::get()
        .uri("/api/facts/testns?limit=0")
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_delete_fact() {
    let (app, _dir) = setup_app!();

    let store_req = test::TestRequest::post()
        .uri("/api/facts")
        .set_json(json!({ "content": "to be deleted" }))
        .to_request();
    let store_resp = test::call_service(&app, store_req).await;
    let body: serde_json::Value = test::read_body_json(store_resp).await;
    let id = body["id"].as_str().unwrap();

    let req = test::TestRequest::delete()
        .uri(&format!("/api/facts/{id}"))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let req2 = test::TestRequest::delete()
        .uri(&format!("/api/facts/{id}"))
        .to_request();
    let resp2 = test::call_service(&app, req2).await;
    assert_eq!(resp2.status(), StatusCode::NOT_FOUND);
}

// ── MCP HTTP transport tests ──────────────────────────────────────────────────

#[tokio::test]
async fn test_mcp_post_initialize() {
    let (app, _dir) = setup_app!();
    let sid = mcp_init!(&app);
    assert!(sid.is_some());
}

#[tokio::test]
async fn test_mcp_post_tools_list() {
    let (app, _dir) = setup_app!();
    let sid = mcp_init!(&app).unwrap();

    let req = test::TestRequest::post()
        .uri("/mcp")
        .insert_header(("Accept", "application/json, text/event-stream"))
        .insert_header(("Content-Type", "application/json"))
        .insert_header(("Mcp-Session-Id", sid))
        .set_json(json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let events = sse_events(resp).await;
    assert!(!events.is_empty());
    assert!(events[0]["result"]["tools"].is_array());
}

#[tokio::test]
async fn test_mcp_post_tools_call_store_fact() {
    let (app, _dir) = setup_app!();
    let sid = mcp_init!(&app).unwrap();

    let req = test::TestRequest::post()
        .uri("/mcp")
        .insert_header(("Accept", "application/json, text/event-stream"))
        .insert_header(("Content-Type", "application/json"))
        .insert_header(("Mcp-Session-Id", sid))
        .set_json(json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "store_fact",
                "arguments": { "content": "hello from MCP HTTP" }
            }
        }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let events = sse_events(resp).await;
    assert!(!events.is_empty());
    let text = events[0]["result"]["content"][0]["text"].as_str().unwrap_or("");
    assert!(text.contains("Stored."));
}

// ── Auth / origin tests ───────────────────────────────────────────────────────

#[tokio::test]
async fn test_mcp_post_local_origin_allowed() {
    let (app, _dir) = setup_app!();
    let req = test::TestRequest::post()
        .uri("/mcp")
        .insert_header(("Origin", "http://localhost:3000"))
        .insert_header(("Accept", "application/json, text/event-stream"))
        .insert_header(("Content-Type", "application/json"))
        .set_json(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": { "name": "test", "version": "1.0" }
            }
        }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_mcp_post_local_origin_127_allowed() {
    let (app, _dir) = setup_app!();
    let req = test::TestRequest::post()
        .uri("/mcp")
        .insert_header(("Origin", "http://127.0.0.1:3000"))
        .insert_header(("Accept", "application/json, text/event-stream"))
        .insert_header(("Content-Type", "application/json"))
        .set_json(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": { "name": "test", "version": "1.0" }
            }
        }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_mcp_post_origin_rejected() {
    let (app, _dir) = setup_app!();
    let req = test::TestRequest::post()
        .uri("/mcp")
        .insert_header(("Origin", "http://evil.com"))
        .insert_header(("Accept", "application/json, text/event-stream"))
        .insert_header(("Content-Type", "application/json"))
        .set_json(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": { "name": "test", "version": "1.0" }
            }
        }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_mcp_post_bearer_auth_valid() {
    let (app, _dir) = setup_app_with_auth!("secret-token");
    let req = test::TestRequest::post()
        .uri("/mcp")
        .insert_header(("Authorization", "Bearer secret-token"))
        .insert_header(("Accept", "application/json, text/event-stream"))
        .insert_header(("Content-Type", "application/json"))
        .set_json(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": { "name": "test", "version": "1.0" }
            }
        }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_mcp_post_bearer_auth_invalid() {
    let (app, _dir) = setup_app_with_auth!("secret-token");
    let req = test::TestRequest::post()
        .uri("/mcp")
        .insert_header(("Authorization", "Bearer wrong-token"))
        .insert_header(("Accept", "application/json, text/event-stream"))
        .insert_header(("Content-Type", "application/json"))
        .set_json(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": { "name": "test", "version": "1.0" }
            }
        }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_mcp_post_bearer_auth_missing() {
    let (app, _dir) = setup_app_with_auth!("secret-token");
    let req = test::TestRequest::post()
        .uri("/mcp")
        .insert_header(("Accept", "application/json, text/event-stream"))
        .insert_header(("Content-Type", "application/json"))
        .set_json(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": { "name": "test", "version": "1.0" }
            }
        }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_mcp_post_no_origin_header_allowed() {
    let (app, _dir) = setup_app!();
    let req = test::TestRequest::post()
        .uri("/mcp")
        .insert_header(("Accept", "application/json, text/event-stream"))
        .insert_header(("Content-Type", "application/json"))
        .set_json(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": { "name": "test", "version": "1.0" }
            }
        }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_mcp_post_bearer_empty_token() {
    let dir = tempfile::tempdir().unwrap();
    let config = MemoryConfig {
        base_dir: dir.path().to_str().unwrap().to_string(),
        auth_token: Some("".to_string()),
        ..Default::default()
    };
    let memory = MemoryService::new(&config).await.unwrap();
    let (dummy_tx, dummy_rx) = crossbeam_channel::bounded(1);
    let watcher = IndexWatcher::new(dummy_tx).unwrap();
    let indexer = IndexService::new(memory.clone(), dummy_rx, watcher);
    let mcp_service = build_mcp_service(memory.clone(), indexer);
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(memory))
            .app_data(web::Data::new(AuthConfig::Bearer("".to_string())))
            .configure(configure_api(mcp_service)),
    ).await;

    let req = test::TestRequest::post()
        .uri("/mcp")
        .insert_header(("Authorization", "Bearer "))
        .insert_header(("Accept", "application/json, text/event-stream"))
        .insert_header(("Content-Type", "application/json"))
        .set_json(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": { "name": "test", "version": "1.0" }
            }
        }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_mcp_post_oidc_missing_token() {
    use mcp_server::api::oidc::OidcVerifier;
    use jsonwebtoken::jwk::JwkSet;

    let dir = tempfile::tempdir().unwrap();
    let config = MemoryConfig {
        base_dir: dir.path().to_str().unwrap().to_string(),
        ..Default::default()
    };
    let memory = MemoryService::new(&config).await.unwrap();
    let (dummy_tx, dummy_rx) = crossbeam_channel::bounded(1);
    let watcher = IndexWatcher::new(dummy_tx).unwrap();
    let indexer = IndexService::new(memory.clone(), dummy_rx, watcher);
    let mcp_service = build_mcp_service(memory.clone(), indexer);
    let verifier = OidcVerifier::test_new("https://example.com", None, JwkSet { keys: vec![] });
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(memory))
            .app_data(web::Data::new(AuthConfig::Oidc(std::sync::Mutex::new(verifier))))
            .configure(configure_api(mcp_service)),
    ).await;

    let req = test::TestRequest::post()
        .uri("/mcp")
        .insert_header(("Accept", "application/json, text/event-stream"))
        .insert_header(("Content-Type", "application/json"))
        .set_json(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": { "name": "test", "version": "1.0" }
            }
        }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_mcp_post_oidc_invalid_token() {
    use mcp_server::api::oidc::OidcVerifier;
    use jsonwebtoken::jwk::{JwkSet, Jwk, AlgorithmParameters, RSAKeyParameters, CommonParameters, PublicKeyUse, KeyAlgorithm, RSAKeyType};

    let dir = tempfile::tempdir().unwrap();
    let config = MemoryConfig {
        base_dir: dir.path().to_str().unwrap().to_string(),
        ..Default::default()
    };
    let memory = MemoryService::new(&config).await.unwrap();
    let (dummy_tx, dummy_rx) = crossbeam_channel::bounded(1);
    let watcher = IndexWatcher::new(dummy_tx).unwrap();
    let indexer = IndexService::new(memory.clone(), dummy_rx, watcher);
    let mcp_service = build_mcp_service(memory.clone(), indexer);
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
        algorithm: AlgorithmParameters::RSA(RSAKeyParameters {
            key_type: RSAKeyType::RSA,
            n: "test".to_string(),
            e: "AQAB".to_string(),
        }),
    };
    let verifier = OidcVerifier::test_new("https://example.com", None, JwkSet { keys: vec![jwk] });
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(memory))
            .app_data(web::Data::new(AuthConfig::Oidc(std::sync::Mutex::new(verifier))))
            .configure(configure_api(mcp_service)),
    ).await;

    let req = test::TestRequest::post()
        .uri("/mcp")
        .insert_header(("Authorization", "Bearer invalid.token.here"))
        .insert_header(("Accept", "application/json, text/event-stream"))
        .insert_header(("Content-Type", "application/json"))
        .set_json(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": { "name": "test", "version": "1.0" }
            }
        }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}
