use mcp_server::semantic::store::SemanticStore;
use mcp_server::semantic::SemanticConfig;

#[tokio::test]
async fn test_hybrid_search_combines_keyword_and_vector() {
    let config = SemanticConfig {
        base_dir: "./tests/data/test_hybrid_data".to_string(),
        dimension: 768,
        model_name: "bge-base-en-v1.5".to_string(),
    };

    let store = SemanticStore::new(&config).await.unwrap();

    let embedding1 = vec![1.0_f32; 768];
    let embedding2 = vec![0.0_f32; 768];
    let embedding3 = {
        let mut v = vec![0.0_f32; 768];
        v[767] = 1.0;
        v
    };

    store.add_fact("test_namespace", "Rust programming language", &embedding1, None).await.unwrap();
    store.add_fact("test_namespace", "Python programming language", &embedding2, None).await.unwrap();
    store.add_fact("test_namespace", "JavaScript programming language", &embedding3, None).await.unwrap();
    store.add_fact("other_namespace", "Rust programming is great", &embedding1, None).await.unwrap();

    // Query similar to embedding1 (all 1s)
    let query_embedding = vec![1.0_f32; 768];
    let results = store.hybrid_search("Rust", &query_embedding, 2).await.unwrap();

    assert_eq!(results.len(), 2);
    assert!(results[0].content.contains("Rust"));
    assert!(results[1].content.contains("Rust"));
}

#[tokio::test]
async fn test_hybrid_search_with_no_keyword_matches() {
    let config = SemanticConfig {
        base_dir: "./tests/data/test_hybrid_no_keyword".to_string(),
        dimension: 3,
        model_name: "test".to_string(),
    };

    let store = SemanticStore::new(&config).await.unwrap();

    let embedding = vec![1.0_f32, 0.0, 0.0];
    store.add_fact("test", "Content without keyword", &embedding, None).await.unwrap();

    // Keyword has no matches — falls back to vector search, so results are non-empty
    let query_embedding = vec![1.0_f32, 0.0, 0.0];
    let results = store.hybrid_search("Nonexistent", &query_embedding, 1).await.unwrap();

    assert!(!results.is_empty(), "Should fall back to vector search when keyword matches nothing");
}

#[tokio::test]
async fn test_hybrid_search_with_no_vector_matches() {
    let config = SemanticConfig {
        base_dir: "./tests/data/test_hybrid_no_vector".to_string(),
        dimension: 3,
        model_name: "test".to_string(),
    };

    let store = SemanticStore::new(&config).await.unwrap();

    let embedding = vec![1.0_f32, 0.0, 0.0];
    store.add_fact("test", "Rust programming", &embedding, None).await.unwrap();

    // Orthogonal query vector — keyword still matches
    let query_embedding = vec![0.0_f32, 0.0, 1.0];
    let results = store.hybrid_search("Rust", &query_embedding, 1).await.unwrap();

    assert_eq!(results.len(), 1);
    assert!(results[0].content.contains("Rust"));
}

#[tokio::test]
async fn test_logging_in_unauthenticated_mode() {
    use mcp_server::logging::FileLogger;
    use std::fs;

    let log_path = "./test_unauth_log.txt";
    let _ = fs::remove_file(log_path);

    let logger = FileLogger::new(log_path.to_string());
    logger.log("GET", "/health", "200");

    assert!(fs::metadata(log_path).is_ok());
    let log_content = fs::read_to_string(log_path).unwrap();
    assert!(log_content.contains("GET /health"));
    assert!(log_content.contains("200"));

    fs::remove_file(log_path).ok();
}
