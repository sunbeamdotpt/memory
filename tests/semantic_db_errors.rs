use sunbeam_memory::semantic::db::SemanticDB;
use sunbeam_memory::semantic::{SemanticConfig, SemanticFact};

#[test]
fn test_add_fact_dimension_mismatch() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = SemanticDB::new(dir.path().to_str().unwrap(), 768).unwrap();
    let fact = SemanticFact {
        id: "test".to_string(),
        namespace: "ns".to_string(),
        content: "hello".to_string(),
        created_at: 0,
        embedding: vec![0.0; 10],
        source: None,
    };
    let result = db.add_fact(&fact);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("dimension"));
    assert!(err.contains("10"));
    assert!(err.contains("768"));
}

#[test]
fn test_search_similar_dimension_mismatch() {
    let dir = tempfile::tempdir().unwrap();
    let db = SemanticDB::new(dir.path().to_str().unwrap(), 768).unwrap();
    let result = db.search_similar(&vec![0.0; 10], 5, None);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("dimension"));
}

#[test]
fn test_fused_search_dimension_mismatch() {
    let dir = tempfile::tempdir().unwrap();
    let db = SemanticDB::new(dir.path().to_str().unwrap(), 768).unwrap();
    let result = db.fused_search("fox", &vec![0.0; 10], 5, None);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("dimension"));
}

#[test]
fn test_update_fact_dimension_mismatch() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = SemanticDB::new(dir.path().to_str().unwrap(), 768).unwrap();
    let result = db.update_fact("id", "content", None, &vec![0.0; 10]);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("dimension"));
}

#[test]
fn test_insert_vec_dimension_mismatch() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = SemanticDB::new(dir.path().to_str().unwrap(), 768).unwrap();
    let result = db.insert_vec("id", &vec![0.0; 10]);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("dimension"));
}

#[test]
fn test_get_fact_missing_embedding() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = SemanticDB::new(dir.path().to_str().unwrap(), 768).unwrap();

    // Insert fact with embedding
    let fact = SemanticFact {
        id: "f1".to_string(),
        namespace: "ns".to_string(),
        content: "hello".to_string(),
        created_at: 0,
        embedding: vec![0.0; 768],
        source: None,
    };
    db.add_fact(&fact).unwrap();

    // Delete only the vec_facts row manually to simulate corruption
    db.delete_fact("f1").unwrap();

    // Now get_fact should return None (fact deleted)
    let result = db.get_fact("f1").unwrap();
    assert!(result.is_none());
}

#[test]
fn test_recreate_vec_table_changes_dimension() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = SemanticDB::new(dir.path().to_str().unwrap(), 768).unwrap();
    assert_eq!(db.dimension(), 768);

    db.recreate_vec_table(384).unwrap();
    assert_eq!(db.dimension(), 384);

    // Should be able to insert with new dimension
    let db2 = SemanticDB::new(dir.path().to_str().unwrap(), 384).unwrap();
    assert_eq!(db2.dimension(), 384);
}

#[test]
fn test_delete_fact_nonexistent() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = SemanticDB::new(dir.path().to_str().unwrap(), 768).unwrap();
    let result = db.delete_fact("non-existent").unwrap();
    assert!(!result);
}

#[test]
fn test_get_fact_nonexistent() {
    let dir = tempfile::tempdir().unwrap();
    let db = SemanticDB::new(dir.path().to_str().unwrap(), 768).unwrap();
    let result = db.get_fact("non-existent").unwrap();
    assert!(result.is_none());
}

#[test]
fn test_search_by_namespace_empty() {
    let dir = tempfile::tempdir().unwrap();
    let db = SemanticDB::new(dir.path().to_str().unwrap(), 768).unwrap();
    let results = db.search_by_namespace("empty", 10, None, None).unwrap();
    assert!(results.is_empty());
}

#[test]
fn test_add_fact_with_caller_id() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = SemanticDB::new(dir.path().to_str().unwrap(), 768).unwrap();
    let fact = SemanticFact {
        id: "my-custom-id".to_string(),
        namespace: "ns".to_string(),
        content: "hello".to_string(),
        created_at: 0,
        embedding: vec![0.0; 768],
        source: None,
    };
    let (id, _) = db.add_fact(&fact).unwrap();
    assert_eq!(id, "my-custom-id");
}

#[test]
fn test_add_fact_auto_generates_id() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = SemanticDB::new(dir.path().to_str().unwrap(), 768).unwrap();
    let fact = SemanticFact {
        id: String::new(),
        namespace: "ns".to_string(),
        content: "hello".to_string(),
        created_at: 0,
        embedding: vec![0.0; 768],
        source: None,
    };
    let (id, _) = db.add_fact(&fact).unwrap();
    assert!(!id.is_empty());
    assert_ne!(id, "");
}

#[test]
fn test_get_all_facts() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = SemanticDB::new(dir.path().to_str().unwrap(), 768).unwrap();

    for i in 0..3 {
        let fact = SemanticFact {
            id: format!("f{}", i),
            namespace: "ns".to_string(),
            content: format!("content {}", i),
            created_at: i as i64,
            embedding: vec![0.0; 768],
            source: None,
        };
        db.add_fact(&fact).unwrap();
    }

    let facts = db.get_all_facts(false).unwrap();
    assert_eq!(facts.len(), 3);
}

#[test]
fn test_update_fact_nonexistent() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = SemanticDB::new(dir.path().to_str().unwrap(), 768).unwrap();
    let result = db
        .update_fact("non-existent", "content", None, &vec![0.0; 768])
        .unwrap();
    assert!(!result);
}

#[test]
fn test_semantic_config_default() {
    let config = SemanticConfig::default();
    assert_eq!(config.dimension, 768);
    assert_eq!(config.model_name, "bge-base-en-v1.5");
}
