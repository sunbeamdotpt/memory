use mcp_server::memory::service::MemoryService;
use mcp_server::config::MemoryConfig;

#[tokio::test]
async fn test_memory_service_can_add_fact() {
    let dir = tempfile::tempdir().unwrap();
    let config = MemoryConfig { base_dir: dir.path().to_str().unwrap().to_string(), ..Default::default() };
    let service = MemoryService::new(&config).await.unwrap();

    let result = service.add_fact("test", "Hello world", None).await;
    assert!(result.is_ok(), "Should be able to add fact");
    assert!(!result.unwrap().id.is_empty());
}

#[tokio::test]
async fn test_memory_service_can_search_facts() {
    let dir = tempfile::tempdir().unwrap();
    let config = MemoryConfig { base_dir: dir.path().to_str().unwrap().to_string(), ..Default::default() };
    let service = MemoryService::new(&config).await.unwrap();

    service.add_fact("test", "Rust programming language", None).await.ok();
    let results = service.search_facts("programming", 5, None).await;
    assert!(results.is_ok(), "Should be able to search facts");
}

#[tokio::test]
async fn test_memory_service_handles_errors() {
    let dir = tempfile::tempdir().unwrap();
    let config = MemoryConfig { base_dir: dir.path().to_str().unwrap().to_string(), ..Default::default() };
    let service = MemoryService::new(&config).await.unwrap();

    // Deleting a non-existent fact should return Ok(false), not an error
    let result = service.delete_fact("non-existent-id").await;
    assert!(result.is_ok(), "Should handle missing fact gracefully");
    assert_eq!(result.unwrap(), false);
}
