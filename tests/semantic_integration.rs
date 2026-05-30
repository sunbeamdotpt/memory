use mcp_server::semantic::{SemanticConfig, SemanticStore};
use mcp_server::embedding::service::{EmbeddingService, EmbeddingModelType};

#[tokio::test]
async fn test_semantic_store_can_be_created() {
    let dir = tempfile::tempdir().unwrap();
    let config = SemanticConfig {
        base_dir: dir.path().to_str().unwrap().to_string(),
        dimension: 768,
        model_name: "bge-base-en-v1.5".to_string(),
    };

    let result = SemanticStore::new(&config).await;
    assert!(result.is_ok(), "Should be able to create semantic store");
}

#[tokio::test]
async fn test_semantic_store_can_add_and_search_facts() {
    let dir = tempfile::tempdir().unwrap();
    let semantic_config = SemanticConfig {
        base_dir: dir.path().to_str().unwrap().to_string(),
        dimension: 768,
        model_name: "bge-base-en-v1.5".to_string(),
    };

    let embedding_service = EmbeddingService::new(EmbeddingModelType::BgeBaseEnglish)
        .await
        .expect("Should create embedding service");

    let semantic_store = SemanticStore::new(&semantic_config)
        .await
        .expect("Should create semantic store");

    let content = "The quick brown fox jumps over the lazy dog";
    let namespace = "test";

    let embeddings = embedding_service.embed(&[content])
        .await
        .expect("Should generate embeddings");

    let (fact_id, _created_at) = semantic_store
        .add_fact(namespace, content, &embeddings[0], None)
        .await
        .expect("Should add fact to semantic store");

    assert!(!fact_id.is_empty(), "Fact ID should not be empty");

    let query = "A fast fox leaps over a sleepy canine";
    let query_embeddings = embedding_service.embed(&[query])
        .await
        .expect("Should generate query embeddings");

    let results = semantic_store
        .search(&query_embeddings[0], 5, None)
        .await
        .expect("Should search semantic store");

    assert!(!results.is_empty(), "Should find similar facts");
    assert_eq!(results[0].0.id, fact_id, "Should find the added fact");
}

#[tokio::test]
async fn test_semantic_search_with_memory_service_integration() {
    use mcp_server::memory::service::MemoryService;
    use mcp_server::config::MemoryConfig;

    let dir = tempfile::tempdir().unwrap();
    let memory_config = MemoryConfig { base_dir: dir.path().to_str().unwrap().to_string(), ..Default::default() };

    let memory_service = MemoryService::new(&memory_config)
        .await
        .expect("Should create memory service");

    let namespace = "animals";
    let content = "Elephants are the largest land animals";

    let result = memory_service.add_fact(namespace, content, None)
        .await
        .expect("Should add fact with embedding");

    assert!(!result.id.is_empty(), "Should return a valid fact ID");

    let query = "What is the biggest animal on land?";
    let results = memory_service.search_facts(query, 3, None)
        .await
        .expect("Should search facts semantically");

    assert!(!results.is_empty(), "Should find semantically similar facts");
}
