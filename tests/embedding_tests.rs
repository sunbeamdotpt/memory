use mcp_server::embedding::service::{EmbeddingService, EmbeddingModelType};

#[tokio::test]
async fn test_bge_base_english_model_works() {
    let service = EmbeddingService::new(EmbeddingModelType::BgeBaseEnglish).await;
    assert!(service.is_ok(), "BGE Base English should be implemented");

    let service = service.unwrap();
    let embeddings = service.embed(&["Test text"]).await.unwrap();
    assert_eq!(embeddings.len(), 1);
    assert_eq!(embeddings[0].len(), 768);
}

#[tokio::test]
async fn test_codebert_model_works() {
    let service = EmbeddingService::new(EmbeddingModelType::CodeBert).await;
    assert!(service.is_ok(), "CodeBERT should be implemented");

    let service = service.unwrap();
    let embeddings = service.embed(&["def test():"]).await.unwrap();
    assert_eq!(embeddings.len(), 1);
    assert_eq!(embeddings[0].len(), 768);
}

#[tokio::test]
async fn test_graphcodebert_model_works() {
    let service = EmbeddingService::new(EmbeddingModelType::GraphCodeBert).await;
    assert!(service.is_ok(), "GraphCodeBERT should be implemented");

    let service = service.unwrap();
    let embeddings = service.embed(&["class Diagram:"]).await.unwrap();
    assert_eq!(embeddings.len(), 1);
    assert_eq!(embeddings[0].len(), 768);
}

#[tokio::test]
async fn test_model_switching_works() {
    use mcp_server::memory::service::MemoryService;
    use mcp_server::config::MemoryConfig;

    let config = MemoryConfig { base_dir: "./tests/data/test_data".to_string(), ..Default::default() };

    let service = MemoryService::new_with_model(
        &config,
        EmbeddingModelType::BgeBaseEnglish,
    ).await.unwrap();

    assert_eq!(service.current_model(), EmbeddingModelType::BgeBaseEnglish);

    let switch_result: Result<(), mcp_server::error::ServerError> =
        service.switch_model(EmbeddingModelType::CodeBert).await;
    assert!(switch_result.is_ok(), "Should be able to switch models");
}
