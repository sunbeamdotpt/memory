use std::time::Duration;

use tokio::time::sleep;

use sunbeam_memory::{
    config::MemoryConfig,
    core::service::{CoreService, ErrorEntry},
    error::ServerError,
    indexer::{IndexService, IndexWatcher},
    memory::service::MemoryService,
};

async fn setup() -> (CoreService, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let config = MemoryConfig {
        base_dir: dir.path().to_str().unwrap().to_string(),
        ..Default::default()
    };
    let memory = MemoryService::new(&config).await.unwrap();
    let (event_tx, event_rx) = crossbeam_channel::bounded(1000);
    let watcher = IndexWatcher::new(event_tx).unwrap();
    let indexer = IndexService::new(memory.clone(), event_rx, watcher);
    (CoreService::new(memory, indexer), dir)
}

async fn wait_for_error<F>(core: &CoreService, predicate: F) -> ErrorEntry
where
    F: Fn(&ErrorEntry) -> bool,
{
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let entries = core.get_recent_errors(None, 100).await.unwrap();
        if let Some(entry) = entries.into_iter().find(&predicate) {
            return entry;
        }
        if tokio::time::Instant::now() >= deadline {
            let all = core.get_recent_errors(None, 100).await.unwrap();
            panic!("timed out waiting for error log entry; had {all:?}");
        }
        sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn test_validation_errors() {
    let (core, _dir) = setup().await;

    assert!(
        matches!(
            core.store_fact("   ", None, None).await,
            Err(ServerError::InvalidArgument(_))
        ),
        "empty content should be rejected"
    );
    assert!(
        matches!(
            core.update_fact("", "x", None).await,
            Err(ServerError::InvalidArgument(_))
        ),
        "empty id should be rejected for update"
    );
    assert!(
        matches!(
            core.delete_fact("").await,
            Err(ServerError::InvalidArgument(_))
        ),
        "empty id should be rejected for delete"
    );
    assert!(
        matches!(
            core.add_watch_target("", None, None).await,
            Err(ServerError::InvalidArgument(_))
        ),
        "empty path should be rejected"
    );
    assert!(
        matches!(
            core.remove_watch_target("").await,
            Err(ServerError::InvalidArgument(_))
        ),
        "empty target_id should be rejected for remove"
    );
    assert!(
        matches!(
            core.sync_watch_target(""),
            Err(ServerError::InvalidArgument(_))
        ),
        "empty target_id should be rejected for sync"
    );
    assert!(
        matches!(
            core.build_source_urn("", "fs", "/x", None),
            Err(ServerError::InvalidArgument(_))
        ),
        "empty content_type should be rejected"
    );
    assert!(
        matches!(
            core.build_source_urn("code", "", "/x", None),
            Err(ServerError::InvalidArgument(_))
        ),
        "empty origin should be rejected"
    );
    assert!(
        matches!(
            core.build_source_urn("code", "fs", "", None),
            Err(ServerError::InvalidArgument(_))
        ),
        "empty locator should be rejected"
    );
    assert!(
        matches!(
            core.parse_source_urn(""),
            Err(ServerError::InvalidArgument(_))
        ),
        "empty urn should be rejected"
    );
    assert!(
        matches!(
            core.restore_stale_fact(""),
            Err(ServerError::InvalidArgument(_))
        ),
        "empty id should be rejected for restore"
    );
    assert!(
        matches!(
            core.resolve_error("").await,
            Err(ServerError::InvalidArgument(_))
        ),
        "empty error_id should be rejected"
    );
}

#[tokio::test]
async fn test_embedding_service_getter() {
    let (core, _dir) = setup().await;
    let svc = core.memory().embedding_service();
    let model = svc.lock().await.current_model();
    assert!(!model.model_name().is_empty());
}

#[tokio::test]
async fn test_restore_stale_fact() {
    let (core, _dir) = setup().await;

    let fact = core.store_fact("restore me", None, None).await.unwrap();

    // Mark the fact stale directly through the store so we can exercise restore.
    let store = core.memory().get_store();
    let db = store.db();
    let db_guard = db.lock().unwrap();
    assert!(db_guard.mark_fact_stale(&fact.id).unwrap());
    drop(db_guard);

    assert!(
        core.restore_stale_fact(&fact.id).unwrap(),
        "stale fact should be restored"
    );
    assert!(
        !core
            .restore_stale_fact("01ABCDEF0123456789ABCDEF01")
            .unwrap(),
        "restoring a non-existent fact should return false"
    );
}

#[tokio::test]
async fn test_error_logs() {
    let (core, _dir) = setup().await;

    core.memory()
        .log_error(
            "E1",
            "core-test",
            "error",
            "unit test error",
            Some("details"),
        )
        .unwrap();

    // Component filter should work.
    let filtered = core.get_recent_errors(Some("core-test"), 10).await.unwrap();
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].error_id, "E1");
    assert_eq!(filtered[0].component, "core-test");
    assert_eq!(filtered[0].severity, "error");
    assert_eq!(filtered[0].message, "unit test error");
    assert_eq!(filtered[0].details, Some("details".to_string()));

    // No-filter query should include the same entry.
    let all = core.get_recent_errors(None, 10).await.unwrap();
    assert!(all.iter().any(|e| e.error_id == "E1"));

    assert!(core.resolve_error("E1").await.unwrap());
    assert!(!core.resolve_error("missing").await.unwrap());
}

#[tokio::test]
async fn test_sync_watch_target_error_logging() {
    let (core, _dir) = setup().await;

    core.sync_watch_target("no-such-target-id").unwrap();

    let entry = wait_for_error(&core, |e| {
        e.component == "indexer"
            && e.message
                .contains("sync failed for target no-such-target-id")
    })
    .await;
    assert!(
        entry
            .details
            .unwrap()
            .contains("target not found: no-such-target-id")
    );
}

#[tokio::test]
async fn test_remove_watch_target_not_found() {
    let (core, _dir) = setup().await;
    assert!(!core.remove_watch_target("nope").await.unwrap());
}

#[tokio::test]
async fn test_add_watch_target_initial_sync_failure_path() {
    let (core, _db_dir) = setup().await;
    let watch_dir = tempfile::tempdir().unwrap();

    let target_id = core
        .add_watch_target(watch_dir.path().to_str().unwrap(), None, Some("directory"))
        .await
        .unwrap()
        .pop()
        .expect("target should be created");

    // Remove the target synchronously from the DB before the spawned initial sync
    // gets a chance to run. The background sync will then see a missing target
    // and log the initial-sync failure path.
    let store = core.memory().get_store();
    let db = store.db();
    {
        let db_guard = db.lock().unwrap();
        assert!(db_guard.delete_ingestion_target(&target_id).unwrap());
    }

    let entry = wait_for_error(&core, |e| {
        e.component == "indexer" && e.message.contains("initial sync failed for target")
    })
    .await;
    assert!(entry.message.contains(&target_id));
    assert!(
        entry.details.as_ref().unwrap().contains("target not found"),
        "expected not-found error, got {:?}",
        entry.details
    );
}
