use sunbeam_memory::{
    config::MemoryConfig,
    embedding::service::EmbeddingModelType,
    memory::service::MemoryService,
    semantic::{SemanticConfig, SemanticStore},
};

async fn setup() -> (MemoryService, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let config = MemoryConfig {
        base_dir: dir.path().to_str().unwrap().to_string(),
        ..Default::default()
    };
    let memory = MemoryService::new(&config).await.unwrap();
    (memory, dir)
}

#[tokio::test]
async fn test_current_model() {
    let (memory, _dir) = setup().await;
    let model = memory.current_model().await;
    assert_eq!(model, EmbeddingModelType::BgeBaseEnglish);
}

#[tokio::test]
async fn test_new_with_model() {
    let dir = tempfile::tempdir().unwrap();
    let config = MemoryConfig {
        base_dir: dir.path().to_str().unwrap().to_string(),
        ..Default::default()
    };
    let memory = MemoryService::new_with_model(&config, EmbeddingModelType::BgeBaseEnglish)
        .await
        .unwrap();
    let model = memory.current_model().await;
    assert_eq!(model, EmbeddingModelType::BgeBaseEnglish);
}

#[tokio::test]
async fn test_switch_model_no_facts() {
    let dir = tempfile::tempdir().unwrap();
    let config = MemoryConfig {
        base_dir: dir.path().to_str().unwrap().to_string(),
        ..Default::default()
    };
    let memory = MemoryService::new(&config).await.unwrap();

    // Switch to a different model (no facts to re-embed, so fast)
    memory
        .switch_model(EmbeddingModelType::CodeBert)
        .await
        .unwrap();
    let model = memory.current_model().await;
    assert_eq!(model, EmbeddingModelType::CodeBert);
}

#[tokio::test]
async fn test_switch_model_with_facts() {
    let dir = tempfile::tempdir().unwrap();
    let config = MemoryConfig {
        base_dir: dir.path().to_str().unwrap().to_string(),
        ..Default::default()
    };
    let memory = MemoryService::new(&config).await.unwrap();

    // Add a fact
    memory.add_fact("test", "hello world", None).await.unwrap();

    // Switch to same dimension model (CodeBert is also 768)
    memory
        .switch_model(EmbeddingModelType::CodeBert)
        .await
        .unwrap();
    let model = memory.current_model().await;
    assert_eq!(model, EmbeddingModelType::CodeBert);

    // Fact should still be searchable
    let results = memory.search_facts("hello", 5, None).await.unwrap();
    assert!(!results.is_empty());
}

#[tokio::test]
async fn test_list_facts_date_filtering() {
    let (memory, _dir) = setup().await;

    memory.add_fact("dated", "fact one", None).await.unwrap();

    // Filter with a future range → no results
    let results = memory
        .list_facts("dated", 10, Some(3000000000), Some(4000000000))
        .await
        .unwrap();
    assert!(results.is_empty());

    // Filter with a past range → no results
    let results = memory
        .list_facts("dated", 10, Some(0), Some(1000))
        .await
        .unwrap();
    assert!(results.is_empty());
}

#[tokio::test]
async fn test_update_fact_preserves_namespace() {
    let (memory, _dir) = setup().await;

    let fact = memory
        .add_fact("original-ns", "original content", None)
        .await
        .unwrap();
    assert_eq!(fact.namespace, "original-ns");

    let updated = memory
        .update_fact(&fact.id, "updated content", None)
        .await
        .unwrap();
    assert_eq!(updated.namespace, "original-ns");
    assert_eq!(updated.content, "updated content");
}

#[tokio::test]
async fn test_update_fact_with_source() {
    let (memory, _dir) = setup().await;

    let fact = memory.add_fact("test", "content", None).await.unwrap();
    let updated = memory
        .update_fact(
            &fact.id,
            "new content",
            Some("urn:smem:code:fs:/home/user/file.rs"),
        )
        .await
        .unwrap();
    assert_eq!(
        updated.source,
        Some("urn:smem:code:fs:/home/user/file.rs".to_string())
    );
}

#[tokio::test]
async fn test_search_facts_namespace_filter() {
    let (memory, _dir) = setup().await;

    memory
        .add_fact("animals", "elephants are big", None)
        .await
        .unwrap();
    memory
        .add_fact("plants", "trees are tall", None)
        .await
        .unwrap();

    let results = memory
        .search_facts("elephants", 5, Some("animals"))
        .await
        .unwrap();
    assert!(!results.is_empty());
    assert_eq!(results[0].namespace, "animals");
}

#[tokio::test]
async fn test_semantic_store_get_base_dir() {
    let dir = tempfile::tempdir().unwrap();
    let config = SemanticConfig {
        base_dir: dir.path().to_str().unwrap().to_string(),
        dimension: 768,
        model_name: "test".to_string(),
    };
    let store = SemanticStore::new(&config).await.unwrap();
    assert_eq!(store.get_base_dir(), config.base_dir);
}

#[tokio::test]
async fn test_semantic_store_dimension() {
    let dir = tempfile::tempdir().unwrap();
    let config = SemanticConfig {
        base_dir: dir.path().to_str().unwrap().to_string(),
        dimension: 768,
        model_name: "test".to_string(),
    };
    let store = SemanticStore::new(&config).await.unwrap();
    assert_eq!(store.dimension(), 768);
}

#[tokio::test]
async fn test_semantic_store_get_fact() {
    let dir = tempfile::tempdir().unwrap();
    let config = SemanticConfig {
        base_dir: dir.path().to_str().unwrap().to_string(),
        dimension: 768,
        model_name: "test".to_string(),
    };
    let store = SemanticStore::new(&config).await.unwrap();

    let fact = store.get_fact("non-existent").await.unwrap();
    assert!(fact.is_none());
}

#[tokio::test]
async fn test_semantic_store_rebuild_vectors_empty() {
    let dir = tempfile::tempdir().unwrap();
    let config = SemanticConfig {
        base_dir: dir.path().to_str().unwrap().to_string(),
        dimension: 768,
        model_name: "test".to_string(),
    };
    let store = SemanticStore::new(&config).await.unwrap();

    let count = store
        .rebuild_vectors(768, |_texts| Ok(vec![vec![0.0; 768]]))
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn test_memory_service_get_store() {
    let (memory, _dir) = setup().await;
    let store = memory.get_store();
    assert!(!store.get_base_dir().is_empty());
}

#[tokio::test]
async fn test_list_facts_with_source() {
    let (memory, _dir) = setup().await;
    memory
        .add_fact(
            "docs",
            "doc content",
            Some("urn:smem:doc:fs:/home/user/file.md"),
        )
        .await
        .unwrap();
    let results = memory.list_facts("docs", 10, None, None).await.unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(
        results[0].source,
        Some("urn:smem:doc:fs:/home/user/file.md".to_string())
    );
}
