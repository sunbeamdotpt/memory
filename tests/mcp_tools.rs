use mcp_server::{
    config::MemoryConfig,
    indexer::{IndexService, IndexWatcher},
    memory::service::MemoryService,
    mcp::{protocol::Request, server::handle},
};
use serde_json::{json, Value};

fn req(method: &str, params: Value, id: u64) -> Request {
    serde_json::from_value(json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    }))
    .expect("valid request JSON")
}

async fn setup() -> (MemoryService, IndexService, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let config = MemoryConfig {
        base_dir: dir.path().to_str().unwrap().to_string(),
        ..Default::default()
    };
    let memory = MemoryService::new(&config).await.unwrap();
    let (dummy_tx, dummy_rx) = crossbeam_channel::bounded(1);
    let watcher = IndexWatcher::new(dummy_tx).unwrap();
    let indexer = IndexService::new(memory.clone(), dummy_rx, watcher);
    (memory, indexer, dir)
}

// ── lifecycle methods ─────────────────────────────────────────────────────────

#[tokio::test]
async fn test_initialize() {
    let (memory, indexer, _dir) = setup().await;
    let r = req("initialize", json!({}), 1);
    let resp = handle(&r, &memory, &indexer).await.unwrap();
    assert!(resp.error.is_none());
    let result = resp.result.unwrap();
    assert_eq!(result["protocolVersion"], "2025-06-18");
    assert_eq!(result["serverInfo"]["name"], "sunbeam-memory");
}

#[tokio::test]
async fn test_ping() {
    let (memory, indexer, _dir) = setup().await;
    let r = req("ping", json!({}), 1);
    let resp = handle(&r, &memory, &indexer).await.unwrap();
    assert!(resp.error.is_none());
    assert_eq!(resp.result.unwrap(), json!({}));
}

#[tokio::test]
async fn test_tools_list() {
    let (memory, indexer, _dir) = setup().await;
    let r = req("tools/list", json!({}), 1);
    let resp = handle(&r, &memory, &indexer).await.unwrap();
    assert!(resp.error.is_none());
    let result = resp.result.unwrap();
    let tools = result["tools"].as_array().unwrap().clone();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"store_fact"));
    assert!(names.contains(&"search_facts"));
    assert!(names.contains(&"update_fact"));
    assert!(names.contains(&"delete_fact"));
    assert!(names.contains(&"list_facts"));
    assert!(names.contains(&"build_source_urn"));
    assert!(names.contains(&"parse_source_urn"));
    assert!(names.contains(&"describe_urn_schema"));
}

#[tokio::test]
async fn test_unknown_method() {
    let (memory, indexer, _dir) = setup().await;
    let r = req("unknown/method", json!({}), 1);
    let resp = handle(&r, &memory, &indexer).await.unwrap();
    assert!(resp.error.is_some());
    assert_eq!(resp.error.unwrap().code, -32601);
}

#[tokio::test]
async fn test_notification_returns_none() {
    let (memory, indexer, _dir) = setup().await;
    let r = serde_json::from_value::<Request>(json!({
        "jsonrpc": "2.0",
        "method": "$/progress",
        "params": {}
    })).unwrap();
    let resp = handle(&r, &memory, &indexer).await;
    assert!(resp.is_none());
}

// ── store_fact ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_store_fact_success() {
    let (memory, indexer, _dir) = setup().await;
    let r = req("tools/call", json!({
        "name": "store_fact",
        "arguments": { "content": "Hello world" }
    }), 1);
    let resp = handle(&r, &memory, &indexer).await.unwrap();
    assert!(resp.error.is_none());
    let result = resp.result.unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("Stored."));
    assert!(text.contains("ID:"));
}

#[tokio::test]
async fn test_store_fact_with_namespace() {
    let (memory, indexer, _dir) = setup().await;
    let r = req("tools/call", json!({
        "name": "store_fact",
        "arguments": { "namespace": "code", "content": "fn main() {}" }
    }), 1);
    let resp = handle(&r, &memory, &indexer).await.unwrap();
    let result = resp.result.unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("Namespace: code"));
}

#[tokio::test]
async fn test_store_fact_with_source() {
    let (memory, indexer, _dir) = setup().await;
    let r = req("tools/call", json!({
        "name": "store_fact",
        "arguments": {
            "content": "Rust code",
            "source": "urn:smem:code:fs:/home/user/main.rs"
        }
    }), 1);
    let resp = handle(&r, &memory, &indexer).await.unwrap();
    let result = resp.result.unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("Source:"));
    assert!(text.contains("local file"));
}

#[tokio::test]
async fn test_store_fact_empty_content() {
    let (memory, indexer, _dir) = setup().await;
    let r = req("tools/call", json!({
        "name": "store_fact",
        "arguments": { "content": "" }
    }), 1);
    let resp = handle(&r, &memory, &indexer).await.unwrap();
    let result = resp.result.unwrap();
    assert!(result["isError"].as_bool().unwrap());
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("required"));
}

#[tokio::test]
async fn test_store_fact_invalid_urn() {
    let (memory, indexer, _dir) = setup().await;
    let r = req("tools/call", json!({
        "name": "store_fact",
        "arguments": {
            "content": "test",
            "source": "not-a-urn"
        }
    }), 1);
    let resp = handle(&r, &memory, &indexer).await.unwrap();
    let result = resp.result.unwrap();
    assert!(result["isError"].as_bool().unwrap());
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("Invalid source URN"));
}

// ── search_facts ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_search_facts_success() {
    let (memory, indexer, _dir) = setup().await;

    // Store a fact
    let store = req("tools/call", json!({
        "name": "store_fact",
        "arguments": { "content": "The quick brown fox jumps over the lazy dog" }
    }), 1);
    handle(&store, &memory, &indexer).await.unwrap();

    // Search
    let r = req("tools/call", json!({
        "name": "search_facts",
        "arguments": { "query": "fox animal", "limit": 5 }
    }), 2);
    let resp = handle(&r, &memory, &indexer).await.unwrap();
    let result = resp.result.unwrap();
    assert!(!result["isError"].as_bool().unwrap());
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("Found"));
    assert!(text.contains("fox"));
}

#[tokio::test]
async fn test_search_facts_empty_query() {
    let (memory, indexer, _dir) = setup().await;
    let r = req("tools/call", json!({
        "name": "search_facts",
        "arguments": { "query": "" }
    }), 1);
    let resp = handle(&r, &memory, &indexer).await.unwrap();
    let result = resp.result.unwrap();
    assert!(result["isError"].as_bool().unwrap());
}

#[tokio::test]
async fn test_search_facts_no_results() {
    let (memory, indexer, _dir) = setup().await;
    let r = req("tools/call", json!({
        "name": "search_facts",
        "arguments": { "query": "quantum chromodynamics" }
    }), 1);
    let resp = handle(&r, &memory, &indexer).await.unwrap();
    let result = resp.result.unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    assert_eq!(text, "No results found.");
}

#[tokio::test]
async fn test_search_facts_with_namespace() {
    let (memory, indexer, _dir) = setup().await;

    let store = req("tools/call", json!({
        "name": "store_fact",
        "arguments": { "namespace": "animals", "content": "elephants are big" }
    }), 1);
    handle(&store, &memory, &indexer).await.unwrap();

    let r = req("tools/call", json!({
        "name": "search_facts",
        "arguments": { "query": "elephants", "namespace": "animals" }
    }), 2);
    let resp = handle(&r, &memory, &indexer).await.unwrap();
    let result = resp.result.unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("Found"));
}

// ── update_fact ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_update_fact_success() {
    let (memory, indexer, _dir) = setup().await;

    // Store
    let store = req("tools/call", json!({
        "name": "store_fact",
        "arguments": { "content": "original" }
    }), 1);
    let resp = handle(&store, &memory, &indexer).await.unwrap();
    let result = resp.result.unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    let id = text.lines().find(|l| l.starts_with("ID:")).unwrap()
        .strip_prefix("ID: ").unwrap();

    // Update
    let r = req("tools/call", json!({
        "name": "update_fact",
        "arguments": { "id": id, "content": "updated" }
    }), 2);
    let resp = handle(&r, &memory, &indexer).await.unwrap();
    let result = resp.result.unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("Updated."));
    assert!(text.contains(id));
}

#[tokio::test]
async fn test_update_fact_missing_id() {
    let (memory, indexer, _dir) = setup().await;
    let r = req("tools/call", json!({
        "name": "update_fact",
        "arguments": { "id": "", "content": "updated" }
    }), 1);
    let resp = handle(&r, &memory, &indexer).await.unwrap();
    let result = resp.result.unwrap();
    assert!(result["isError"].as_bool().unwrap());
}

#[tokio::test]
async fn test_update_fact_empty_content() {
    let (memory, indexer, _dir) = setup().await;
    let r = req("tools/call", json!({
        "name": "update_fact",
        "arguments": { "id": "some-id", "content": "" }
    }), 1);
    let resp = handle(&r, &memory, &indexer).await.unwrap();
    let result = resp.result.unwrap();
    assert!(result["isError"].as_bool().unwrap());
}

#[tokio::test]
async fn test_update_fact_invalid_urn() {
    let (memory, indexer, _dir) = setup().await;

    let store = req("tools/call", json!({
        "name": "store_fact",
        "arguments": { "content": "original" }
    }), 1);
    let resp = handle(&store, &memory, &indexer).await.unwrap();
    let result = resp.result.unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    let id = text.lines().find(|l| l.starts_with("ID:")).unwrap()
        .strip_prefix("ID: ").unwrap();

    let r = req("tools/call", json!({
        "name": "update_fact",
        "arguments": { "id": id, "content": "updated", "source": "bad-urn" }
    }), 2);
    let resp = handle(&r, &memory, &indexer).await.unwrap();
    let result = resp.result.unwrap();
    assert!(result["isError"].as_bool().unwrap());
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("Invalid source URN"));
}

#[tokio::test]
async fn test_update_fact_not_found() {
    let (memory, indexer, _dir) = setup().await;
    let r = req("tools/call", json!({
        "name": "update_fact",
        "arguments": { "id": "non-existent-id", "content": "updated" }
    }), 1);
    let resp = handle(&r, &memory, &indexer).await.unwrap();
    let result = resp.result.unwrap();
    assert!(result["isError"].as_bool().unwrap());
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("Update failed"));
}

// ── delete_fact ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_delete_fact_success() {
    let (memory, indexer, _dir) = setup().await;

    let store = req("tools/call", json!({
        "name": "store_fact",
        "arguments": { "content": "to delete" }
    }), 1);
    let resp = handle(&store, &memory, &indexer).await.unwrap();
    let result = resp.result.unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    let id = text.lines().find(|l| l.starts_with("ID:")).unwrap()
        .strip_prefix("ID: ").unwrap();

    let r = req("tools/call", json!({
        "name": "delete_fact",
        "arguments": { "id": id }
    }), 2);
    let resp = handle(&r, &memory, &indexer).await.unwrap();
    let result = resp.result.unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("Deleted"));
}

#[tokio::test]
async fn test_delete_fact_not_found() {
    let (memory, indexer, _dir) = setup().await;
    let r = req("tools/call", json!({
        "name": "delete_fact",
        "arguments": { "id": "non-existent" }
    }), 1);
    let resp = handle(&r, &memory, &indexer).await.unwrap();
    let result = resp.result.unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("not found"));
}

#[tokio::test]
async fn test_delete_fact_missing_id() {
    let (memory, indexer, _dir) = setup().await;
    let r = req("tools/call", json!({
        "name": "delete_fact",
        "arguments": { "id": "" }
    }), 1);
    let resp = handle(&r, &memory, &indexer).await.unwrap();
    let result = resp.result.unwrap();
    assert!(result["isError"].as_bool().unwrap());
}

// ── list_facts ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_list_facts_empty() {
    let (memory, indexer, _dir) = setup().await;
    let r = req("tools/call", json!({
        "name": "list_facts",
        "arguments": { "namespace": "empty" }
    }), 1);
    let resp = handle(&r, &memory, &indexer).await.unwrap();
    let result = resp.result.unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("No facts"));
}

#[tokio::test]
async fn test_list_facts_with_results() {
    let (memory, indexer, _dir) = setup().await;

    for i in 0..3 {
        let store = req("tools/call", json!({
            "name": "store_fact",
            "arguments": { "namespace": "docs", "content": format!("doc {}", i) }
        }), i);
        handle(&store, &memory, &indexer).await.unwrap();
    }

    let r = req("tools/call", json!({
        "name": "list_facts",
        "arguments": { "namespace": "docs", "limit": 10 }
    }), 100);
    let resp = handle(&r, &memory, &indexer).await.unwrap();
    let result = resp.result.unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("3 fact(s)"));
    assert!(text.contains("doc 0"));
}

#[tokio::test]
async fn test_list_facts_with_date_range() {
    let (memory, indexer, _dir) = setup().await;

    let store = req("tools/call", json!({
        "name": "store_fact",
        "arguments": { "namespace": "dated", "content": "old fact" }
    }), 1);
    handle(&store, &memory, &indexer).await.unwrap();

    // Query with a future date range → no results
    let r = req("tools/call", json!({
        "name": "list_facts",
        "arguments": {
            "namespace": "dated",
            "from": "2099-01-01T00:00:00Z",
            "to": "2099-12-31T23:59:59Z"
        }
    }), 2);
    let resp = handle(&r, &memory, &indexer).await.unwrap();
    let result = resp.result.unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("No facts"));
}

// ── build_source_urn ──────────────────────────────────────────────────────────

#[tokio::test]
async fn test_build_source_urn_success() {
    let (memory, indexer, _dir) = setup().await;
    let r = req("tools/call", json!({
        "name": "build_source_urn",
        "arguments": {
            "content_type": "code",
            "origin": "fs",
            "locator": "/home/user/main.rs",
            "fragment": "L10-L30"
        }
    }), 1);
    let resp = handle(&r, &memory, &indexer).await.unwrap();
    let result = resp.result.unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    assert_eq!(text, "urn:smem:code:fs:/home/user/main.rs#L10-L30");
}

#[tokio::test]
async fn test_build_source_urn_missing_content_type() {
    let (memory, indexer, _dir) = setup().await;
    let r = req("tools/call", json!({
        "name": "build_source_urn",
        "arguments": { "origin": "fs", "locator": "/foo" }
    }), 1);
    let resp = handle(&r, &memory, &indexer).await.unwrap();
    let result = resp.result.unwrap();
    assert!(result["isError"].as_bool().unwrap());
}

#[tokio::test]
async fn test_build_source_urn_invalid_type() {
    let (memory, indexer, _dir) = setup().await;
    let r = req("tools/call", json!({
        "name": "build_source_urn",
        "arguments": { "content_type": "blob", "origin": "fs", "locator": "/foo" }
    }), 1);
    let resp = handle(&r, &memory, &indexer).await.unwrap();
    let result = resp.result.unwrap();
    assert!(result["isError"].as_bool().unwrap());
}

#[tokio::test]
async fn test_build_source_urn_empty_origin() {
    let (memory, indexer, _dir) = setup().await;
    let r = req("tools/call", json!({
        "name": "build_source_urn",
        "arguments": { "content_type": "code", "origin": "", "locator": "/foo" }
    }), 1);
    let resp = handle(&r, &memory, &indexer).await.unwrap();
    let result = resp.result.unwrap();
    assert!(result["isError"].as_bool().unwrap());
}

#[tokio::test]
async fn test_build_source_urn_empty_locator() {
    let (memory, indexer, _dir) = setup().await;
    let r = req("tools/call", json!({
        "name": "build_source_urn",
        "arguments": { "content_type": "code", "origin": "fs", "locator": "" }
    }), 1);
    let resp = handle(&r, &memory, &indexer).await.unwrap();
    let result = resp.result.unwrap();
    assert!(result["isError"].as_bool().unwrap());
}

// ── parse_source_urn ──────────────────────────────────────────────────────────

#[tokio::test]
async fn test_parse_source_urn_success() {
    let (memory, indexer, _dir) = setup().await;
    let r = req("tools/call", json!({
        "name": "parse_source_urn",
        "arguments": { "urn": "urn:smem:code:fs:/home/user/main.rs#L10" }
    }), 1);
    let resp = handle(&r, &memory, &indexer).await.unwrap();
    let result = resp.result.unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("\"valid\": true"));
    assert!(text.contains("\"content_type\": \"code\""));
}

#[tokio::test]
async fn test_parse_source_urn_invalid() {
    let (memory, indexer, _dir) = setup().await;
    let r = req("tools/call", json!({
        "name": "parse_source_urn",
        "arguments": { "urn": "not-a-urn" }
    }), 1);
    let resp = handle(&r, &memory, &indexer).await.unwrap();
    let result = resp.result.unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("\"valid\": false"));
}

#[tokio::test]
async fn test_parse_source_urn_missing() {
    let (memory, indexer, _dir) = setup().await;
    let r = req("tools/call", json!({
        "name": "parse_source_urn",
        "arguments": {}
    }), 1);
    let resp = handle(&r, &memory, &indexer).await.unwrap();
    let result = resp.result.unwrap();
    assert!(result["isError"].as_bool().unwrap());
}

#[test]
fn test_parse_ts_unix_timestamp() {
    use mcp_server::mcp::server::parse_ts;
    let result = parse_ts("1609459200");
    assert_eq!(result, Some(1609459200));
}

#[test]
fn test_parse_ts_rfc3339() {
    use mcp_server::mcp::server::parse_ts;
    let result = parse_ts("2021-01-01T00:00:00Z");
    assert_eq!(result, Some(1609459200));
}

#[test]
fn test_parse_ts_invalid() {
    use mcp_server::mcp::server::parse_ts;
    let result = parse_ts("not-a-date");
    assert_eq!(result, None);
}

// ── describe_urn_schema ───────────────────────────────────────────────────────

#[tokio::test]
async fn test_describe_urn_schema() {
    let (memory, indexer, _dir) = setup().await;
    let r = req("tools/call", json!({
        "name": "describe_urn_schema",
        "arguments": {}
    }), 1);
    let resp = handle(&r, &memory, &indexer).await.unwrap();
    let result = resp.result.unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("format"));
    assert!(text.contains("content_types"));
    assert!(text.contains("origins"));
}

// ── unknown tool ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_unknown_tool() {
    let (memory, indexer, _dir) = setup().await;
    let r = req("tools/call", json!({
        "name": "unknown_tool",
        "arguments": {}
    }), 1);
    let resp = handle(&r, &memory, &indexer).await.unwrap();
    let result = resp.result.unwrap();
    assert!(result["isError"].as_bool().unwrap());
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("Unknown tool"));
}
