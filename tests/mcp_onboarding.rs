/// Acceptance tests that onboard a representative slice of the mcp-server repo
/// through the MCP protocol layer and verify semantic retrieval quality.
///
/// Three scenarios are exercised in separate tests:
///   1. General semantic knowledge  — high-level docs about the server
///   2. Code search                 — exact function signatures and struct definitions
///   3. Code semantic search        — natural-language descriptions of code behaviour
///
/// All requests go through `handle()` exactly as a real MCP client would.
/// The embedding model is downloaded once per test process and reused from
/// the global MODEL_CACHE, so only the first test incurs the load cost.
///
/// Run with:   cargo test --test mcp_onboarding -- --nocapture
/// (Tests are slow on first run due to model download.)
use mcp_server::{
    config::MemoryConfig,
    memory::service::MemoryService,
    mcp::{protocol::Request, server::handle},
};
use serde_json::{json, Value};

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
    "pub async fn hybrid_search(&self, keyword: &str, query_embedding: &[f32], limit: usize) -> Result<Vec<SemanticFact>>",
];

/// Semantic prose descriptions of what the code does — bridges English queries to code concepts.
const INDEX: &[&str] = &[
    "To embed and persist a piece of text call store_fact; it generates a vector \
     embedding and writes both the text and the embedding bytes to SQLite",
    "To retrieve semantically similar content use search_facts with a natural language \
     query; the query is embedded and stored vectors are ranked by cosine similarity",
    "Deleting a memory removes the row from SQLite and evicts the vector from the \
     in-memory HashMap index so it never appears in future search results",
    "The hybrid_search operation filters facts whose text contains a keyword then \
     ranks those candidates by vector similarity; when no keyword matches it falls \
     back to pure vector search so callers always receive useful results",
    "Each fact is assigned a UUID as its ID and a Unix timestamp for ordering; \
     list_facts returns facts in a namespace sorted newest-first",
    "Switching embedding models replaces the EmbeddingService held inside a Mutex; \
     the new model is loaded from the fastembed cache before the atomic swap",
];

// ── MCP helpers ───────────────────────────────────────────────────────────────

fn req(method: &str, params: Value, id: u64) -> Request {
    serde_json::from_value(json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    }))
    .expect("valid request JSON")
}

async fn store(memory: &MemoryService, namespace: &str, content: &str, source: Option<&str>, id: u64) {
    let mut args = json!({ "namespace": namespace, "content": content });
    if let Some(s) = source {
        args["source"] = json!(s);
    }
    let r = req("tools/call", json!({ "name": "store_fact", "arguments": args }), id);
    let resp = handle(&r, memory).await.expect("response");
    assert!(resp.error.is_none(), "store_fact RPC error: {:?}", resp.error);
    let result = resp.result.as_ref().expect("result");
    assert!(
        !result["isError"].as_bool().unwrap_or(false),
        "store_fact tool error: {}",
        result["content"][0]["text"].as_str().unwrap_or("")
    );
}

/// Returns the text body of the first content block in the tool response.
async fn search(
    memory: &MemoryService,
    query: &str,
    limit: usize,
    namespace: Option<&str>,
    id: u64,
) -> String {
    let mut args = json!({ "query": query, "limit": limit });
    if let Some(ns) = namespace {
        args["namespace"] = json!(ns);
    }
    let r = req("tools/call", json!({ "name": "search_facts", "arguments": args }), id);
    let resp = handle(&r, memory).await.expect("response");
    assert!(resp.error.is_none(), "search_facts RPC error: {:?}", resp.error);
    let result = resp.result.as_ref().expect("result");
    result["content"][0]["text"]
        .as_str()
        .unwrap_or("")
        .to_string()
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
    let dir = tempfile::tempdir().expect("tempdir");
    let config = MemoryConfig { base_dir: dir.path().to_str().unwrap().to_string() , ..Default::default() };
    let memory = MemoryService::new(&config).await.expect("MemoryService");

    // Onboard: index all docs-namespace facts through the MCP interface.
    for (i, fact) in DOCS.iter().enumerate() {
        store(&memory, "docs", fact, None, i as u64).await;
    }

    let q = "how does this server communicate with clients?";
    let result = search(&memory, q, 3, None, 100).await;
    eprintln!("\n── Q: {q}\n{result}");
    assert_hit(&result, &["stdio", "json-rpc", "transport", "stdin"], q);

    let q = "what embedding model is used for vector search?";
    let result = search(&memory, q, 3, None, 101).await;
    eprintln!("\n── Q: {q}\n{result}");
    assert_hit(&result, &["bge", "fastembed", "768", "embedding"], q);

    let q = "what operations can I perform with this server?";
    let result = search(&memory, q, 3, None, 102).await;
    eprintln!("\n── Q: {q}\n{result}");
    assert_hit(&result, &["store_fact", "search_facts", "four", "tools"], q);

    let q = "where is the data stored on disk?";
    let result = search(&memory, q, 3, None, 103).await;
    eprintln!("\n── Q: {q}\n{result}");
    assert_hit(&result, &["sqlite", "mcp_memory_base_dir", "base_dir", "database"], q);
}

// ── test 2: code search ───────────────────────────────────────────────────────

#[tokio::test]
async fn test_onboard_code_search() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config = MemoryConfig { base_dir: dir.path().to_str().unwrap().to_string() , ..Default::default() };
    let memory = MemoryService::new(&config).await.expect("MemoryService");

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
        store(&memory, "code", fact, Some(CODE_URNS[i]), i as u64).await;
    }

    // Code search: function signatures and types by name / shape

    let q = "search_facts function signature";
    let result = search(&memory, q, 3, Some("code"), 100).await;
    eprintln!("\n── Q: {q}\n{result}");
    assert_hit(&result, &["search_facts", "result", "vec"], q);

    let q = "MemoryFact struct fields";
    let result = search(&memory, q, 3, Some("code"), 101).await;
    eprintln!("\n── Q: {q}\n{result}");
    assert_hit(&result, &["memoryfact", "namespace", "score", "content"], q);

    let q = "delete a fact by id";
    let result = search(&memory, q, 3, Some("code"), 102).await;
    eprintln!("\n── Q: {q}\n{result}");
    assert_hit(&result, &["delete_fact", "bool", "result"], q);

    let q = "cosine similarity calculation";
    let result = search(&memory, q, 3, Some("code"), 103).await;
    eprintln!("\n── Q: {q}\n{result}");
    assert_hit(&result, &["cosine_similarity", "f32", "norm", "dot"], q);

    let q = "hybrid keyword and vector search";
    let result = search(&memory, q, 3, Some("code"), 104).await;
    eprintln!("\n── Q: {q}\n{result}");
    assert_hit(&result, &["hybrid_search", "keyword", "embedding"], q);

    // Verify source URNs appear in results
    let q = "function signature for adding facts";
    let result = search(&memory, q, 3, Some("code"), 105).await;
    eprintln!("\n── source URN check:\n{result}");
    assert!(
        result.contains("urn:smem:code:fs:"),
        "Search results should include source URN, got:\n{result}"
    );
}

// ── test 3: code semantic search ─────────────────────────────────────────────

#[tokio::test]
async fn test_onboard_code_semantic() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config = MemoryConfig { base_dir: dir.path().to_str().unwrap().to_string() , ..Default::default() };
    let memory = MemoryService::new(&config).await.expect("MemoryService");

    for (i, fact) in INDEX.iter().enumerate() {
        store(&memory, "index", fact, None, i as u64).await;
    }

    // Natural-language queries against semantic descriptions of code behaviour

    let q = "how do I save text to memory?";
    let result = search(&memory, q, 3, Some("index"), 100).await;
    eprintln!("\n── Q: {q}\n{result}");
    assert_hit(&result, &["store_fact", "embed", "persist", "sqlite"], q);

    let q = "finding the most relevant stored content";
    let result = search(&memory, q, 3, Some("index"), 101).await;
    eprintln!("\n── Q: {q}\n{result}");
    assert_hit(&result, &["cosine", "similarity", "search_facts", "ranked"], q);

    let q = "what happens when I delete a fact?";
    let result = search(&memory, q, 3, Some("index"), 102).await;
    eprintln!("\n── Q: {q}\n{result}");
    assert_hit(&result, &["sqlite", "evict", "hashmap", "delete", "index"], q);

    let q = "searching with a keyword plus vector";
    let result = search(&memory, q, 3, Some("index"), 103).await;
    eprintln!("\n── Q: {q}\n{result}");
    assert_hit(&result, &["hybrid", "keyword", "vector", "cosine", "falls back"], q);
}
