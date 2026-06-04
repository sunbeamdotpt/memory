use sunbeam_memory::embedding::service::{EmbeddingService, EmbeddingModelType};

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
    use sunbeam_memory::memory::service::MemoryService;
    use sunbeam_memory::config::MemoryConfig;

    let dir = tempfile::tempdir().unwrap();
    let config = MemoryConfig { base_dir: dir.path().to_str().unwrap().to_string(), ..Default::default() };

    let service = MemoryService::new_with_model(
        &config,
        EmbeddingModelType::BgeBaseEnglish,
    ).await.unwrap();

    assert_eq!(service.current_model().await, EmbeddingModelType::BgeBaseEnglish);

    let switch_result: Result<(), sunbeam_memory::error::ServerError> =
        service.switch_model(EmbeddingModelType::CodeBert).await;
    assert!(switch_result.is_ok(), "Should be able to switch models");
}

#[tokio::test]
async fn test_embedding_service_current_model_and_dimensions() {
    let service = EmbeddingService::new(EmbeddingModelType::CodeBert).await.unwrap();
    assert_eq!(service.current_model(), EmbeddingModelType::CodeBert);
    assert_eq!(service.dimensions(), 768);
}

#[test]
fn test_embedding_model_type_properties() {
    assert_eq!(EmbeddingModelType::BgeBaseEnglish.model_name(), "bge-base-en-v1.5");
    assert_eq!(EmbeddingModelType::CodeBert.model_name(), "codebert");
    assert_eq!(EmbeddingModelType::GraphCodeBert.model_name(), "graphcodebert");
    assert_eq!(EmbeddingModelType::BgeBaseEnglish.dimensions(), 768);
    assert_eq!(EmbeddingModelType::CodeBert.dimensions(), 768);
    assert_eq!(EmbeddingModelType::GraphCodeBert.dimensions(), 768);
}
