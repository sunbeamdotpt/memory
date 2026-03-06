// Test for semantic search functionality
use mcp_server::semantic::index::SemanticIndex;

#[test]
fn test_semantic_index_cosine_similarity() {
    let mut index = SemanticIndex::new(3);
    
    // Add some test vectors
    index.add_vector(&[1.0, 0.0, 0.0], "vec1");
    index.add_vector(&[0.0, 1.0, 0.0], "vec2");
    index.add_vector(&[0.0, 0.0, 1.0], "vec3");
    index.add_vector(&[0.6, 0.6, 0.0], "vec4");
    
    // Search for vectors similar to [1.0, 0.0, 0.0]
    let results = index.search(&[1.0, 0.0, 0.0], 2);
    
    // vec1 should be most similar (cosine similarity = 1.0)
    // vec4 should be next most similar (cosine similarity = 0.6)
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].0, "vec1");
    assert!(results[0].1 > 0.9); // Should be very similar
    assert_eq!(results[1].0, "vec4");
    assert!(results[1].1 > 0.5); // Should be somewhat similar
}

#[test]
fn test_semantic_index_search_with_fewer_results() {
    let mut index = SemanticIndex::new(2);
    
    // Add only one vector
    index.add_vector(&[1.0, 0.0], "single");
    
    // Search for 3 results when only 1 exists
    let results = index.search(&[1.0, 0.0], 3);
    
    // Should return only 1 result
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0, "single");
    assert!(results[0].1 > 0.9);
}

#[test]
fn test_semantic_index_zero_vector_handling() {
    let mut index = SemanticIndex::new(3);
    
    // Add a zero vector
    index.add_vector(&[0.0, 0.0, 0.0], "zero");
    
    // Search with a non-zero vector
    let results = index.search(&[1.0, 0.0, 0.0], 1);
    
    // Should handle gracefully (similarity should be 0)
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0, "zero");
    assert_eq!(results[0].1, 0.0);
}