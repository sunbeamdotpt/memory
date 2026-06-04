use sunbeam_memory::semantic::store::SemanticStore;
use sunbeam_memory::semantic::SemanticConfig;

#[tokio::test]
async fn test_fused_search_combines_bm25_and_vector() {
    let dir = tempfile::tempdir().unwrap();
    let config = SemanticConfig {
        base_dir: dir.path().to_str().unwrap().to_string(),
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

    // Query similar to embedding1 (all 1s) with keyword "Rust"
    let query_embedding = vec![1.0_f32; 768];
    let results = store.fused_search("Rust", &query_embedding, 2, None).await.unwrap();

    assert_eq!(results.len(), 2);
    assert!(results[0].0.content.contains("Rust"));
    assert!(results[1].0.content.contains("Rust"));
}

#[tokio::test]
async fn test_fused_search_no_bm25_matches() {
    let dir = tempfile::tempdir().unwrap();
    let config = SemanticConfig {
        base_dir: dir.path().to_str().unwrap().to_string(),
        dimension: 3,
        model_name: "test".to_string(),
    };

    let store = SemanticStore::new(&config).await.unwrap();

    let embedding = vec![1.0_f32, 0.0, 0.0];
    store.add_fact("test", "Content without keyword", &embedding, None).await.unwrap();

    // Keyword has no BM25 matches — vector search still returns results via RRF
    let query_embedding = vec![1.0_f32, 0.0, 0.0];
    let results = store.fused_search("Nonexistent", &query_embedding, 1, None).await.unwrap();

    assert!(!results.is_empty(), "Should return vector results when BM25 matches nothing");
}

#[tokio::test]
async fn test_fused_search_no_vector_matches() {
    let dir = tempfile::tempdir().unwrap();
    let config = SemanticConfig {
        base_dir: dir.path().to_str().unwrap().to_string(),
        dimension: 3,
        model_name: "test".to_string(),
    };

    let store = SemanticStore::new(&config).await.unwrap();

    let embedding = vec![1.0_f32, 0.0, 0.0];
    store.add_fact("test", "Rust programming", &embedding, None).await.unwrap();

    // Orthogonal query vector — BM25 still matches "Rust"
    let query_embedding = vec![0.0_f32, 0.0, 1.0];
    let results = store.fused_search("Rust", &query_embedding, 1, None).await.unwrap();

    assert_eq!(results.len(), 1);
    assert!(results[0].0.content.contains("Rust"));
}

#[tokio::test]
async fn test_logging_in_unauthenticated_mode() {
    use sunbeam_memory::logging::FileLogger;
    use std::fs;

    let dir = tempfile::tempdir().unwrap();
    let log_path = dir.path().join("test_unauth_log.txt");
    let log_path_str = log_path.to_str().unwrap();

    let logger = FileLogger::new(log_path_str.to_string());
    logger.log("GET", "/health", "200");

    assert!(fs::metadata(log_path_str).is_ok());
    let log_content = fs::read_to_string(log_path_str).unwrap();
    assert!(log_content.contains("GET /health"));
    assert!(log_content.contains("200"));
}
