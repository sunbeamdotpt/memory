use sunbeam_memory::config::MemoryConfig;
use sunbeam_memory::memory::service::MemoryService;

#[tokio::test]
async fn test_memory_service_can_be_created() {
    let dir = tempfile::tempdir().unwrap();
    let config = MemoryConfig {
        base_dir: dir.path().to_str().unwrap().to_string(),
        ..Default::default()
    };
    let service = MemoryService::new(&config).await;
    assert!(
        service.is_ok(),
        "Memory service should be created successfully"
    );
}

#[tokio::test]
async fn test_memory_service_handles_invalid_path() {
    let config = MemoryConfig {
        base_dir: "/invalid/path/that/does/not/exist".to_string(),
        ..Default::default()
    };
    let service = MemoryService::new(&config).await;
    assert!(
        service.is_err(),
        "Memory service should fail with invalid path"
    );
}
