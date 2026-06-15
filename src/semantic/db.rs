// Semantic Database Layer — backed by SQLite for metadata/FTS5 and USearch for HNSW vector search.

use crate::error::{Result, ServerError};
use crate::semantic::SemanticFact;
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::HashMap;
use std::path::Path;
use ulid::Ulid;

/// SQLite-based semantic fact storage with USearch HNSW vector indexing.
pub struct SemanticDB {
    conn: Connection,
    dimension: usize,
    index: usearch::Index,
    key_to_fact: HashMap<u64, String>,
    fact_to_key: HashMap<String, u64>,
}

impl SemanticDB {
    /// Create new database connection, initialize schema, load or create USearch index.
    pub fn new(base_dir: &str, dimension: usize) -> Result<Self> {
        let db_path = Path::new(base_dir).join("semantic.db");
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(db_path)?;

        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA busy_timeout = 5000;
             PRAGMA foreign_keys = ON;",
        )?;

        // Metadata table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS _meta (
                key TEXT PRIMARY KEY,
                value TEXT
            )",
            [],
        )?;

        // Facts table (metadata only)
        conn.execute(
            "CREATE TABLE IF NOT EXISTS facts (
                id TEXT PRIMARY KEY,
                namespace TEXT NOT NULL,
                content TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                source TEXT
            )",
            [],
        )?;

        // Migrate existing databases — add source column if missing
        let has_source: bool = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('facts') WHERE name='source'",
            [],
            |row| row.get::<_, i64>(0),
        )? > 0;
        if !has_source {
            conn.execute("ALTER TABLE facts ADD COLUMN source TEXT", [])?;
        }

        // Migrate existing databases — add stale column if missing
        let has_stale: bool = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('facts') WHERE name='stale'",
            [],
            |row| row.get::<_, i64>(0),
        )? > 0;
        if !has_stale {
            conn.execute("ALTER TABLE facts ADD COLUMN stale INTEGER DEFAULT 0", [])?;
        }

        // Ingestion targets table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS ingestion_targets (
                id TEXT PRIMARY KEY,
                path TEXT NOT NULL UNIQUE,
                target_type TEXT NOT NULL CHECK(target_type IN ('file','directory','git_repo')),
                namespace TEXT NOT NULL DEFAULT 'default',
                enabled INTEGER NOT NULL DEFAULT 1,
                created_at INTEGER NOT NULL,
                last_scan_at INTEGER,
                last_scan_git_branch TEXT,
                last_scan_git_commit TEXT
            )",
            [],
        )?;

        // Tracked files table (for cleanup and branch tracking)
        conn.execute(
            "CREATE TABLE IF NOT EXISTS tracked_files (
                id TEXT PRIMARY KEY,
                target_id TEXT NOT NULL REFERENCES ingestion_targets(id) ON DELETE CASCADE,
                repo_path TEXT NOT NULL,
                last_seen_branch TEXT NOT NULL,
                last_seen_commit TEXT,
                last_seen_at INTEGER NOT NULL,
                UNIQUE(target_id, repo_path)
            )",
            [],
        )?;

        // USearch key mapping table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS _usearch_keys (
                ukey INTEGER PRIMARY KEY,
                fact_id TEXT NOT NULL UNIQUE
            )",
            [],
        )?;

        // USearch index blob storage (single row)
        conn.execute(
            "CREATE TABLE IF NOT EXISTS _usearch_index (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                blob BLOB
            )",
            [],
        )?;

        // FTS5 full-text index for BM25 keyword search (external content table)
        let has_old_fts: bool = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='fts_facts' AND sql NOT LIKE '%content=%'",
            [],
            |row| row.get::<_, i64>(0),
        )? > 0;
        if has_old_fts {
            conn.execute("DROP TABLE IF EXISTS fts_facts", [])?;
        }

        conn.execute(
            "CREATE VIRTUAL TABLE IF NOT EXISTS fts_facts USING fts5(content, content='facts', content_rowid='rowid')",
            [],
        )?;

        // Triggers to keep fts_facts in sync with facts
        conn.execute(
            "CREATE TRIGGER IF NOT EXISTS fts_facts_insert AFTER INSERT ON facts BEGIN
                INSERT INTO fts_facts(rowid, content) VALUES (new.rowid, new.content);
             END",
            [],
        )?;
        conn.execute(
            "CREATE TRIGGER IF NOT EXISTS fts_facts_delete AFTER DELETE ON facts BEGIN
                INSERT INTO fts_facts(fts_facts, rowid) VALUES ('delete', old.rowid);
             END",
            [],
        )?;
        conn.execute(
            "CREATE TRIGGER IF NOT EXISTS fts_facts_update AFTER UPDATE ON facts BEGIN
                INSERT INTO fts_facts(fts_facts, rowid) VALUES ('delete', old.rowid);
                INSERT INTO fts_facts(rowid, content) VALUES (new.rowid, new.content);
             END",
            [],
        )?;

        // Errors table (for LLM-queriable failure logs)
        conn.execute(
            "CREATE TABLE IF NOT EXISTS errors (
                id TEXT PRIMARY KEY,
                timestamp INTEGER NOT NULL,
                component TEXT NOT NULL,
                severity TEXT NOT NULL CHECK(severity IN ('warn','error')),
                message TEXT NOT NULL,
                details TEXT,
                resolved INTEGER NOT NULL DEFAULT 0
            )",
            [],
        )?;

        // Backfill fts_facts for existing databases
        Self::maybe_migrate_fts(&conn)?;

        // Load or create USearch index
        let (index, key_to_fact, fact_to_key) = Self::load_or_create_index(&conn, dimension)?;

        // Store dimension in meta
        conn.execute(
            "INSERT OR REPLACE INTO _meta (key, value) VALUES ('dimension', ?)",
            params![dimension.to_string()],
        )?;

        Ok(Self {
            conn,
            dimension,
            index,
            key_to_fact,
            fact_to_key,
        })
    }

    /// Load an existing USearch index from SQLite blob, or create a fresh one.
    fn load_or_create_index(
        conn: &Connection,
        dimension: usize,
    ) -> Result<(usearch::Index, HashMap<u64, String>, HashMap<String, u64>)> {
        let mut options = usearch::IndexOptions::default();
        options.dimensions = dimension;
        options.metric = usearch::MetricKind::Cos;
        options.quantization = usearch::ScalarKind::F32;
        options.connectivity = 16;
        options.expansion_add = 40;
        options.expansion_search = 16;

        let index = usearch::new_index(&options)?;

        let blob: Option<Vec<u8>> = conn
            .query_row("SELECT blob FROM _usearch_index WHERE id = 1", [], |row| {
                row.get(0)
            })
            .optional()?;

        if let Some(data) = blob {
            if !data.is_empty() {
                index.load_from_buffer(&data)?;
            }
        }

        // Reserve capacity after loading — load_from_buffer overwrites reservations.
        let existing_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM _usearch_keys", [], |row| row.get(0))
            .unwrap_or(0);
        let reserve = (existing_count + 1000) as usize;
        index.reserve(reserve)?;

        let mut key_to_fact = HashMap::new();
        let mut fact_to_key = HashMap::new();

        let mut stmt = conn.prepare("SELECT ukey, fact_id FROM _usearch_keys")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, i64>(0)? as u64, row.get::<_, String>(1)?))
        })?;

        for row in rows {
            let (ukey, fact_id) = row?;
            key_to_fact.insert(ukey, fact_id.clone());
            fact_to_key.insert(fact_id, ukey);
        }

        Ok((index, key_to_fact, fact_to_key))
    }

    /// Persist the in-memory USearch index blob to SQLite.
    fn save_index_blob(&self) -> Result<()> {
        let len = self.index.serialized_length();
        if len == 0 {
            // Empty index — store a zero-length blob so we don't try to load stale data
            self.conn.execute(
                "INSERT OR REPLACE INTO _usearch_index (id, blob) VALUES (1, ?)",
                params![&[] as &[u8]],
            )?;
            return Ok(());
        }
        let mut buf = vec![0u8; len];
        self.index.save_to_buffer(&mut buf)?;
        self.conn.execute(
            "INSERT OR REPLACE INTO _usearch_index (id, blob) VALUES (1, ?)",
            params![buf],
        )?;
        Ok(())
    }

    /// Allocate a new USearch key.
    fn next_ukey(&self) -> Result<u64> {
        let next: i64 = self.conn.query_row(
            "SELECT COALESCE(MAX(ukey), 0) + 1 FROM _usearch_keys",
            [],
            |row| row.get(0),
        )?;
        Ok(next as u64)
    }

    /// Backfill fts_facts for existing databases that predate the FTS5 index.
    fn maybe_migrate_fts(conn: &Connection) -> Result<()> {
        let fts_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM fts_facts", [], |row| row.get(0))
            .unwrap_or(0);

        let facts_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM facts", [], |row| row.get(0))
            .unwrap_or(0);

        if fts_count == 0 && facts_count > 0 {
            conn.execute(
                "INSERT INTO fts_facts(rowid, content) SELECT rowid, content FROM facts",
                [],
            )?;
            let err_id = Ulid::new().to_string();
            let _ = conn.execute(
                "INSERT INTO errors (id, timestamp, component, severity, message, details, resolved) VALUES (?, ?, ?, ?, ?, ?, ?)",
                params![err_id, chrono::Utc::now().timestamp(), "semantic", "warn", format!("backfilled {} facts into fts_facts", facts_count), None::<&str>, 0],
            );
        }
        Ok(())
    }

    /// Add a fact and return its generated (id, created_at timestamp).
    pub fn add_fact(&mut self, fact: &SemanticFact) -> Result<(String, i64)> {
        if fact.embedding.len() != self.dimension {
            return Err(ServerError::InvalidArgument(format!(
                "embedding dimension {} does not match expected {}",
                fact.embedding.len(),
                self.dimension
            )));
        }

        let fact_id = if fact.id.is_empty() {
            Ulid::new().to_string()
        } else {
            fact.id.clone()
        };
        let timestamp = chrono::Utc::now().timestamp();
        let ukey = self.next_ukey()?;

        // Update in-memory index first (so we can serialize it)
        self.index.add(ukey, &fact.embedding)?;

        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO facts (id, namespace, content, created_at, source) VALUES (?, ?, ?, ?, ?)",
            params![
                &fact_id,
                &fact.namespace,
                &fact.content,
                timestamp,
                &fact.source
            ],
        )?;
        tx.execute(
            "INSERT INTO _usearch_keys (ukey, fact_id) VALUES (?, ?)",
            params![ukey as i64, &fact_id],
        )?;

        // Save blob inside transaction for atomicity
        let len = self.index.serialized_length();
        if len == 0 {
            tx.execute(
                "INSERT OR REPLACE INTO _usearch_index (id, blob) VALUES (1, ?)",
                params![&[] as &[u8]],
            )?;
        } else {
            let mut buf = vec![0u8; len];
            self.index.save_to_buffer(&mut buf)?;
            tx.execute(
                "INSERT OR REPLACE INTO _usearch_index (id, blob) VALUES (1, ?)",
                params![buf],
            )?;
        }

        tx.commit()?;

        self.key_to_fact.insert(ukey, fact_id.clone());
        self.fact_to_key.insert(fact_id.clone(), ukey);

        Ok((fact_id, timestamp))
    }

    /// Get a fact by ID, including its embedding.
    pub fn get_fact(&self, fact_id: &str) -> Result<Option<SemanticFact>> {
        let fact = self
            .conn
            .query_row(
                "SELECT id, namespace, content, created_at, source FROM facts WHERE id = ?",
                params![fact_id],
                |row| {
                    Ok(SemanticFact {
                        id: row.get(0)?,
                        namespace: row.get(1)?,
                        content: row.get(2)?,
                        created_at: row.get(3)?,
                        embedding: vec![],
                        source: row.get(4)?,
                    })
                },
            )
            .optional()?;

        match fact {
            Some(mut f) => {
                let ukey = self.fact_to_key.get(fact_id).ok_or_else(|| {
                    ServerError::DatabaseError(format!("missing usearch key for fact {fact_id}"))
                })?;
                let mut vec = Vec::new();
                let found = self.index.export(*ukey, &mut vec)?;
                if found == 0 {
                    return Err(ServerError::DatabaseError(format!(
                        "missing embedding for fact {fact_id}"
                    )));
                }
                f.embedding = vec;
                Ok(Some(f))
            }
            None => Ok(None),
        }
    }

    /// Search facts by namespace with optional Unix timestamp bounds.
    pub fn search_by_namespace(
        &self,
        namespace: &str,
        limit: usize,
        from_ts: Option<i64>,
        to_ts: Option<i64>,
    ) -> Result<Vec<SemanticFact>> {
        let from = from_ts.unwrap_or(i64::MIN);
        let to = to_ts.unwrap_or(i64::MAX);

        let mut stmt = self.conn.prepare(
            "SELECT id, namespace, content, created_at, source
             FROM facts
             WHERE namespace = ? AND created_at >= ? AND created_at <= ? AND (stale = 0 OR stale IS NULL)
             ORDER BY created_at DESC LIMIT ?"
        )?;

        let facts = stmt
            .query_map(params![namespace, from, to, limit as i64], |row| {
                Ok(SemanticFact {
                    id: row.get(0)?,
                    namespace: row.get(1)?,
                    content: row.get(2)?,
                    created_at: row.get(3)?,
                    embedding: vec![],
                    source: row.get(4)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, rusqlite::Error>>()?;

        Ok(facts)
    }

    /// Vector similarity search via USearch HNSW. Returns facts with cosine distance score.
    pub fn search_similar(
        &self,
        query_embedding: &[f32],
        limit: usize,
        namespace_filter: Option<&str>,
    ) -> Result<Vec<(SemanticFact, f32)>> {
        if query_embedding.len() != self.dimension {
            return Err(ServerError::InvalidArgument(format!(
                "query embedding dimension {} does not match expected {}",
                query_embedding.len(),
                self.dimension
            )));
        }

        let k = if namespace_filter.is_some() {
            (limit * 10).max(50)
        } else {
            limit
        };

        let matches = self.index.search(query_embedding, k)?;
        let mut results = Vec::new();

        for (ukey, distance) in matches.keys.iter().zip(matches.distances.iter()) {
            let fact_id = match self.key_to_fact.get(ukey) {
                Some(id) => id,
                None => continue,
            };

            let fact = match self.conn.query_row(
                "SELECT id, namespace, content, created_at, source FROM facts WHERE id = ? AND (stale = 0 OR stale IS NULL)",
                params![fact_id],
                |row| {
                    Ok(SemanticFact {
                        id: row.get(0)?,
                        namespace: row.get(1)?,
                        content: row.get(2)?,
                        created_at: row.get(3)?,
                        embedding: vec![],
                        source: row.get(4)?,
                    })
                },
            ).optional()? {
                Some(f) => f,
                None => continue,
            };

            if let Some(ns) = namespace_filter {
                if fact.namespace != ns {
                    continue;
                }
            }

            // USearch Cos metric returns distance; convert to similarity score
            let score = (1.0 - *distance).clamp(0.0, 1.0);
            results.push((fact, score));

            if results.len() >= limit {
                break;
            }
        }

        Ok(results)
    }

    /// BM25 keyword search via FTS5. Returns facts ordered by BM25 rank (best first).
    pub fn search_bm25(
        &self,
        query: &str,
        limit: usize,
        namespace_filter: Option<&str>,
    ) -> Result<Vec<SemanticFact>> {
        let sanitized = sanitize_fts5_query(query);
        let raw_query = if sanitized.trim().is_empty() {
            query
        } else {
            &sanitized
        };

        let mut results = Vec::new();

        if let Some(ns) = namespace_filter {
            let mut stmt = self.conn.prepare(
                "SELECT f.id, f.namespace, f.content, f.created_at, f.source
                 FROM fts_facts fts
                 JOIN facts f ON f.rowid = fts.rowid
                 WHERE fts.content MATCH ? AND f.namespace = ? AND (f.stale = 0 OR f.stale IS NULL)
                 ORDER BY rank
                 LIMIT ?",
            )?;
            let rows = stmt.query_map(params![raw_query, ns, limit as i64], |row| {
                Ok(SemanticFact {
                    id: row.get(0)?,
                    namespace: row.get(1)?,
                    content: row.get(2)?,
                    created_at: row.get(3)?,
                    embedding: vec![],
                    source: row.get(4)?,
                })
            });
            match rows {
                Ok(r) => {
                    for row in r {
                        results.push(row?);
                    }
                }
                Err(_) => {} // FTS syntax error — return empty
            }
        } else {
            let mut stmt = self.conn.prepare(
                "SELECT f.id, f.namespace, f.content, f.created_at, f.source
                 FROM fts_facts fts
                 JOIN facts f ON f.rowid = fts.rowid
                 WHERE fts.content MATCH ? AND (f.stale = 0 OR f.stale IS NULL)
                 ORDER BY rank
                 LIMIT ?",
            )?;
            let rows = stmt.query_map(params![raw_query, limit as i64], |row| {
                Ok(SemanticFact {
                    id: row.get(0)?,
                    namespace: row.get(1)?,
                    content: row.get(2)?,
                    created_at: row.get(3)?,
                    embedding: vec![],
                    source: row.get(4)?,
                })
            });
            match rows {
                Ok(r) => {
                    for row in r {
                        results.push(row?);
                    }
                }
                Err(_) => {} // FTS syntax error — return empty
            }
        }

        Ok(results)
    }

    /// Fused BM25 + vector search via Reciprocal Rank Fusion (RRF, k=60).
    pub fn fused_search(
        &self,
        query: &str,
        query_embedding: &[f32],
        limit: usize,
        namespace_filter: Option<&str>,
    ) -> Result<Vec<(SemanticFact, f32)>> {
        if query_embedding.len() != self.dimension {
            return Err(ServerError::InvalidArgument(format!(
                "query embedding dimension {} does not match expected {}",
                query_embedding.len(),
                self.dimension
            )));
        }

        const RRF_K: f32 = 60.0;
        const FETCH_MULTIPLIER: usize = 3;

        let vec_results =
            self.search_similar(query_embedding, limit * FETCH_MULTIPLIER, namespace_filter)?;
        let bm25_results = self.search_bm25(query, limit * FETCH_MULTIPLIER, namespace_filter)?;

        let mut facts: HashMap<String, SemanticFact> =
            HashMap::with_capacity(vec_results.len() + bm25_results.len());

        for (fact, _) in &vec_results {
            facts.entry(fact.id.clone()).or_insert_with(|| fact.clone());
        }
        for fact in &bm25_results {
            facts.entry(fact.id.clone()).or_insert_with(|| fact.clone());
        }

        let mut rrf_scores: HashMap<String, f32> = HashMap::with_capacity(facts.len());

        for (rank, (fact, _)) in vec_results.iter().enumerate() {
            let score = 1.0 / (RRF_K + rank as f32);
            *rrf_scores.entry(fact.id.clone()).or_insert(0.0) += score;
        }

        for (rank, fact) in bm25_results.iter().enumerate() {
            let score = 1.0 / (RRF_K + rank as f32);
            *rrf_scores.entry(fact.id.clone()).or_insert(0.0) += score;
        }

        let mut scored: Vec<(String, f32)> = rrf_scores.into_iter().collect();
        scored.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0)) // stable tie-break by fact ID ascending
        });
        scored.truncate(limit);

        Ok(scored
            .into_iter()
            .filter_map(|(id, score)| facts.remove(&id).map(|f| (f, score)))
            .collect())
    }

    /// Update content and/or source of an existing fact. Re-stores the embedding.
    /// Returns true if the fact existed.
    pub fn update_fact(
        &mut self,
        fact_id: &str,
        content: &str,
        source: Option<&str>,
        embedding: &[f32],
    ) -> Result<bool> {
        if embedding.len() != self.dimension {
            return Err(ServerError::InvalidArgument(format!(
                "embedding dimension {} does not match expected {}",
                embedding.len(),
                self.dimension
            )));
        }

        let ukey = match self.fact_to_key.get(fact_id) {
            Some(&k) => k,
            None => return Ok(false),
        };

        self.index.remove(ukey)?;
        self.index.add(ukey, embedding)?;

        let tx = self.conn.transaction()?;
        let updated = tx.execute(
            "UPDATE facts SET content = ?, source = ? WHERE id = ?",
            params![content, source, fact_id],
        )?;
        if updated == 0 {
            tx.commit()?;
            return Ok(false);
        }

        let len = self.index.serialized_length();
        if len == 0 {
            tx.execute(
                "INSERT OR REPLACE INTO _usearch_index (id, blob) VALUES (1, ?)",
                params![&[] as &[u8]],
            )?;
        } else {
            let mut buf = vec![0u8; len];
            self.index.save_to_buffer(&mut buf)?;
            tx.execute(
                "INSERT OR REPLACE INTO _usearch_index (id, blob) VALUES (1, ?)",
                params![buf],
            )?;
        }

        tx.commit()?;
        Ok(true)
    }

    /// Delete a fact and its embedding. Returns true if the fact existed.
    pub fn delete_fact(&mut self, fact_id: &str) -> Result<bool> {
        let ukey = match self.fact_to_key.get(fact_id) {
            Some(&k) => k,
            None => return Ok(false),
        };

        self.index.remove(ukey)?;

        let tx = self.conn.transaction()?;
        tx.execute(
            "DELETE FROM _usearch_keys WHERE fact_id = ?",
            params![fact_id],
        )?;
        let count = tx.execute("DELETE FROM facts WHERE id = ?", params![fact_id])?;

        let len = self.index.serialized_length();
        if len == 0 {
            tx.execute(
                "INSERT OR REPLACE INTO _usearch_index (id, blob) VALUES (1, ?)",
                params![&[] as &[u8]],
            )?;
        } else {
            let mut buf = vec![0u8; len];
            self.index.save_to_buffer(&mut buf)?;
            tx.execute(
                "INSERT OR REPLACE INTO _usearch_index (id, blob) VALUES (1, ?)",
                params![buf],
            )?;
        }

        tx.commit()?;

        self.key_to_fact.remove(&ukey);
        self.fact_to_key.remove(fact_id);

        Ok(count > 0)
    }

    /// Get all facts (metadata only), optionally including stale.
    pub fn get_all_facts(&self, include_stale: bool) -> Result<Vec<SemanticFact>> {
        let sql = if include_stale {
            "SELECT id, namespace, content, created_at, source FROM facts ORDER BY created_at DESC"
        } else {
            "SELECT id, namespace, content, created_at, source FROM facts WHERE stale = 0 OR stale IS NULL ORDER BY created_at DESC"
        };
        let mut stmt = self.conn.prepare(sql)?;

        let facts = stmt
            .query_map([], |row| {
                Ok(SemanticFact {
                    id: row.get(0)?,
                    namespace: row.get(1)?,
                    content: row.get(2)?,
                    created_at: row.get(3)?,
                    embedding: vec![],
                    source: row.get(4)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, rusqlite::Error>>()?;

        Ok(facts)
    }

    /// Rebuild the USearch index with a new dimension. Used on model switch.
    pub fn recreate_vec_table(&mut self, new_dimension: usize) -> Result<()> {
        let mut options = usearch::IndexOptions::default();
        options.dimensions = new_dimension;
        options.metric = usearch::MetricKind::Cos;
        options.quantization = usearch::ScalarKind::F32;
        options.connectivity = 16;
        options.expansion_add = 40;
        options.expansion_search = 16;

        self.index = usearch::new_index(&options)?;
        self.index.reserve(1000)?;
        self.dimension = new_dimension;

        self.conn.execute("DELETE FROM _usearch_keys", [])?;
        self.conn.execute("DELETE FROM _usearch_index", [])?;
        self.key_to_fact.clear();
        self.fact_to_key.clear();

        self.conn.execute(
            "INSERT OR REPLACE INTO _meta (key, value) VALUES ('dimension', ?)",
            params![new_dimension.to_string()],
        )?;
        Ok(())
    }

    /// Insert a fact's embedding into the USearch index (used during re-embedding).
    pub fn insert_vec(&mut self, fact_id: &str, embedding: &[f32]) -> Result<()> {
        if embedding.len() != self.dimension {
            return Err(ServerError::InvalidArgument(format!(
                "embedding dimension {} does not match expected {}",
                embedding.len(),
                self.dimension
            )));
        }

        let ukey = if let Some(&k) = self.fact_to_key.get(fact_id) {
            k
        } else {
            let next = self.next_ukey()?;
            self.conn.execute(
                "INSERT INTO _usearch_keys (ukey, fact_id) VALUES (?, ?)",
                params![next as i64, fact_id],
            )?;
            self.key_to_fact.insert(next, fact_id.to_string());
            self.fact_to_key.insert(fact_id.to_string(), next);
            next
        };

        self.index.remove(ukey)?;
        self.index.add(ukey, embedding)?;
        self.save_index_blob()?;
        Ok(())
    }

    pub fn dimension(&self) -> usize {
        self.dimension
    }

    // ── Ingestion target registry ───────────────────────────────────────────────

    pub fn add_ingestion_target(&self, target: &crate::indexer::IngestionTarget) -> Result<()> {
        self.conn.execute(
            "INSERT INTO ingestion_targets (id, path, target_type, namespace, enabled, created_at, last_scan_at, last_scan_git_branch, last_scan_git_commit)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                &target.id,
                &target.path,
                target.target_type.as_str(),
                &target.namespace,
                target.enabled as i32,
                target.created_at,
                target.last_scan_at,
                target.last_scan_git_branch.as_ref(),
                target.last_scan_git_commit.as_ref(),
            ],
        )?;
        Ok(())
    }

    pub fn get_ingestion_target(
        &self,
        id: &str,
    ) -> Result<Option<crate::indexer::IngestionTarget>> {
        let result = self.conn.query_row(
            "SELECT id, path, target_type, namespace, enabled, created_at, last_scan_at, last_scan_git_branch, last_scan_git_commit
             FROM ingestion_targets WHERE id = ?",
            params![id],
            |row| {
                Ok(crate::indexer::IngestionTarget {
                    id: row.get(0)?,
                    path: row.get(1)?,
                    target_type: crate::indexer::TargetType::from_str(&row.get::<_, String>(2)?)
                        .unwrap_or(crate::indexer::TargetType::Directory),
                    namespace: row.get(3)?,
                    enabled: row.get::<_, i32>(4)? != 0,
                    created_at: row.get(5)?,
                    last_scan_at: row.get(6)?,
                    last_scan_git_branch: row.get(7)?,
                    last_scan_git_commit: row.get(8)?,
                })
            },
        ).optional()?;
        Ok(result)
    }

    pub fn get_ingestion_target_by_path(
        &self,
        path: &str,
    ) -> Result<Option<crate::indexer::IngestionTarget>> {
        let result = self.conn.query_row(
            "SELECT id, path, target_type, namespace, enabled, created_at, last_scan_at, last_scan_git_branch, last_scan_git_commit
             FROM ingestion_targets WHERE path = ?",
            params![path],
            |row| {
                Ok(crate::indexer::IngestionTarget {
                    id: row.get(0)?,
                    path: row.get(1)?,
                    target_type: crate::indexer::TargetType::from_str(&row.get::<_, String>(2)?)
                        .unwrap_or(crate::indexer::TargetType::Directory),
                    namespace: row.get(3)?,
                    enabled: row.get::<_, i32>(4)? != 0,
                    created_at: row.get(5)?,
                    last_scan_at: row.get(6)?,
                    last_scan_git_branch: row.get(7)?,
                    last_scan_git_commit: row.get(8)?,
                })
            },
        ).optional()?;
        Ok(result)
    }

    pub fn list_ingestion_targets(&self) -> Result<Vec<crate::indexer::IngestionTarget>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, path, target_type, namespace, enabled, created_at, last_scan_at, last_scan_git_branch, last_scan_git_commit
             FROM ingestion_targets ORDER BY created_at DESC"
        )?;
        let targets = stmt
            .query_map([], |row| {
                Ok(crate::indexer::IngestionTarget {
                    id: row.get(0)?,
                    path: row.get(1)?,
                    target_type: crate::indexer::TargetType::from_str(&row.get::<_, String>(2)?)
                        .unwrap_or(crate::indexer::TargetType::Directory),
                    namespace: row.get(3)?,
                    enabled: row.get::<_, i32>(4)? != 0,
                    created_at: row.get(5)?,
                    last_scan_at: row.get(6)?,
                    last_scan_git_branch: row.get(7)?,
                    last_scan_git_commit: row.get(8)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, rusqlite::Error>>()?;
        Ok(targets)
    }

    pub fn delete_ingestion_target(&self, id: &str) -> Result<bool> {
        let count = self
            .conn
            .execute("DELETE FROM ingestion_targets WHERE id = ?", params![id])?;
        Ok(count > 0)
    }

    pub fn update_target_scan(
        &self,
        id: &str,
        branch: Option<&str>,
        commit: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE ingestion_targets SET last_scan_at = ?, last_scan_git_branch = ?, last_scan_git_commit = ? WHERE id = ?",
            params![chrono::Utc::now().timestamp(), branch, commit, id],
        )?;
        Ok(())
    }

    // ── Tracked files ─────────────────────────────────────────────────────────

    pub fn upsert_tracked_file(
        &self,
        target_id: &str,
        repo_path: &str,
        branch: &str,
        commit: Option<&str>,
    ) -> Result<()> {
        let id = Ulid::new().to_string();
        self.conn.execute(
            "INSERT INTO tracked_files (id, target_id, repo_path, last_seen_branch, last_seen_commit, last_seen_at)
             VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT(target_id, repo_path) DO UPDATE SET
                 last_seen_branch = excluded.last_seen_branch,
                 last_seen_commit = excluded.last_seen_commit,
                 last_seen_at = excluded.last_seen_at",
            params![id, target_id, repo_path, branch, commit, chrono::Utc::now().timestamp()],
        )?;
        Ok(())
    }

    pub fn get_tracked_file(
        &self,
        target_id: &str,
        repo_path: &str,
    ) -> Result<Option<(String, Option<String>, i64)>> {
        let result = self.conn.query_row(
            "SELECT last_seen_branch, last_seen_commit, last_seen_at FROM tracked_files WHERE target_id = ? AND repo_path = ?",
            params![target_id, repo_path],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        ).optional()?;
        Ok(result)
    }

    // ── Stale fact management ─────────────────────────────────────────────────

    pub fn mark_fact_stale(&self, fact_id: &str) -> Result<bool> {
        let count = self
            .conn
            .execute("UPDATE facts SET stale = 1 WHERE id = ?", params![fact_id])?;
        Ok(count > 0)
    }

    pub fn restore_fact(&self, fact_id: &str) -> Result<bool> {
        let count = self
            .conn
            .execute("UPDATE facts SET stale = 0 WHERE id = ?", params![fact_id])?;
        Ok(count > 0)
    }

    pub fn get_fact_by_source(&self, source: &str) -> Result<Option<SemanticFact>> {
        let result = self.conn.query_row(
            "SELECT id, namespace, content, created_at, source FROM facts WHERE source = ? AND (stale = 0 OR stale IS NULL)",
            params![source],
            |row| {
                Ok(SemanticFact {
                    id: row.get(0)?,
                    namespace: row.get(1)?,
                    content: row.get(2)?,
                    created_at: row.get(3)?,
                    embedding: vec![],
                    source: row.get(4)?,
                })
            },
        ).optional()?;
        Ok(result)
    }

    // ── Error logging ─────────────────────────────────────────────────────────

    pub fn log_error(
        &self,
        id: &str,
        component: &str,
        severity: &str,
        message: &str,
        details: Option<&str>,
    ) -> Result<()> {
        let timestamp = chrono::Utc::now().timestamp();
        self.conn.execute(
            "INSERT INTO errors (id, timestamp, component, severity, message, details) VALUES (?, ?, ?, ?, ?, ?)",
            params![id, timestamp, component, severity, message, details],
        )?;
        Ok(())
    }

    pub fn get_recent_errors(
        &self,
        component: Option<&str>,
        limit: usize,
    ) -> Result<Vec<(String, i64, String, String, String, Option<String>)>> {
        let sql = if component.is_some() {
            "SELECT id, timestamp, component, severity, message, details FROM errors WHERE resolved = 0 AND component = ? ORDER BY timestamp DESC LIMIT ?"
        } else {
            "SELECT id, timestamp, component, severity, message, details FROM errors WHERE resolved = 0 ORDER BY timestamp DESC LIMIT ?"
        };
        let mut stmt = self.conn.prepare(sql)?;
        if let Some(c) = component {
            let rows = stmt.query_map(params![c, limit as i64], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            })?;
            let collected: std::result::Result<Vec<_>, rusqlite::Error> = rows.collect();
            Ok(collected?)
        } else {
            let rows = stmt.query_map(params![limit as i64], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            })?;
            let collected: std::result::Result<Vec<_>, rusqlite::Error> = rows.collect();
            Ok(collected?)
        }
    }

    pub fn resolve_error(&self, error_id: &str) -> Result<bool> {
        let count = self.conn.execute(
            "UPDATE errors SET resolved = 1 WHERE id = ?",
            params![error_id],
        )?;
        Ok(count > 0)
    }
}

/// Sanitize a raw query string for FTS5 MATCH to avoid syntax errors.
/// Removes quotes, asterisks, and question marks, collapses whitespace.
fn sanitize_fts5_query(query: &str) -> String {
    query
        .chars()
        .map(|c| {
            if c == '"' || c == '*' || c == '?' {
                ' '
            } else {
                c
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
