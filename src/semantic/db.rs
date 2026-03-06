// Semantic Database Layer

use crate::error::{Result, ServerError};
use crate::semantic::SemanticFact;
use rusqlite::{Connection, params, OptionalExtension};
use std::path::Path;
use uuid::Uuid;

/// SQLite-based semantic fact storage
pub struct SemanticDB {
    conn: Connection,
}

impl SemanticDB {
    /// Create new database connection and initialize schema
    pub fn new(base_dir: &str) -> Result<Self> {
        let db_path = Path::new(base_dir).join("semantic.db");

        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(db_path)?;

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

        // Migrate existing databases — no-op if column already present
        let has_source: bool = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('facts') WHERE name='source'",
            [],
            |row| row.get::<_, i64>(0),
        )? > 0;
        if !has_source {
            conn.execute("ALTER TABLE facts ADD COLUMN source TEXT", [])?;
        }

        conn.execute(
            "CREATE TABLE IF NOT EXISTS embeddings (
                fact_id TEXT PRIMARY KEY,
                embedding BLOB NOT NULL,
                FOREIGN KEY (fact_id) REFERENCES facts(id)
            )",
            [],
        )?;

        Ok(Self { conn })
    }

    /// Add a fact and return its generated (id, created_at timestamp).
    pub fn add_fact(&self, fact: &SemanticFact) -> Result<(String, i64)> {
        let fact_id = Uuid::new_v4().to_string();
        let timestamp = chrono::Utc::now().timestamp();

        self.conn.execute(
            "INSERT INTO facts (id, namespace, content, created_at, source) VALUES (?, ?, ?, ?, ?)",
            params![&fact_id, &fact.namespace, &fact.content, timestamp, &fact.source],
        )?;

        let embedding_bytes: Vec<u8> = fact.embedding.iter()
            .flat_map(|f| f.to_le_bytes().to_vec())
            .collect();

        self.conn.execute(
            "INSERT INTO embeddings (fact_id, embedding) VALUES (?, ?)",
            params![&fact_id, &embedding_bytes],
        )?;

        Ok((fact_id, timestamp))
    }

    /// Get a fact by ID, including its embedding.
    pub fn get_fact(&self, fact_id: &str) -> Result<Option<SemanticFact>> {
        let fact = self.conn.query_row(
            "SELECT namespace, content, created_at, source FROM facts WHERE id = ?",
            params![fact_id],
            |row| {
                Ok(SemanticFact {
                    id: fact_id.to_string(),
                    namespace: row.get(0)?,
                    content: row.get(1)?,
                    created_at: row.get(2)?,
                    embedding: vec![],
                    source: row.get(3)?,
                })
            },
        ).optional()?;

        let fact = match fact {
            Some(mut f) => {
                let embedding_bytes: Vec<u8> = self.conn.query_row(
                    "SELECT embedding FROM embeddings WHERE fact_id = ?",
                    params![fact_id],
                    |row| row.get(0),
                ).optional()?.unwrap_or_default();

                f.embedding = decode_embedding(&embedding_bytes)?;
                Some(f)
            }
            None => None,
        };

        Ok(fact)
    }

    /// Get embedding by fact ID.
    pub fn get_embedding(&self, fact_id: &str) -> Result<Option<Vec<f32>>> {
        let embedding_bytes: Vec<u8> = self.conn.query_row(
            "SELECT embedding FROM embeddings WHERE fact_id = ?",
            params![fact_id],
            |row| row.get(0),
        ).optional()?.unwrap_or_default();

        if embedding_bytes.is_empty() {
            return Ok(None);
        }

        Ok(Some(decode_embedding(&embedding_bytes)?))
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
            "SELECT id FROM facts WHERE namespace = ? AND created_at >= ? AND created_at <= ? ORDER BY created_at DESC LIMIT ?",
        )?;

        let fact_ids: Vec<String> = stmt
            .query_map(params![namespace, from, to, limit as i64], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();

        let mut results = vec![];
        for fact_id in fact_ids {
            if let Some(fact) = self.get_fact(&fact_id)? {
                results.push(fact);
            }
        }

        Ok(results)
    }

    /// Update content and/or source of an existing fact. Re-stores the embedding.
    /// Returns true if the fact existed.
    pub fn update_fact(&self, fact_id: &str, content: &str, source: Option<&str>, embedding: &[f32]) -> Result<bool> {
        let updated = self.conn.execute(
            "UPDATE facts SET content = ?, source = ? WHERE id = ?",
            params![content, source, fact_id],
        )?;
        if updated == 0 {
            return Ok(false);
        }
        let embedding_bytes: Vec<u8> = embedding.iter()
            .flat_map(|f| f.to_le_bytes())
            .collect();
        self.conn.execute(
            "UPDATE embeddings SET embedding = ? WHERE fact_id = ?",
            params![&embedding_bytes, fact_id],
        )?;
        Ok(true)
    }

    /// Delete a fact and its embedding. Returns true if the fact existed.
    pub fn delete_fact(&mut self, fact_id: &str) -> Result<bool> {
        let tx = self.conn.transaction()?;

        let count = tx.execute(
            "DELETE FROM facts WHERE id = ?",
            params![fact_id],
        )?;

        tx.execute(
            "DELETE FROM embeddings WHERE fact_id = ?",
            params![fact_id],
        )?;

        tx.commit()?;

        Ok(count > 0)
    }

    /// Get all facts (used by hybrid search).
    pub fn get_all_facts(&self) -> Result<Vec<SemanticFact>> {
        let mut stmt = self.conn.prepare(
            "SELECT id FROM facts ORDER BY created_at DESC",
        )?;

        let fact_ids: Vec<String> = stmt
            .query_map([], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();

        let mut results = vec![];
        for fact_id in fact_ids {
            if let Some(fact) = self.get_fact(&fact_id)? {
                results.push(fact);
            }
        }

        Ok(results)
    }
}

/// Decode a raw byte blob into an f32 vector, returning an error on malformed data
/// instead of panicking.
fn decode_embedding(bytes: &[u8]) -> Result<Vec<f32>> {
    bytes.chunks_exact(4)
        .map(|chunk| -> Result<f32> {
            let arr: [u8; 4] = chunk.try_into().map_err(|_| {
                ServerError::DatabaseError("malformed embedding blob: length not divisible by 4".to_string())
            })?;
            Ok(f32::from_le_bytes(arr))
        })
        .collect()
}
