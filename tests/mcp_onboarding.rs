use rmcp::handler::server::wrapper::Parameters;
/// Acceptance tests that onboard a representative slice of the mcp-server repo
/// through the MCP protocol layer and verify semantic retrieval quality.
///
/// Three scenarios are exercised in separate tests:
///   1. General semantic knowledge  — high-level docs about the server
///   2. Code search                 — exact function signatures and struct definitions
///   3. Code semantic search        — natural-language descriptions of code behaviour
///
/// All requests call SunbeamServer tool methods directly with typed Parameters,
/// exactly as rmcp's own test suite does.  The embedding model is downloaded
/// once per test process and reused from the global MODEL_CACHE, so only the
/// first test incurs the load cost.
///
/// Run with:   cargo test --test mcp_onboarding -- --nocapture
/// (Tests are slow on first run due to model download.)
use sunbeam_memory::{
    config::MemoryConfig,
    core::service::CoreService,
    indexer::{IndexService, IndexWatcher},
    mcp::server::SunbeamServer,
    mcp::{SearchFactsParams, StoreFactParams},
    memory::service::MemoryService,
};

async fn setup_server() -> (SunbeamServer, MemoryService) {
    let dir = tempfile::tempdir().expect("tempdir");
    let config = MemoryConfig {
        base_dir: dir.path().to_str().unwrap().to_string(),
        ..Default::default()
    };
    let memory = MemoryService::new(&config).await.expect("MemoryService");
    let (tx, rx) = crossbeam_channel::bounded(1);
    let watcher = IndexWatcher::new(tx).unwrap();
    let indexer = IndexService::new(memory.clone(), rx, watcher);
    let core = CoreService::new(memory.clone(), indexer);
    let server = SunbeamServer::new(core);
    (server, memory)
}

// ── corpus ────────────────────────────────────────────────────────────────────

/// High-level prose about what the server does and how it works.
const DOCS: &[&str] = &[
    "sunbeam-memory is an MCP server that provides semantic memory over stdio \
     JSON-RPC transport, compatible with any MCP client such as Claude Desktop, Cursor, or Zed",
    "The server reads newline-delimited JSON-RPC 2.0 from stdin and writes \
     responses to stdout; all diagnostic logs go to stderr to avoid contaminating the data stream",
    "Embeddings are generated locally using the BGE-Base-English-v1.5 model via \
     the fastembed library, producing 768-dimensional float vectors",
    "Facts are persisted in a SQLite database and searched using cosine similarity; \
     the in-memory vector index uses a HashMap keyed by fact ID",
    "The server exposes four MCP tools: store_fact to embed and save text, \
     search_facts for semantic similarity search, delete_fact to remove by ID, \
     and list_facts to enumerate a namespace",
    "Namespaces are logical groupings of facts — store code signatures in a 'code' \
     namespace and documentation in a 'docs' namespace and search them independently",
    "The MemoryConfig struct reads the MCP_MEMORY_BASE_DIR environment variable \
     to determine where to store the SQLite database and model cache",
];

/// Actual function signatures and struct definitions from the codebase.
const CODE: &[&str] = &[
    "pub async fn add_fact(&self, namespace: &str, content: &str) -> Result<MemoryFact>",
    "pub async fn search_facts(&self, query: &str, limit: usize, namespace: Option<&str>) -> Result<Vec<MemoryFact>>",
    "pub async fn delete_fact(&self, fact_id: &str) -> Result<bool>",
    "pub async fn list_facts(&self, namespace: &str, limit: usize) -> Result<Vec<MemoryFact>>",
    "pub struct MemoryFact { pub id: String, pub namespace: String, pub content: String, pub created_at: String, pub score: f32 }",
    "pub struct MemoryConfig { pub base_dir: String } // reads MCP_MEMORY_BASE_DIR env var",
    "pub async fn handle(req: &Request, memory: &MemoryService) -> Option<Response> // None for notifications",
    "pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 // dot product divided by product of L2 norms",
    "pub struct SemanticIndex { vectors: HashMap<String, Vec<f32>> } // in-memory cosine index",
    "pub async fn fused_search(&self, query: &str, query_embedding: &[f32], limit: usize, namespace_filter: Option<&str>) -> Result<Vec<(SemanticFact, f32)>>",
];

/// Semantic prose descriptions of what the code does — bridges English queries to code concepts.
const INDEX: &[&str] = &[
    "To embed and persist a piece of text call store_fact; it generates a vector \
     embedding and writes both the text and the embedding bytes to SQLite",
    "To retrieve semantically similar content use search_facts with a natural language \
     query; the query is embedded and stored vectors are ranked by cosine similarity",
    "Deleting a memory removes the row from SQLite and evicts the vector from the \
     in-memory HashMap index so it never appears in future search results",
    "The fused_search operation combines BM25 keyword relevance (via FTS5) with \
     vector similarity using Reciprocal Rank Fusion (RRF) so callers always receive \
     useful results even when one modality produces no matches",
    "Each fact is assigned a UUID as its ID and a Unix timestamp for ordering; \
     list_facts returns facts in a namespace sorted newest-first",
    "Switching embedding models replaces the EmbeddingService held inside a Mutex; \
     the new model is loaded from the fastembed cache before the atomic swap",
];

// ── MCP helpers ───────────────────────────────────────────────────────────────

async fn store(server: &SunbeamServer, namespace: &str, content: &str, source: Option<&str>) {
    let result = server
        .store_fact(Parameters(StoreFactParams {
            content: content.to_string(),
            namespace: Some(namespace.to_string()),
            source: source.map(|s| s.to_string()),
        }))
        .await
        .expect("store_fact should succeed");
    assert!(
        result.is_error != Some(true),
        "store_fact tool error: {}",
        result.content[0].as_text().map(|t| &*t.text).unwrap_or("")
    );
}

/// Returns the text body of the first content block in the tool response.
async fn search(
    server: &SunbeamServer,
    query: &str,
    limit: usize,
    namespace: Option<&str>,
) -> String {
    let result = server
        .search_facts(Parameters(SearchFactsParams {
            query: query.to_string(),
            limit: Some(limit as u64),
            namespace: namespace.map(|s| s.to_string()),
        }))
        .await
        .expect("search_facts should succeed");
    result.content[0]
        .as_text()
        .map(|t| t.text.clone())
        .unwrap_or_default()
}

fn assert_hit(result: &str, expected_terms: &[&str], query: &str) {
    let lower = result.to_lowercase();
    let matched: Vec<&str> = expected_terms
        .iter()
        .copied()
        .filter(|t| lower.contains(&t.to_lowercase()))
        .collect();
    assert!(
        !matched.is_empty(),
        "Query {:?} — expected at least one of {:?} in result, got:\n{}",
        query,
        expected_terms,
        result,
    );
}

// ── test 1: general semantic knowledge ───────────────────────────────────────

#[tokio::test]
async fn test_onboard_general_knowledge() {
    let (server, _memory) = setup_server().await;

    // Onboard: index all docs-namespace facts through the MCP interface.
    for fact in DOCS.iter() {
        store(&server, "docs", fact, None).await;
    }

    let q = "how does this server communicate with clients?";
    let result = search(&server, q, 3, None).await;
    eprintln!("\n── Q: {q}\n{result}");
    assert_hit(&result, &["stdio", "json-rpc", "transport", "stdin"], q);

    let q = "what embedding model is used for vector search?";
    let result = search(&server, q, 3, None).await;
    eprintln!("\n── Q: {q}\n{result}");
    assert_hit(&result, &["bge", "fastembed", "768", "embedding"], q);

    let q = "what operations can I perform with this server?";
    let result = search(&server, q, 3, None).await;
    eprintln!("\n── Q: {q}\n{result}");
    assert_hit(&result, &["store_fact", "search_facts", "four", "tools"], q);

    let q = "where is the data stored on disk?";
    let result = search(&server, q, 3, None).await;
    eprintln!("\n── Q: {q}\n{result}");
    assert_hit(
        &result,
        &["sqlite", "mcp_memory_base_dir", "base_dir", "database"],
        q,
    );
}

// ── test 2: code search ───────────────────────────────────────────────────────

#[tokio::test]
async fn test_onboard_code_search() {
    let (server, _memory) = setup_server().await;

    // URNs pointing to the actual source files for each CODE fact.
    const CODE_URNS: &[&str] = &[
        "urn:smem:code:fs:/Users/sienna/Development/sunbeam/mcp-server/src/memory/service.rs",
        "urn:smem:code:fs:/Users/sienna/Development/sunbeam/mcp-server/src/memory/service.rs",
        "urn:smem:code:fs:/Users/sienna/Development/sunbeam/mcp-server/src/memory/service.rs",
        "urn:smem:code:fs:/Users/sienna/Development/sunbeam/mcp-server/src/memory/service.rs",
        "urn:smem:code:fs:/Users/sienna/Development/sunbeam/mcp-server/src/memory/service.rs",
        "urn:smem:code:fs:/Users/sienna/Development/sunbeam/mcp-server/src/config.rs",
        "urn:smem:code:fs:/Users/sienna/Development/sunbeam/mcp-server/src/mcp/server.rs",
        "urn:smem:code:fs:/Users/sienna/Development/sunbeam/mcp-server/src/semantic/index.rs",
        "urn:smem:code:fs:/Users/sienna/Development/sunbeam/mcp-server/src/semantic/index.rs",
        "urn:smem:code:fs:/Users/sienna/Development/sunbeam/mcp-server/src/semantic/store.rs",
    ];
    for (i, fact) in CODE.iter().enumerate() {
        store(&server, "code", fact, Some(CODE_URNS[i])).await;
    }

    // Code search: function signatures and types by name / shape

    let q = "search_facts function signature";
    let result = search(&server, q, 3, Some("code")).await;
    eprintln!("\n── Q: {q}\n{result}");
    assert_hit(&result, &["search_facts", "result", "vec"], q);

    let q = "MemoryFact struct fields";
    let result = search(&server, q, 3, Some("code")).await;
    eprintln!("\n── Q: {q}\n{result}");
    assert_hit(&result, &["memoryfact", "namespace", "score", "content"], q);

    let q = "delete a fact by id";
    let result = search(&server, q, 3, Some("code")).await;
    eprintln!("\n── Q: {q}\n{result}");
    assert_hit(&result, &["delete_fact", "bool", "result"], q);

    let q = "cosine similarity calculation";
    let result = search(&server, q, 3, Some("code")).await;
    eprintln!("\n── Q: {q}\n{result}");
    assert_hit(&result, &["cosine_similarity", "f32", "norm", "dot"], q);

    let q = "fused bm25 and vector search";
    let result = search(&server, q, 3, Some("code")).await;
    eprintln!("\n── Q: {q}\n{result}");
    assert_hit(&result, &["fused_search", "bm25", "embedding", "rrf"], q);

    // Verify source URNs appear in results
    let q = "function signature for adding facts";
    let result = search(&server, q, 3, Some("code")).await;
    eprintln!("\n── source URN check:\n{result}");
    assert!(
        result.contains("urn:smem:code:fs:"),
        "Search results should include source URN, got:\n{result}"
    );
}

// ── test 3: code semantic search ─────────────────────────────────────────────

#[tokio::test]
async fn test_onboard_code_semantic() {
    let (server, _memory) = setup_server().await;

    for fact in INDEX.iter() {
        store(&server, "index", fact, None).await;
    }

    // Natural-language queries against semantic descriptions of code behaviour

    let q = "how do I save text to memory?";
    let result = search(&server, q, 3, Some("index")).await;
    eprintln!("\n── Q: {q}\n{result}");
    assert_hit(&result, &["store_fact", "embed", "persist", "sqlite"], q);

    let q = "finding the most relevant stored content";
    let result = search(&server, q, 3, Some("index")).await;
    eprintln!("\n── Q: {q}\n{result}");
    assert_hit(
        &result,
        &["cosine", "similarity", "search_facts", "ranked"],
        q,
    );

    let q = "what happens when I delete a fact?";
    let result = search(&server, q, 3, Some("index")).await;
    eprintln!("\n── Q: {q}\n{result}");
    assert_hit(
        &result,
        &["sqlite", "evict", "hashmap", "delete", "index"],
        q,
    );

    let q = "searching with a keyword plus vector";
    let result = search(&server, q, 3, Some("index")).await;
    eprintln!("\n── Q: {q}\n{result}");
    assert_hit(&result, &["fused", "keyword", "vector", "bm25", "rrf"], q);
}
