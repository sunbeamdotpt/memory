use mcp_server::{
    config::MemoryConfig,
    indexer::{IndexService, IndexWatcher},
    memory::service::MemoryService,
};
use std::path::PathBuf;

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

#[tokio::test]
async fn test_pdf_extraction_and_search() {
    let (memory, indexer, _dir) = setup().await;

    let pdf_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/test_pdf_extraction/chubby-lock-service.pdf");

    assert!(
        pdf_path.exists(),
        "PDF fixture not found at {}",
        pdf_path.display()
    );

    // Add the PDF as an ingestion target and sync it
    let target_ids = indexer
        .add_target(pdf_path.to_str().unwrap(), Some("research"), None)
        .await
        .expect("failed to add PDF target");
    assert_eq!(target_ids.len(), 1, "single PDF should produce one target");
    let target_id = &target_ids[0];

    indexer
        .sync_target(target_id)
        .await
        .expect("failed to sync PDF target");

    // Give embedding a moment to settle (optional, but safer)
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Search for a concept from the paper
    let results = memory
        .search_facts("lock service loosely coupled distributed", 5, Some("research"))
        .await
        .expect("search failed");

    assert!(
        !results.is_empty(),
        "expected at least one search result for the PDF content"
    );

    // Verify the result contains the PDF source URN
    let has_pdf_source = results.iter().any(|f| {
        f.source
            .as_ref()
            .map(|s| s.contains("chubby-lock-service.pdf"))
            .unwrap_or(false)
    });
    assert!(
        has_pdf_source,
        "expected search result to contain the PDF source URN, got sources: {:?}",
        results.iter().map(|f| &f.source).collect::<Vec<_>>()
    );

    // Also do a more specific search that should definitely hit the title
    let title_results = memory
        .search_facts("Chubby consensus protocol", 5, Some("research"))
        .await
        .expect("title search failed");

    assert!(
        !title_results.is_empty(),
        "expected search results for 'Chubby consensus protocol'"
    );
}

#[tokio::test]
async fn test_pdf_target_progress_tracked() {
    let (_memory, indexer, _dir) = setup().await;

    let pdf_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/test_pdf_extraction/chubby-lock-service.pdf");

    let target_ids = indexer
        .add_target(pdf_path.to_str().unwrap(), Some("research"), None)
        .await
        .expect("failed to add PDF target");
    assert_eq!(target_ids.len(), 1);
    let target_id = &target_ids[0];

    indexer
        .sync_target(target_id)
        .await
        .expect("failed to sync PDF target");

    // Progress should show the file was processed
    let progress = indexer.progress().get(target_id);
    assert!(progress.is_some(), "progress should be tracked for target");
    let p = progress.unwrap();
    assert_eq!(p.files_total, 1, "PDF is a single file");
    assert_eq!(p.files_completed, 1, "PDF should be fully processed");
    assert_eq!(p.files_failed, 0, "PDF extraction should not fail");
}
