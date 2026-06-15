use rmcp::handler::server::wrapper::Parameters;
use sunbeam_memory::{
    config::MemoryConfig,
    core::service::CoreService,
    indexer::{IndexService, IndexWatcher},
    mcp::server::SunbeamServer,
    mcp::{
        AddWatchTargetParams, BuildSourceUrnParams, DeleteFactParams, GetIndexProgressParams,
        GetRecentErrorsParams, ListFactsParams, ParseSourceUrnParams, RemoveWatchTargetParams,
        SearchFactsParams, StoreFactParams, SyncWatchTargetParams,
    },
    memory::service::MemoryService,
};

async fn setup() -> (SunbeamServer, CoreService, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let config = MemoryConfig {
        base_dir: dir.path().to_str().unwrap().to_string(),
        ..Default::default()
    };
    let memory = MemoryService::new(&config).await.unwrap();
    let (dummy_tx, dummy_rx) = crossbeam_channel::bounded(1);
    let watcher = IndexWatcher::new(dummy_tx).unwrap();
    let indexer = IndexService::new(memory.clone(), dummy_rx, watcher);
    let core = CoreService::new(memory, indexer);
    let server = SunbeamServer::new(core.clone());
    (server, core, dir)
}

fn tool_text(result: &rmcp::model::CallToolResult) -> String {
    result.content[0]
        .as_text()
        .map(|t| t.text.clone())
        .unwrap_or_default()
}

fn is_error(result: &rmcp::model::CallToolResult) -> bool {
    result.is_error == Some(true)
}

/// Drop a table from the database using a separate connection so that the next
/// operation on the in-process connection hits a "no such table" error.
fn drop_table(base_dir: &std::path::Path, table: &str) {
    let db_path = base_dir.join("semantic.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    conn.execute(&format!("DROP TABLE IF EXISTS {table};"), [])
        .unwrap();
}

// ── store_fact ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_store_fact_db_error_path() {
    let (server, _core, dir) = setup().await;
    drop_table(dir.path(), "facts");

    let result = server
        .store_fact(Parameters(StoreFactParams {
            content: "this should fail to persist".to_string(),
            namespace: None,
            source: None,
        }))
        .await
        .unwrap();

    assert!(is_error(&result));
    let text = tool_text(&result);
    assert!(
        text.contains("Failed to store") || text.contains("database"),
        "unexpected error text: {text}"
    );
}

// ── search_facts ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_search_facts_source_human_readable() {
    let (server, _core, _dir) = setup().await;

    server
        .store_fact(Parameters(StoreFactParams {
            content: "Rust main function".to_string(),
            namespace: None,
            source: Some("urn:smem:code:fs:/home/user/main.rs#L10".to_string()),
        }))
        .await
        .unwrap();

    let result = server
        .search_facts(Parameters(SearchFactsParams {
            query: "Rust main".to_string(),
            limit: Some(5),
            namespace: None,
        }))
        .await
        .unwrap();

    assert!(!is_error(&result));
    let text = tool_text(&result);
    assert!(
        text.contains("local file"),
        "human_readable missing: {text}"
    );
    assert!(text.contains("source:"), "source line missing: {text}");
}

// ── delete_fact ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_delete_fact_db_error_path() {
    let (server, _core, dir) = setup().await;

    // Store a fact so delete has to hit the database.
    let store_result = server
        .store_fact(Parameters(StoreFactParams {
            content: "to delete".to_string(),
            namespace: None,
            source: None,
        }))
        .await
        .unwrap();
    let id = tool_text(&store_result)
        .lines()
        .find(|l| l.starts_with("ID:"))
        .unwrap()
        .strip_prefix("ID: ")
        .unwrap()
        .to_string();

    drop_table(dir.path(), "facts");

    let result = server
        .delete_fact(Parameters(DeleteFactParams { id }))
        .await
        .unwrap();

    assert!(is_error(&result));
    let text = tool_text(&result);
    assert!(
        text.contains("Delete failed") || text.contains("database"),
        "unexpected error text: {text}"
    );
}

// ── list_facts ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_list_facts_invalid_argument_out_of_range_timestamp() {
    let (server, _core, dir) = setup().await;

    // Insert a fact directly with an out-of-range created_at timestamp so that
    // formatting it as RFC 3339 fails with InvalidArgument.
    let db_path = dir.path().join("semantic.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    conn.execute(
        "INSERT INTO facts (id, namespace, content, created_at, source) VALUES (?, ?, ?, ?, ?)",
        rusqlite::params![
            "01ABCDEF0123456789ABCDEF01",
            "bad_ts",
            "bad fact",
            i64::MAX,
            None::<&str>
        ],
    )
    .unwrap();
    drop(conn);

    let result = server
        .list_facts(Parameters(ListFactsParams {
            namespace: Some("bad_ts".to_string()),
            limit: None,
            from: None,
            to: None,
        }))
        .await
        .unwrap();

    assert!(is_error(&result));
    let text = tool_text(&result);
    assert!(
        text.contains("timestamp") || text.contains("out of range"),
        "unexpected error text: {text}"
    );
}

#[tokio::test]
async fn test_list_facts_db_error_path() {
    let (server, _core, dir) = setup().await;
    drop_table(dir.path(), "facts");

    let result = server
        .list_facts(Parameters(ListFactsParams {
            namespace: Some("default".to_string()),
            limit: None,
            from: None,
            to: None,
        }))
        .await
        .unwrap();

    assert!(is_error(&result));
    let text = tool_text(&result);
    assert!(
        text.contains("List failed") || text.contains("database"),
        "unexpected error text: {text}"
    );
}

// ── build_source_urn ──────────────────────────────────────────────────────────

#[tokio::test]
async fn test_build_source_urn_invalid_argument() {
    let (server, _core, _dir) = setup().await;

    // Empty content type is already covered; exercise the invalid content_type branch.
    let result = server
        .build_source_urn(Parameters(BuildSourceUrnParams {
            content_type: "unsupported_type".to_string(),
            origin: "fs".to_string(),
            locator: "/tmp/main.rs".to_string(),
            fragment: None,
        }))
        .await
        .unwrap();

    assert!(is_error(&result));
}

// ── parse_source_urn ──────────────────────────────────────────────────────────

#[tokio::test]
async fn test_parse_source_urn_invalid_returns_valid_false() {
    let (server, _core, _dir) = setup().await;

    let result = server
        .parse_source_urn(Parameters(ParseSourceUrnParams {
            urn: "urn:smem:bad:fs:/tmp/main.rs".to_string(),
        }))
        .await
        .unwrap();

    assert!(!is_error(&result));
    let text = tool_text(&result);
    assert!(
        text.contains("\"valid\": false"),
        "expected valid false: {text}"
    );
}

#[tokio::test]
async fn test_parse_source_urn_empty_invalid_argument() {
    let (server, _core, _dir) = setup().await;

    let result = server
        .parse_source_urn(Parameters(ParseSourceUrnParams {
            urn: "".to_string(),
        }))
        .await
        .unwrap();

    assert!(is_error(&result));
    let text = tool_text(&result);
    assert!(text.contains("required"), "unexpected error text: {text}");
}

// ── add_watch_target ──────────────────────────────────────────────────────────

#[tokio::test]
async fn test_add_watch_target_glob_success() {
    let (server, _core, dir) = setup().await;

    // Create several files that match a glob pattern.
    for i in 0..3 {
        let path = dir.path().join(format!("file{i}.txt"));
        std::fs::write(&path, format!("content {i}")).unwrap();
    }

    let pattern = dir.path().join("*.txt").to_string_lossy().to_string();
    let result = server
        .add_watch_target(Parameters(AddWatchTargetParams {
            path: pattern,
            namespace: None,
            target_type: Some("file".to_string()),
        }))
        .await
        .unwrap();

    assert!(!is_error(&result));
    let text = tool_text(&result);
    assert!(
        text.contains("Watch targets added from glob") || text.contains("Watch target added"),
        "unexpected success text: {text}"
    );
}

#[tokio::test]
async fn test_add_watch_target_invalid_argument_empty_path() {
    let (server, _core, _dir) = setup().await;

    let result = server
        .add_watch_target(Parameters(AddWatchTargetParams {
            path: "".to_string(),
            namespace: None,
            target_type: None,
        }))
        .await
        .unwrap();

    assert!(is_error(&result));
    let text = tool_text(&result);
    assert!(text.contains("required"), "unexpected error text: {text}");
}

#[tokio::test]
async fn test_add_watch_target_db_error_path() {
    let (server, _core, dir) = setup().await;
    drop_table(dir.path(), "ingestion_targets");

    let result = server
        .add_watch_target(Parameters(AddWatchTargetParams {
            path: dir.path().to_str().unwrap().to_string(),
            namespace: None,
            target_type: Some("directory".to_string()),
        }))
        .await
        .unwrap();

    assert!(is_error(&result));
    let text = tool_text(&result);
    assert!(
        text.contains("Failed to add watch target") || text.contains("database"),
        "unexpected error text: {text}"
    );
}

// ── sync_watch_target ─────────────────────────────────────────────────────────

#[tokio::test]
async fn test_sync_watch_target_invalid_argument_empty_id() {
    let (server, _core, _dir) = setup().await;

    let result = server
        .sync_watch_target(Parameters(SyncWatchTargetParams {
            target_id: "".to_string(),
        }))
        .await
        .unwrap();

    assert!(is_error(&result));
    let text = tool_text(&result);
    assert!(text.contains("required"), "unexpected error text: {text}");
}

#[tokio::test]
async fn test_sync_watch_target_success() {
    let (server, _core, dir) = setup().await;

    let add = server
        .add_watch_target(Parameters(AddWatchTargetParams {
            path: dir.path().to_str().unwrap().to_string(),
            namespace: None,
            target_type: Some("directory".to_string()),
        }))
        .await
        .unwrap();
    let id = tool_text(&add)
        .lines()
        .find(|l| l.starts_with("ID:"))
        .unwrap()
        .strip_prefix("ID: ")
        .unwrap()
        .to_string();

    let result = server
        .sync_watch_target(Parameters(SyncWatchTargetParams { target_id: id }))
        .await
        .unwrap();

    assert!(!is_error(&result));
    let text = tool_text(&result);
    assert!(
        text.contains("Sync started"),
        "unexpected success text: {text}"
    );
}

// ── remove_watch_target ───────────────────────────────────────────────────────

#[tokio::test]
async fn test_remove_watch_target_not_found() {
    let (server, _core, _dir) = setup().await;

    let result = server
        .remove_watch_target(Parameters(RemoveWatchTargetParams {
            target_id: "01ABCDEF0123456789ABCDEF01".to_string(),
        }))
        .await
        .unwrap();

    assert!(!is_error(&result));
    let text = tool_text(&result);
    assert!(text.contains("not found"), "unexpected text: {text}");
}

#[tokio::test]
async fn test_remove_watch_target_db_error_path() {
    let (server, _core, dir) = setup().await;
    drop_table(dir.path(), "ingestion_targets");

    let result = server
        .remove_watch_target(Parameters(RemoveWatchTargetParams {
            target_id: "01ABCDEF0123456789ABCDEF01".to_string(),
        }))
        .await
        .unwrap();

    assert!(is_error(&result));
    let text = tool_text(&result);
    assert!(
        text.contains("Failed to remove watch target") || text.contains("database"),
        "unexpected error text: {text}"
    );
}

// ── list_watch_targets ────────────────────────────────────────────────────────

#[tokio::test]
async fn test_list_watch_targets_empty() {
    let (server, _core, _dir) = setup().await;

    let result = server.list_watch_targets().await.unwrap();

    assert!(!is_error(&result));
    let text = tool_text(&result);
    assert_eq!(text, "No watch targets configured.");
}

#[tokio::test]
async fn test_list_watch_targets_db_error_path() {
    let (server, _core, dir) = setup().await;
    drop_table(dir.path(), "ingestion_targets");

    let result = server.list_watch_targets().await.unwrap();

    assert!(is_error(&result));
    let text = tool_text(&result);
    assert!(
        text.contains("Failed to list watch targets") || text.contains("database"),
        "unexpected error text: {text}"
    );
}

// ── get_index_progress ────────────────────────────────────────────────────────

#[tokio::test]
async fn test_get_index_progress_with_data() {
    let (server, _core, dir) = setup().await;

    let add = server
        .add_watch_target(Parameters(AddWatchTargetParams {
            path: dir.path().to_str().unwrap().to_string(),
            namespace: None,
            target_type: Some("directory".to_string()),
        }))
        .await
        .unwrap();
    let id = tool_text(&add)
        .lines()
        .find(|l| l.starts_with("ID:"))
        .unwrap()
        .strip_prefix("ID: ")
        .unwrap()
        .to_string();

    // Trigger a sync so progress is populated.
    server
        .sync_watch_target(Parameters(SyncWatchTargetParams {
            target_id: id.clone(),
        }))
        .await
        .unwrap();

    // Give the background sync a moment to update progress.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let result = server
        .get_index_progress(Parameters(GetIndexProgressParams { target_id: id }))
        .await
        .unwrap();

    assert!(!is_error(&result));
    let text = tool_text(&result);
    assert!(
        text.contains("Progress for target") || text.contains("No progress data"),
        "unexpected text: {text}"
    );
}

// ── get_recent_errors ─────────────────────────────────────────────────────────

#[tokio::test]
async fn test_get_recent_errors_with_entries() {
    let (server, core, _dir) = setup().await;

    // Log an error directly through the memory service.
    let err_id = ulid::Ulid::new().to_string();
    core.memory()
        .log_error(&err_id, "mcp", "error", "test error", Some("details"))
        .unwrap();

    let result = server
        .get_recent_errors(Parameters(GetRecentErrorsParams {
            component: None,
            limit: None,
        }))
        .await
        .unwrap();

    assert!(!is_error(&result));
    let text = tool_text(&result);
    assert!(text.contains("test error"), "expected logged error: {text}");
}

#[tokio::test]
async fn test_get_recent_errors_db_error_path() {
    let (server, _core, dir) = setup().await;
    drop_table(dir.path(), "errors");

    let result = server
        .get_recent_errors(Parameters(GetRecentErrorsParams {
            component: None,
            limit: None,
        }))
        .await
        .unwrap();

    assert!(is_error(&result));
    let text = tool_text(&result);
    assert!(
        text.contains("Failed to fetch errors") || text.contains("database"),
        "unexpected error text: {text}"
    );
}
