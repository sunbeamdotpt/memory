use mcp_server::{
    config::MemoryConfig,
    indexer::{IndexService, IndexWatcher},
    memory::service::MemoryService,
    mcp::server::SunbeamServer,
    mcp::{
        StoreFactParams, SearchFactsParams, UpdateFactParams, DeleteFactParams,
        ListFactsParams, BuildSourceUrnParams, ParseSourceUrnParams,
    },
};
use rmcp::handler::server::wrapper::Parameters;

async fn setup() -> (SunbeamServer, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let config = MemoryConfig {
        base_dir: dir.path().to_str().unwrap().to_string(),
        ..Default::default()
    };
    let memory = MemoryService::new(&config).await.unwrap();
    let (dummy_tx, dummy_rx) = crossbeam_channel::bounded(1);
    let watcher = IndexWatcher::new(dummy_tx).unwrap();
    let indexer = IndexService::new(memory.clone(), dummy_rx, watcher);
    let server = SunbeamServer::new(memory, indexer);
    (server, dir)
}

fn tool_text(result: &rmcp::model::CallToolResult) -> String {
    result.content[0].as_text().map(|t| t.text.clone()).unwrap_or_default()
}

fn is_error(result: &rmcp::model::CallToolResult) -> bool {
    result.is_error == Some(true)
}

// ── store_fact ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_store_fact_success() {
    let (server, _dir) = setup().await;
    let result = server.store_fact(Parameters(StoreFactParams {
        content: "Hello world".to_string(),
        namespace: None,
        source: None,
    })).await.unwrap();
    assert!(!is_error(&result));
    let text = tool_text(&result);
    assert!(text.contains("Stored."));
    assert!(text.contains("ID:"));
}

#[tokio::test]
async fn test_store_fact_with_namespace() {
    let (server, _dir) = setup().await;
    let result = server.store_fact(Parameters(StoreFactParams {
        content: "fn main() {}".to_string(),
        namespace: Some("code".to_string()),
        source: None,
    })).await.unwrap();
    let text = tool_text(&result);
    assert!(text.contains("Namespace: code"));
}

#[tokio::test]
async fn test_store_fact_with_source() {
    let (server, _dir) = setup().await;
    let result = server.store_fact(Parameters(StoreFactParams {
        content: "Rust code".to_string(),
        namespace: None,
        source: Some("urn:smem:code:fs:/home/user/main.rs".to_string()),
    })).await.unwrap();
    let text = tool_text(&result);
    assert!(text.contains("Source:"));
    assert!(text.contains("local file"));
}

#[tokio::test]
async fn test_store_fact_empty_content() {
    let (server, _dir) = setup().await;
    let result = server.store_fact(Parameters(StoreFactParams {
        content: "".to_string(),
        namespace: None,
        source: None,
    })).await.unwrap();
    assert!(is_error(&result));
    let text = tool_text(&result);
    assert!(text.contains("required"));
}

#[tokio::test]
async fn test_store_fact_invalid_urn() {
    let (server, _dir) = setup().await;
    let result = server.store_fact(Parameters(StoreFactParams {
        content: "test".to_string(),
        namespace: None,
        source: Some("not-a-urn".to_string()),
    })).await.unwrap();
    assert!(is_error(&result));
    let text = tool_text(&result);
    assert!(text.contains("Invalid source URN"));
}

// ── search_facts ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_search_facts_success() {
    let (server, _dir) = setup().await;

    // Store a fact
    server.store_fact(Parameters(StoreFactParams {
        content: "The quick brown fox jumps over the lazy dog".to_string(),
        namespace: None,
        source: None,
    })).await.unwrap();

    // Search
    let result = server.search_facts(Parameters(SearchFactsParams {
        query: "fox animal".to_string(),
        limit: Some(5),
        namespace: None,
    })).await.unwrap();
    assert!(!is_error(&result));
    let text = tool_text(&result);
    assert!(text.contains("Found"));
    assert!(text.contains("fox"));
}

#[tokio::test]
async fn test_search_facts_empty_query() {
    let (server, _dir) = setup().await;
    let result = server.search_facts(Parameters(SearchFactsParams {
        query: "".to_string(),
        limit: None,
        namespace: None,
    })).await.unwrap();
    assert!(is_error(&result));
}

#[tokio::test]
async fn test_search_facts_no_results() {
    let (server, _dir) = setup().await;
    let result = server.search_facts(Parameters(SearchFactsParams {
        query: "quantum chromodynamics".to_string(),
        limit: None,
        namespace: None,
    })).await.unwrap();
    let text = tool_text(&result);
    assert_eq!(text, "No results found.");
}

#[tokio::test]
async fn test_search_facts_with_namespace() {
    let (server, _dir) = setup().await;

    server.store_fact(Parameters(StoreFactParams {
        content: "elephants are big".to_string(),
        namespace: Some("animals".to_string()),
        source: None,
    })).await.unwrap();

    let result = server.search_facts(Parameters(SearchFactsParams {
        query: "elephants".to_string(),
        limit: None,
        namespace: Some("animals".to_string()),
    })).await.unwrap();
    let text = tool_text(&result);
    assert!(text.contains("Found"));
}

// ── update_fact ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_update_fact_success() {
    let (server, _dir) = setup().await;

    // Store
    let store_result = server.store_fact(Parameters(StoreFactParams {
        content: "original".to_string(),
        namespace: None,
        source: None,
    })).await.unwrap();
    let store_text = tool_text(&store_result);
    let id = store_text.lines().find(|l| l.starts_with("ID:")).unwrap()
        .strip_prefix("ID: ").unwrap();

    // Update
    let result = server.update_fact(Parameters(UpdateFactParams {
        id: id.to_string(),
        content: "updated".to_string(),
        source: None,
    })).await.unwrap();
    let text = tool_text(&result);
    assert!(text.contains("Updated."));
    assert!(text.contains(id));
}

#[tokio::test]
async fn test_update_fact_missing_id() {
    let (server, _dir) = setup().await;
    let result = server.update_fact(Parameters(UpdateFactParams {
        id: "".to_string(),
        content: "updated".to_string(),
        source: None,
    })).await.unwrap();
    assert!(is_error(&result));
}

#[tokio::test]
async fn test_update_fact_empty_content() {
    let (server, _dir) = setup().await;
    let result = server.update_fact(Parameters(UpdateFactParams {
        id: "some-id".to_string(),
        content: "".to_string(),
        source: None,
    })).await.unwrap();
    assert!(is_error(&result));
}

#[tokio::test]
async fn test_update_fact_invalid_urn() {
    let (server, _dir) = setup().await;

    let store_result = server.store_fact(Parameters(StoreFactParams {
        content: "original".to_string(),
        namespace: None,
        source: None,
    })).await.unwrap();
    let store_text = tool_text(&store_result);
    let id = store_text.lines().find(|l| l.starts_with("ID:")).unwrap()
        .strip_prefix("ID: ").unwrap();

    let result = server.update_fact(Parameters(UpdateFactParams {
        id: id.to_string(),
        content: "updated".to_string(),
        source: Some("bad-urn".to_string()),
    })).await.unwrap();
    assert!(is_error(&result));
    let text = tool_text(&result);
    assert!(text.contains("Invalid source URN"));
}

#[tokio::test]
async fn test_update_fact_not_found() {
    let (server, _dir) = setup().await;
    let result = server.update_fact(Parameters(UpdateFactParams {
        id: "non-existent-id".to_string(),
        content: "updated".to_string(),
        source: None,
    })).await.unwrap();
    assert!(is_error(&result));
    let text = tool_text(&result);
    assert!(text.contains("Update failed"));
}

// ── delete_fact ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_delete_fact_success() {
    let (server, _dir) = setup().await;

    let store_result = server.store_fact(Parameters(StoreFactParams {
        content: "to delete".to_string(),
        namespace: None,
        source: None,
    })).await.unwrap();
    let store_text = tool_text(&store_result);
    let id = store_text.lines().find(|l| l.starts_with("ID:")).unwrap()
        .strip_prefix("ID: ").unwrap();

    let result = server.delete_fact(Parameters(DeleteFactParams {
        id: id.to_string(),
    })).await.unwrap();
    let text = tool_text(&result);
    assert!(text.contains("Deleted"));
}

#[tokio::test]
async fn test_delete_fact_not_found() {
    let (server, _dir) = setup().await;
    let result = server.delete_fact(Parameters(DeleteFactParams {
        id: "non-existent".to_string(),
    })).await.unwrap();
    let text = tool_text(&result);
    assert!(text.contains("not found"));
}

#[tokio::test]
async fn test_delete_fact_missing_id() {
    let (server, _dir) = setup().await;
    let result = server.delete_fact(Parameters(DeleteFactParams {
        id: "".to_string(),
    })).await.unwrap();
    assert!(is_error(&result));
}

// ── list_facts ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_list_facts_empty() {
    let (server, _dir) = setup().await;
    let result = server.list_facts(Parameters(ListFactsParams {
        namespace: Some("empty".to_string()),
        limit: None,
        from: None,
        to: None,
    })).await.unwrap();
    let text = tool_text(&result);
    assert!(text.contains("No facts"));
}

#[tokio::test]
async fn test_list_facts_with_results() {
    let (server, _dir) = setup().await;

    for i in 0..3 {
        server.store_fact(Parameters(StoreFactParams {
            content: format!("doc {}", i),
            namespace: Some("docs".to_string()),
            source: None,
        })).await.unwrap();
    }

    let result = server.list_facts(Parameters(ListFactsParams {
        namespace: Some("docs".to_string()),
        limit: Some(10),
        from: None,
        to: None,
    })).await.unwrap();
    let text = tool_text(&result);
    assert!(text.contains("3 fact(s)"));
    assert!(text.contains("doc 0"));
}

#[tokio::test]
async fn test_list_facts_with_date_range() {
    let (server, _dir) = setup().await;

    server.store_fact(Parameters(StoreFactParams {
        content: "old fact".to_string(),
        namespace: Some("dated".to_string()),
        source: None,
    })).await.unwrap();

    // Query with a future date range → no results
    let result = server.list_facts(Parameters(ListFactsParams {
        namespace: Some("dated".to_string()),
        limit: None,
        from: Some("2099-01-01T00:00:00Z".to_string()),
        to: Some("2099-12-31T23:59:59Z".to_string()),
    })).await.unwrap();
    let text = tool_text(&result);
    assert!(text.contains("No facts"));
}

// ── build_source_urn ──────────────────────────────────────────────────────────

#[tokio::test]
async fn test_build_source_urn_success() {
    let (server, _dir) = setup().await;
    let result = server.build_source_urn(Parameters(BuildSourceUrnParams {
        content_type: "code".to_string(),
        origin: "fs".to_string(),
        locator: "/home/user/main.rs".to_string(),
        fragment: Some("L10-L30".to_string()),
    })).await.unwrap();
    let text = tool_text(&result);
    assert_eq!(text, "urn:smem:code:fs:/home/user/main.rs#L10-L30");
}

#[tokio::test]
async fn test_build_source_urn_invalid_type() {
    let (server, _dir) = setup().await;
    let result = server.build_source_urn(Parameters(BuildSourceUrnParams {
        content_type: "blob".to_string(),
        origin: "fs".to_string(),
        locator: "/foo".to_string(),
        fragment: None,
    })).await.unwrap();
    assert!(is_error(&result));
}

#[tokio::test]
async fn test_build_source_urn_empty_origin() {
    let (server, _dir) = setup().await;
    let result = server.build_source_urn(Parameters(BuildSourceUrnParams {
        content_type: "code".to_string(),
        origin: "".to_string(),
        locator: "/foo".to_string(),
        fragment: None,
    })).await.unwrap();
    assert!(is_error(&result));
}

#[tokio::test]
async fn test_build_source_urn_empty_locator() {
    let (server, _dir) = setup().await;
    let result = server.build_source_urn(Parameters(BuildSourceUrnParams {
        content_type: "code".to_string(),
        origin: "fs".to_string(),
        locator: "".to_string(),
        fragment: None,
    })).await.unwrap();
    assert!(is_error(&result));
}

// ── parse_source_urn ──────────────────────────────────────────────────────────

#[tokio::test]
async fn test_parse_source_urn_success() {
    let (server, _dir) = setup().await;
    let result = server.parse_source_urn(Parameters(ParseSourceUrnParams {
        urn: "urn:smem:code:fs:/home/user/main.rs#L10".to_string(),
    })).await.unwrap();
    let text = tool_text(&result);
    assert!(text.contains("\"valid\": true"));
    assert!(text.contains("\"content_type\": \"code\""));
}

#[tokio::test]
async fn test_parse_source_urn_invalid() {
    let (server, _dir) = setup().await;
    let result = server.parse_source_urn(Parameters(ParseSourceUrnParams {
        urn: "not-a-urn".to_string(),
    })).await.unwrap();
    let text = tool_text(&result);
    assert!(text.contains("\"valid\": false"));
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
    let (server, _dir) = setup().await;
    let result = server.describe_urn_schema().await.unwrap();
    let text = tool_text(&result);
    assert!(text.contains("format"));
    assert!(text.contains("content_types"));
    assert!(text.contains("origins"));
}


