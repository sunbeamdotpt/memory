# sunbeam-memory — Agent Guide

> Personal semantic memory server for AI assistants. Written in Rust. Store facts, code snippets, notes, and documents with vector embeddings, then search them by meaning. Also watches directories and auto-ingests files.

---

## Project Overview

**Name:** `mcp-server` (crate), binary `sunbeam-memory`  
**Language:** Rust (edition 2021, MSRV ~1.75)  
**Binary entry point:** `src/main.rs`  
**Library root:** `src/lib.rs`

The server exposes an MCP (Model Context Protocol) interface over **stdio** (local, zero-config) or **HTTP** (remote). Under the hood it:

- Embeds text with a local ONNX model (BGE-Base-English-v1.5 via `fastembed`, ~130 MB download on first run).
- Stores facts in **SQLite** (`semantic.db`) with an FTS5 full-text index.
- Builds an **HNSW vector index** via `usearch` and persists it as a blob inside SQLite.
- Watches files/directories/git repos, extracts text from code and PDFs, and auto-ingests them.

### Runtime modes

| Mode | Trigger | Transport | Auth |
|------|---------|-----------|------|
| stdio (default) | no subcommand | stdin/stdout MCP | none (localhost only) |
| HTTP | `http --port <PORT>` | Streamable HTTP `/mcp` + REST endpoints | `MCP_AUTH_TOKEN` bearer or OIDC JWT |

---

## Technology Stack

| Layer | Crate / Tool | Purpose |
|-------|--------------|---------|
| Async runtime | `tokio` (full features) | Async I/O, spawning tasks |
| HTTP server | `actix-web` | REST API and MCP HTTP transport |
| MCP protocol | `rmcp` | Native Rust MCP SDK (stdio + Streamable HTTP) |
| MCP actix-web | `rmcp-actix-web` | Streamable HTTP transport for actix-web |
| Serialization | `serde`, `serde_json` | JSON-RPC and REST payloads |
| Database | `rusqlite` (bundled) | SQLite persistence, FTS5 |
| Vector search | `usearch` | HNSW ANN index (cosine similarity) |
| Embeddings | `fastembed` | Local ONNX embedding models |
| Error handling | `thiserror`, `anyhow` | Typed errors + ergonomic main errors |
| CLI parsing | `clap` (derive feature) | Structured command-line interface |
| File watching | `notify`, `crossbeam-channel` | Async file-system events |
| Git scanning | `gix` | Pure-Rust git repo introspection |
| PDF extraction | `pdf_oxide` | Page-by-page text extraction |
| Hashing | `blake3` | Content-addressed dedup |
| UUIDs | `ulid` | Fact and error IDs |
| Auth | `jsonwebtoken`, `reqwest` | OIDC JWKS fetching + JWT validation |

**External build dependency:** a C++ toolchain is required because `usearch` includes C++ code via `cxx`.

---

## Build and Run

```bash
# Debug build
cargo build

# Release build (recommended)
cargo build --release

# Run HTTP server on port 3456
cargo run --release -- http --port 3456

# Run stdio MCP server (default)
cargo run --release
```

The release binary is at `target/release/sunbeam-memory`.

### Environment variables

| Variable | Default | Description |
|----------|---------|-------------|
| `MCP_MEMORY_BASE_DIR` | platform data dir (`~/Library/Application Support/sunbeam/memory` on macOS) | Where `semantic.db` and model cache live |
| `MCP_AUTH_TOKEN` | unset | Simple bearer token for remote HTTP mode |
| `MCP_OIDC_ISSUER` | unset | OIDC issuer URL; enables JWT validation |
| `MCP_OIDC_AUDIENCE` | unset | Expected `aud` claim (optional) |

---

## Code Organization

```
src/
├── main.rs              # Binary entry point: clap CLI, stdio / HTTP mode dispatch
├── lib.rs               # Public module re-exports
├── config.rs            # MemoryConfig (env-driven)
├── error.rs             # ServerError enum + Result<T>
├── logging.rs           # Simple file logger
├── paths.rs             # Platform directory helpers (cache, data, model cache)
├── semantic.rs          # SemanticConfig + SemanticFact types
├── urn.rs               # Source URN parser/builder/validator (~800 lines)
├── api/                 # HTTP REST layer
│   ├── config.rs        # Actix route registration (REST + MCP Streamable HTTP)
│   ├── handlers.rs      # REST endpoint handlers
│   ├── mcp_http.rs      # MCP Streamable HTTP transport (rmcp) + auth middleware
│   ├── middleware.rs    # Placeholder
│   ├── oidc.rs          # OIDC JWKS fetching & JWT validation
│   └── types.rs         # Request/response DTOs
├── embedding/           # Local embedding model wrapper
│   └── service.rs       # EmbeddingService, EmbeddingModelType, global model cache
├── indexer/             # File watching & ingestion
│   ├── extract.rs       # PDF text extraction
│   ├── git.rs           # Git state inspection + URN building
│   ├── mod.rs           # Public re-exports
│   ├── progress.rs      # Ingestion progress tracking
│   ├── scanner.rs       # Directory/file scanning
│   ├── service.rs       # IndexService (event loop, ingestion pipeline)
│   ├── target.rs        # IngestionTarget types
│   └── watcher.rs       # notify-based file watcher
├── memory/              # Business logic
│   ├── mod.rs
│   ├── service.rs       # MemoryService (add/search/update/delete facts)
│   └── store.rs         # Thin SemanticStore wrapper
├── mcp/                 # MCP protocol implementation
│   ├── mod.rs
│   └── server.rs        # SunbeamServer: rmcp ServerHandler + tool router (~630 lines)
└── semantic/            # Storage engine
    ├── db.rs            # SemanticDB: SQLite schema, FTS5, USearch persistence (~1000 lines)
    ├── search.rs        # Search helpers
    └── store.rs         # SemanticStore: async wrapper around SemanticDB
```

---

## Testing

The project uses the standard Rust test harness plus **nextest** (preferred).

```bash
# Standard runner
cargo test

# Recommended — respects nextest.config.toml
cargo nextest run

# Run only integration tests
cargo nextest run --tests

# Run only lib tests
cargo nextest run --lib

# Benchmarks (Criterion; indexes alice-in-wonderland.txt + all src/**/*.rs)
cargo bench
```

### Test conventions

- Integration tests live in `tests/`; each file is a separate test target.
- Almost every async test uses a `setup()` helper that creates a `tempfile::TempDir`, builds a `MemoryConfig` pointing at that dir, and instantiates `MemoryService` + `IndexService`.
- Tests against the MCP layer construct JSON-RPC `Request` values with `serde_json::json!` and call `mcp::server::handle` directly.
- The project includes `tests/data/` with fixtures for ingestion tests.

### Notable test files

| File | Focus |
|------|-------|
| `tests/mcp_tools.rs` | Full tool call coverage (store/search/update/delete facts, URN tools, indexer tools) |
| `tests/api_integration.rs` | HTTP REST endpoint tests |
| `tests/semantic_db_errors.rs` | Dimension-mismatch and DB error handling |
| `tests/mcp_onboarding.rs` | MCP protocol handshake and onboarding |
| `tests/main_tests.rs` | `process_mcp_line` edge cases + auth config tests |
| `tests/pdf_ingestion.rs` | PDF extraction pipeline |
| `tests/oidc_tests.rs` | OIDC/JWT validation logic |

### nextest configuration (`nextest.config.toml`)

- Default profile: 2 retries, 4–16 threads, 30 s timeout.
- CI profile: 3 retries, 2–8 threads, 60 s timeout.
- `failure-output = immediate` so you see failures as they happen.

---

## Code Style Guidelines

- **Formatting:** `cargo fmt` — enforced in CI (`cargo fmt --all -- --check`).
- **Linting:** `cargo clippy --all-targets -- -D warnings` — warnings are treated as errors in CI.
- **Error type:** `thiserror` enums in `src/error.rs`. `ServerError` is the top-level error. Conversions exist for `rusqlite::Error`, `std::io::Error`, `cxx::Exception`, and `EmbeddingError`.
- **Async:** `tokio` everywhere. CPU-bound embedding calls are done inside `tokio::sync::Mutex` locks on the embedding service.
- **IDs:** ULIDs (`ulid::Ulid::new().to_string()`) for facts and error log entries.
- **Timestamps:** stored as Unix epoch seconds (`INTEGER`) in SQLite, exposed as RFC 3339 strings in APIs.
- **Comments:** `// ── section name ──` dividers are common in larger files.
- **Namespaces:** default namespace is literally `"default"`. Always use `.unwrap_or("default")` or equivalent.

---

## Security Considerations

- **Auth priority:** OIDC > bearer token > localhost-only. If `MCP_OIDC_ISSUER` is set, the server fetches JWKS at startup and validates every HTTP request. If `MCP_AUTH_TOKEN` is set, it requires `Authorization: Bearer <token>`. If neither is set, the server binds to `127.0.0.1` only.
- **Token exposure:** The bearer token is printed to stderr on startup (intentional for local debugging), but do not log it elsewhere.
- **Path traversal:** The indexer only works with absolute paths. `is_likely_binary` and glob matching are used to skip dangerous or irrelevant files.
- **SQL injection:** All SQLite queries use parameterized statements (`params![]`).
- **Self-contained data:** The entire database is a single `semantic.db` file. Back it up by copying the file.

---

## CI / Deployment

The project ships with a custom CI definition in `workflows.yaml` (consumed by an internal `wfe-server` webhook runner, not GitHub Actions). The pipeline has three workflows:

1. **checkout** — clones the repo at a given commit.
2. **lint** — `cargo fmt --check` + `cargo clippy --all-targets -- -D warnings`.
3. **test-unit** — `cargo nextest run --lib` then `cargo nextest run --tests`.
4. **ci** — orchestrates the above.

CI uses `sccache` for Rust compilation caching and runs in a Kubernetes pod with the `src.sunbeam.pt/studio/wfe-ci:latest` image.

**Local `.cargo/config.toml`** sets `rustc-wrapper = "/opt/homebrew/bin/sccache"` (macOS/Homebrew). This path is machine-specific and ignored via `.gitignore`.

---

## Useful Commands Summary

```bash
# Format
cargo fmt

# Lint (warnings = errors in CI)
cargo clippy --all-targets -- -D warnings

# Test (all)
cargo nextest run

# Test (lib only)
cargo nextest run --lib

# Test (integration only)
cargo nextest run --tests

# Benchmark
cargo bench

# Doc
cargo doc --no-deps

# Clean
cargo clean
```

---

## Quick Architecture Map

```
Claude / MCP client
      │
      │  stdio (local)  or  HTTP POST /mcp  (remote)
      ▼
 mcp/server.rs        ← JSON-RPC dispatch, tool handlers
      │
 memory/service.rs    ← embed content, business logic
      │
 semantic/store.rs    ← HNSW vector search (usearch)
 semantic/db.rs       ← SQLite persistence (facts + FTS5 + errors + index blob)
      │
 indexer/             ← file watcher, PDF extractor, git scanner
```

**Search strategy:** `MemoryService::search_facts` performs a **fused BM25 + vector search** via Reciprocal Rank Fusion (RRF). The `score` field in results is the RRF score, not raw cosine similarity.
