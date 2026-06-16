use sunbeam_memory::error::ServerError;
use sunbeam_memory::indexer::{IngestionTarget, TargetType};
use sunbeam_memory::semantic::db::SemanticDB;
use sunbeam_memory::semantic::store::SemanticStore;
use sunbeam_memory::semantic::{SemanticConfig, SemanticFact};
use ulid::Ulid;

/// Build an old-style F32 USearch index blob for migration tests.
fn f32_index_blob(dimension: usize) -> Vec<u8> {
    let options = usearch::IndexOptions {
        dimensions: dimension,
        metric: usearch::MetricKind::Cos,
        quantization: usearch::ScalarKind::F32,
        connectivity: 16,
        expansion_add: 40,
        expansion_search: 16,
        ..Default::default()
    };
    let index = usearch::new_index(&options).unwrap();
    index.reserve(10).unwrap();
    let embedding: Vec<f32> = (0..dimension).map(|i| (i % 10) as f32 / 10.0).collect();
    index.add(1, &embedding).unwrap();
    let mut buf = vec![0u8; index.serialized_length()];
    index.save_to_buffer(&mut buf).unwrap();
    buf
}

fn fact(id: &str, content: &str, embedding: Vec<f32>) -> SemanticFact {
    SemanticFact {
        id: id.to_string(),
        namespace: "ns".to_string(),
        content: content.to_string(),
        created_at: 0,
        embedding,
        source: None,
    }
}

#[test]
fn test_delete_fact_success_and_get_fact_returns_none() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = SemanticDB::new(dir.path().to_str().unwrap(), 768).unwrap();

    let f = fact("f1", "hello", vec![0.0; 768]);
    db.add_fact(&f).unwrap();

    let before = db.get_fact("f1").unwrap();
    assert!(before.is_some());

    let deleted = db.delete_fact("f1").unwrap();
    assert!(deleted);

    let after = db.get_fact("f1").unwrap();
    assert!(after.is_none());

    // Deleting again returns false
    assert!(!db.delete_fact("f1").unwrap());
}

#[test]
fn test_get_fact_by_source() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = SemanticDB::new(dir.path().to_str().unwrap(), 768).unwrap();

    let mut f = fact("f1", "hello", vec![0.0; 768]);
    f.source = Some("urn:source:1".to_string());
    db.add_fact(&f).unwrap();

    let found = db.get_fact_by_source("urn:source:1").unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().id, "f1");

    let not_found = db.get_fact_by_source("urn:source:missing").unwrap();
    assert!(not_found.is_none());
}

#[test]
fn test_ingestion_target_crud() {
    let dir = tempfile::tempdir().unwrap();
    let db = SemanticDB::new(dir.path().to_str().unwrap(), 768).unwrap();

    let target = IngestionTarget {
        id: "t1".to_string(),
        path: "/tmp/project".to_string(),
        target_type: TargetType::Directory,
        namespace: "ns".to_string(),
        enabled: true,
        created_at: 42,
        last_scan_at: None,
        last_scan_git_branch: None,
        last_scan_git_commit: None,
    };

    db.add_ingestion_target(&target).unwrap();

    let found = db.get_ingestion_target("t1").unwrap();
    assert_eq!(found, Some(target.clone()));

    let by_path = db.get_ingestion_target_by_path("/tmp/project").unwrap();
    assert_eq!(by_path, Some(target.clone()));

    let list = db.list_ingestion_targets().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, "t1");

    db.update_target_scan("t1", Some("main"), Some("abc123"))
        .unwrap();
    let updated = db.get_ingestion_target("t1").unwrap().unwrap();
    assert_eq!(updated.last_scan_git_branch, Some("main".to_string()));
    assert_eq!(updated.last_scan_git_commit, Some("abc123".to_string()));
    assert!(updated.last_scan_at.is_some());

    let deleted = db.delete_ingestion_target("t1").unwrap();
    assert!(deleted);
    assert!(db.get_ingestion_target("t1").unwrap().is_none());
    assert!(!db.delete_ingestion_target("t1").unwrap());
}

#[test]
fn test_migration_adds_source_and_stale_columns() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("semantic.db");

    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute(
            "CREATE TABLE facts (
                id TEXT PRIMARY KEY,
                namespace TEXT NOT NULL,
                content TEXT NOT NULL,
                created_at INTEGER NOT NULL
            )",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO facts (id, namespace, content, created_at) VALUES (?, ?, ?, ?)",
            rusqlite::params!["old", "ns", "legacy content", 1],
        )
        .unwrap();
    }

    let db = SemanticDB::new(dir.path().to_str().unwrap(), 768).unwrap();

    // Existing row should now be readable (stale = 0 / NULL by default)
    let all = db.get_all_facts(false).unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].id, "old");
    assert_eq!(all[0].source, None);
}

#[test]
fn test_fts_rebuild_when_missing_content_reference() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("semantic.db");

    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute(
            "CREATE TABLE facts (
                id TEXT PRIMARY KEY,
                namespace TEXT NOT NULL,
                content TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                source TEXT,
                stale INTEGER DEFAULT 0
            )",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO facts (id, namespace, content, created_at) VALUES (?, ?, ?, ?)",
            rusqlite::params!["f1", "ns", "hello world", 1],
        )
        .unwrap();
        // Old-style FTS5 table without external content reference
        conn.execute("CREATE VIRTUAL TABLE fts_facts USING fts5(content)", [])
            .unwrap();
    }

    let mut db = SemanticDB::new(dir.path().to_str().unwrap(), 768).unwrap();

    // The old FTS table was dropped and replaced with an external-content table.
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let sql: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE name='fts_facts'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(sql.contains("content='facts'"));
    }

    // Newly-added facts are indexed by the rebuilt table.
    let f = fact("f2", "new world", vec![0.0; 768]);
    db.add_fact(&f).unwrap();
    let results = db.search_bm25("world", 10, None).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, "f2");
}

#[test]
fn test_get_all_facts_include_stale_and_restore() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = SemanticDB::new(dir.path().to_str().unwrap(), 768).unwrap();

    for i in 0..3 {
        let f = fact(
            &format!("f{}", i),
            &format!("content {}", i),
            vec![0.0; 768],
        );
        db.add_fact(&f).unwrap();
    }

    db.mark_fact_stale("f1").unwrap();

    let without_stale = db.get_all_facts(false).unwrap();
    assert_eq!(without_stale.len(), 2);
    assert!(!without_stale.iter().any(|f| f.id == "f1"));

    let with_stale = db.get_all_facts(true).unwrap();
    assert_eq!(with_stale.len(), 3);
    assert!(with_stale.iter().any(|f| f.id == "f1"));

    // mark_fact_stale returns false for non-existent fact
    assert!(!db.mark_fact_stale("missing").unwrap());

    // restore_fact brings the fact back
    let restored = db.restore_fact("f1").unwrap();
    assert!(restored);
    let without_stale_after_restore = db.get_all_facts(false).unwrap();
    assert_eq!(without_stale_after_restore.len(), 3);

    // restore_fact returns false for non-existent fact
    assert!(!db.restore_fact("missing").unwrap());
}

#[test]
fn test_error_logging_and_resolution() {
    let dir = tempfile::tempdir().unwrap();
    let db = SemanticDB::new(dir.path().to_str().unwrap(), 768).unwrap();

    let id1 = Ulid::new().to_string();
    let id2 = Ulid::new().to_string();

    db.log_error(
        &id1,
        "semantic",
        "error",
        "something failed",
        Some("details"),
    )
    .unwrap();
    db.log_error(&id2, "indexer", "warn", "slow ingest", None)
        .unwrap();

    let all = db.get_recent_errors(None, 10).unwrap();
    assert_eq!(all.len(), 2);

    let semantic = db.get_recent_errors(Some("semantic"), 10).unwrap();
    assert_eq!(semantic.len(), 1);
    assert_eq!(semantic[0].2, "semantic");

    assert!(db.resolve_error(&id1).unwrap());
    assert!(!db.resolve_error("missing").unwrap());

    let unresolved = db.get_recent_errors(None, 10).unwrap();
    assert_eq!(unresolved.len(), 1);
    assert_eq!(unresolved[0].2, "indexer");
}

#[test]
fn test_search_bm25_sanitizes_special_characters() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = SemanticDB::new(dir.path().to_str().unwrap(), 768).unwrap();
    let f = fact("f1", "hello world", vec![0.0; 768]);
    db.add_fact(&f).unwrap();

    // Quotes, asterisks, question marks, and colons are stripped and the
    // remaining token is used for the FTS5 MATCH.
    let results = db.search_bm25("\"world*?\"", 10, None).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, "f1");

    // A colon would normally be interpreted as an FTS5 column filter
    // ("no such column: kms"); it should be sanitized to a literal search.
    let f2 = fact("f2", "kms vault world", vec![0.0; 768]);
    db.add_fact(&f2).unwrap();
    let results = db.search_bm25("kms: vault world", 10, None).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, "f2");

    // A query that is only punctuation should return empty results, not error.
    let results = db.search_bm25("*?:()", 10, None).unwrap();
    assert!(results.is_empty());
}

#[test]
fn test_empty_index_serialization_after_delete() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = SemanticDB::new(dir.path().to_str().unwrap(), 768).unwrap();

    let f = fact("f1", "hello", vec![0.0; 768]);
    db.add_fact(&f).unwrap();
    db.delete_fact("f1").unwrap();

    // Re-opening exercises the empty-blob load path.
    let db2 = SemanticDB::new(dir.path().to_str().unwrap(), 768).unwrap();
    assert_eq!(db2.dimension(), 768);
    let all = db2.get_all_facts(false).unwrap();
    assert!(all.is_empty());
}

#[tokio::test]
async fn test_rebuild_vectors_success() {
    let dir = tempfile::tempdir().unwrap();
    let config = SemanticConfig {
        base_dir: dir.path().to_str().unwrap().to_string(),
        dimension: 768,
        model_name: "test".to_string(),
    };

    let store = SemanticStore::new(&config).await.unwrap();
    store
        .add_fact("ns", "hello", &[0.0; 768], None)
        .await
        .unwrap();

    let count = store
        .rebuild_vectors(4, |contents| {
            Ok(contents.iter().map(|_| vec![1.0, 0.0, 0.0, 0.0]).collect())
        })
        .await
        .unwrap();

    assert_eq!(count, 1);

    // Search still works after dimension change
    let results = store
        .search(&[1.0_f32, 0.0, 0.0, 0.0], 5, None)
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
}

#[tokio::test]
async fn test_rebuild_vectors_error_from_embed_fn() {
    let dir = tempfile::tempdir().unwrap();
    let config = SemanticConfig {
        base_dir: dir.path().to_str().unwrap().to_string(),
        dimension: 768,
        model_name: "test".to_string(),
    };

    let store = SemanticStore::new(&config).await.unwrap();
    store
        .add_fact("ns", "hello", &[0.0; 768], None)
        .await
        .unwrap();

    let result = store
        .rebuild_vectors(4, |_contents| {
            Err(ServerError::MemoryError("boom".to_string()))
        })
        .await;

    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("boom"));
}

#[test]
fn test_f32_index_blob_migrates_to_f16() {
    let dir = tempfile::tempdir().unwrap();
    let base_dir = dir.path().to_str().unwrap();

    // Create a database with an F32 index blob and a matching vector row.
    {
        let mut db = SemanticDB::new(base_dir, 768).unwrap();
        let fact = SemanticFact {
            id: "f1".to_string(),
            namespace: "ns".to_string(),
            content: "migration test".to_string(),
            created_at: 0,
            embedding: (0..768).map(|i| (i % 10) as f32 / 10.0).collect(),
            source: None,
        };
        db.add_fact(&fact).unwrap();
    }

    // Overwrite the saved blob with an F32-serialized index.
    {
        let db_path = std::path::Path::new(base_dir).join("semantic.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let blob = f32_index_blob(768);
        conn.execute(
            "INSERT OR REPLACE INTO _usearch_index (id, blob) VALUES (1, ?)",
            [&blob],
        )
        .unwrap();
    }

    // Reopening should detect the mismatch and rebuild from vectors.
    let db = SemanticDB::new(base_dir, 768).unwrap();
    let results = db.search_similar(&[0.0; 768], 5, None).unwrap();
    assert!(
        results.iter().any(|(f, _)| f.id == "f1"),
        "fact should be searchable after F32->F16 migration"
    );
}
