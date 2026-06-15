use std::time::Duration;

use crossbeam_channel::bounded;
use sunbeam_memory::{
    config::MemoryConfig,
    indexer::{IndexService, IndexWatcher, IngestionEvent},
    memory::service::MemoryService,
};

async fn setup() -> (MemoryService, IndexService, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let config = MemoryConfig {
        base_dir: root.to_str().unwrap().to_string(),
        ..Default::default()
    };
    let memory = MemoryService::new(&config).await.unwrap();
    let (dummy_tx, dummy_rx) = bounded(1);
    let watcher = IndexWatcher::new(dummy_tx).unwrap();
    let indexer = IndexService::new(memory.clone(), dummy_rx, watcher);
    (memory, indexer, dir)
}

async fn setup_with_channel() -> (
    MemoryService,
    IndexService,
    crossbeam_channel::Sender<IngestionEvent>,
    tempfile::TempDir,
) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let config = MemoryConfig {
        base_dir: root.to_str().unwrap().to_string(),
        ..Default::default()
    };
    let memory = MemoryService::new(&config).await.unwrap();
    // The watcher keeps its own sender; use a dummy channel so dropping the
    // test sender actually disconnects the indexer's receiver.
    let (dummy_tx, _dummy_rx) = bounded(1);
    let watcher = IndexWatcher::new(dummy_tx).unwrap();
    let (tx, rx) = bounded(10);
    let indexer = IndexService::new(memory.clone(), rx, watcher);
    (memory, indexer, tx, dir)
}

#[tokio::test]
async fn test_add_target_and_sync_directory() {
    let (memory, indexer, dir) = setup().await;
    let root = dir.path().canonicalize().unwrap();

    let file_path = root.join("notes.txt");
    std::fs::write(&file_path, "sunbeam-memory indexer test content").unwrap();

    let ids = indexer
        .add_target(root.to_str().unwrap(), Some("default"), Some("directory"))
        .await
        .unwrap();
    assert_eq!(ids.len(), 1);

    indexer.sync_target(&ids[0]).await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;

    let results = memory
        .search_facts("indexer test content", 5, None)
        .await
        .unwrap();
    assert!(!results.is_empty());

    let targets = indexer.list_targets().await.unwrap();
    assert_eq!(targets.len(), 1);
}

#[tokio::test]
async fn test_add_target_with_glob_pattern() {
    let (memory, indexer, dir) = setup().await;
    let root = dir.path().canonicalize().unwrap();

    std::fs::write(root.join("a.txt"), "alpha content").unwrap();
    std::fs::write(root.join("b.txt"), "beta content").unwrap();

    let pattern = format!("{}/*.txt", root.to_str().unwrap());
    let ids = indexer
        .add_target(&pattern, Some("glob"), Some("file"))
        .await
        .unwrap();
    assert_eq!(ids.len(), 2);

    for id in &ids {
        indexer.sync_target(id).await.unwrap();
    }
    tokio::time::sleep(Duration::from_millis(500)).await;

    let results = memory.search_facts("alpha content", 5, None).await.unwrap();
    assert!(!results.is_empty());
}

#[tokio::test]
async fn test_remove_target() {
    let (_memory, indexer, dir) = setup().await;
    let root = dir.path().canonicalize().unwrap();

    let ids = indexer
        .add_target(root.to_str().unwrap(), None, Some("directory"))
        .await
        .unwrap();
    assert!(!ids.is_empty());

    let removed = indexer.remove_target(&ids[0]).await.unwrap();
    assert!(removed);

    let removed_again = indexer.remove_target(&ids[0]).await.unwrap();
    assert!(!removed_again);
}

#[tokio::test]
async fn test_get_progress_for_unknown_target() {
    let (_memory, indexer, _dir) = setup().await;
    assert!(indexer.progress().get("unknown").is_none());
}

#[tokio::test]
async fn test_sync_target_not_found() {
    let (_memory, indexer, _dir) = setup().await;
    let err = indexer.sync_target("no-such-target").await.unwrap_err();
    assert!(err.to_string().contains("target not found"));
}

#[tokio::test]
async fn test_sync_target_binary_file_is_recorded_as_failed() {
    let (_memory, indexer, dir) = setup().await;
    let root = dir.path().canonicalize().unwrap();

    let png = root.join("image.png");
    std::fs::write(&png, b"\x89PNG\r\n\x1a\nfake").unwrap();

    let ids = indexer
        .add_target(png.to_str().unwrap(), None, Some("file"))
        .await
        .unwrap();

    indexer.sync_target(&ids[0]).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;

    let progress = indexer.progress().get(&ids[0]).unwrap();
    assert_eq!(progress.files_total, 1);
    assert_eq!(progress.files_failed, 1);
}

#[tokio::test]
async fn test_sync_target_empty_file_completes() {
    let (_memory, indexer, dir) = setup().await;
    let root = dir.path().canonicalize().unwrap();

    let empty = root.join("empty.txt");
    std::fs::write(&empty, "").unwrap();

    let ids = indexer
        .add_target(empty.to_str().unwrap(), None, Some("file"))
        .await
        .unwrap();

    indexer.sync_target(&ids[0]).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;

    let progress = indexer.progress().get(&ids[0]).unwrap();
    assert_eq!(progress.files_total, 1);
    assert_eq!(progress.files_completed, 1);
}

#[tokio::test]
async fn test_process_batch_delete_marks_fact_stale() {
    let (memory, indexer, tx, dir) = setup_with_channel().await;
    let root = dir.path().canonicalize().unwrap();

    let file_path = root.join("doc.txt");
    std::fs::write(&file_path, "content to be deleted").unwrap();

    let ids = indexer
        .add_target(file_path.to_str().unwrap(), None, Some("file"))
        .await
        .unwrap();

    indexer.sync_target(&ids[0]).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;

    let before = memory
        .search_facts("content to be deleted", 5, None)
        .await
        .unwrap();
    assert!(!before.is_empty());

    tx.send(IngestionEvent::Delete(file_path)).unwrap();
    drop(tx);
    indexer.run().await;

    let after = memory
        .search_facts("content to be deleted", 5, None)
        .await
        .unwrap();
    assert!(after.is_empty());
}

#[tokio::test]
async fn test_process_batch_modify_reingests_file() {
    let (memory, indexer, tx, dir) = setup_with_channel().await;
    let root = dir.path().canonicalize().unwrap();

    let file_path = root.join("doc.txt");
    std::fs::write(&file_path, "original content").unwrap();

    let ids = indexer
        .add_target(root.to_str().unwrap(), None, Some("directory"))
        .await
        .unwrap();

    indexer.sync_target(&ids[0]).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;

    std::fs::write(&file_path, "updated content").unwrap();
    tx.send(IngestionEvent::Modify(file_path)).unwrap();
    drop(tx);
    indexer.run().await;

    let results = memory
        .search_facts("updated content", 5, None)
        .await
        .unwrap();
    assert!(!results.is_empty());
}

#[tokio::test]
async fn test_process_batch_ignores_binary_and_directory_events() {
    let (_memory, indexer, tx, dir) = setup_with_channel().await;
    let root = dir.path().canonicalize().unwrap();

    let bin_path = root.join("image.png");
    std::fs::write(&bin_path, b"\x89PNG\r\n\x1a\nfake").unwrap();

    let subdir = root.join("subdir");
    std::fs::create_dir(&subdir).unwrap();

    indexer
        .add_target(root.to_str().unwrap(), None, Some("directory"))
        .await
        .unwrap();

    tx.send(IngestionEvent::Create(bin_path)).unwrap();
    tx.send(IngestionEvent::Create(subdir)).unwrap();
    drop(tx);
    indexer.run().await;
}
