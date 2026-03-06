use mcp_server::memory::service::MemoryService;
use mcp_server::config::MemoryConfig;

#[tokio::test]
async fn test_memory_service_can_add_fact_to_semantic_memory() {
    let config = MemoryConfig { base_dir: "./tests/data/test_semantic_data".to_string(), ..Default::default() };

    let service = MemoryService::new(&config).await.unwrap();

    let result = service.add_fact("test_namespace", "Test fact content", None).await;

    assert!(result.is_ok(), "Should be able to add fact to semantic memory");

    if let Ok(fact) = result {
        assert_eq!(fact.namespace, "test_namespace");
        assert_eq!(fact.content, "Test fact content");
        assert!(!fact.id.is_empty(), "Fact should have an ID");
    }
}

#[tokio::test]
async fn test_memory_service_can_search_semantic_memory() {
    let config = MemoryConfig { base_dir: "./tests/data/test_semantic_search".to_string(), ..Default::default() };

    let service = MemoryService::new(&config).await.unwrap();

    service.add_fact("test", "Rust is a systems programming language", None).await.ok();

    let result = service.search_facts("programming language", 5, None).await;

    assert!(result.is_ok(), "Should be able to search semantic memory");

    if let Ok(search_results) = result {
        assert!(!search_results.is_empty(), "Should find at least one result");
    }
}

#[tokio::test]
async fn test_memory_service_handles_semantic_errors() {
    let config = MemoryConfig { base_dir: "/invalid/semantic/path".to_string(), ..Default::default() };

    let result = MemoryService::new(&config).await;
    assert!(result.is_err(), "Should handle invalid paths gracefully");
}

#[tokio::test]
async fn test_memory_service_can_delete_facts() {
    let config = MemoryConfig { base_dir: "./tests/data/test_semantic_delete".to_string(), ..Default::default() };

    let service = MemoryService::new(&config).await.unwrap();

    let add_result = service.add_fact("test", "Fact to be deleted", None).await;
    assert!(add_result.is_ok());

    if let Ok(fact) = add_result {
        let delete_result = service.delete_fact(&fact.id).await;
        assert!(delete_result.is_ok(), "Should be able to delete fact");
    }
}
