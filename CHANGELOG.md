# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.0] — 2026-06-04

### Added

- **`CoreService`** — protocol-agnostic orchestration layer that sits between transport adapters and persistence. All validation and business logic lives here; MCP and ConnectRPC adapters are thin wrappers.
- **ConnectRPC adapter** (`src/connect/`) — full programmatic API over protobuf + HTTP/2, generated from `proto/sunbeam/memory/v1/memory.proto` via `connectrpc-build`. Implements 16 RPCs covering facts, indexer, URNs, observability, and health.
- **`ignore`-based directory scanning** — the indexer now respects `.gitignore`, global gitignore, and `.git/info/exclude` when scanning targets. Previously it indexed build artifacts and git internals.
- **`.git/` exclusion** — scanner explicitly skips git internal directories (object database, hooks, logs, etc.) to avoid binary ingestion failures.
- Regression test for indexer progress tracking (`test_sync_progress_processing_counter_resets`).

### Changed

- **Crate rename:** Library root and all imports changed from `mcp_server` to `sunbeam_memory`. This is a breaking change for any downstream crates.
- **Architecture refactor:** `SunbeamServer` now delegates to `CoreService` instead of owning business logic directly. `MemoryConnectService` implements the generated `MemoryService` trait by delegating to the same core.
- **Module normalization:** `src/semantic.rs` moved to `src/semantic/mod.rs` for consistent module layout.
- **Dependencies:** added `connectrpc`, `buffa`, `buffa-types`, `axum`, `tower`, `sunbeam-g2v`, `ignore`; removed unused `walkdir` (superseded by `ignore`).
- **Edition:** bumped from `2021` to `2024`.

### Removed

- `src/api/handlers.rs`, `src/api/mcp_http.rs`, `src/api/oidc.rs` — superseded by rmcp's Streamable HTTP and the new ConnectRPC adapter.
- `src/semantic/search.rs` — orphaned module with no callers after the search logic moved into `MemoryService`.
- `tests/oidc_tests.rs` — removed alongside the old OIDC middleware; JWT validation now lives in `src/api/oidc.rs` but is only used by the REST layer.
- `git::build_source_urn` — replaced by `SourceUrn::build_git_urn` on the `Urn` type.
- Old backup test files (`tests/semantic_db_errors_backup.rs`, `tests/api_endpoints_backup.rs`).

## [0.2.0] — 2026-06-04

### Added

- Native MCP protocol support via the official [`rmcp`](https://github.com/modelcontextprotocol/rust-sdk) Rust SDK.
- `rmcp-actix-web` for Streamable HTTP transport with session management.
- `clap` derive-based CLI: `sunbeam-memory http --port 3456` replaces `--http 3456`.
- `anyhow` for ergonomic error handling in the binary entry point.
- All tool parameter structs are now public and exported from `mcp_server::mcp`.

### Changed

- **MCP layer:** Replaced hand-rolled JSON-RPC dispatch with rmcp's `#[tool]`, `#[tool_router]`, and `#[tool_handler]` macros. Tool methods are first-class async methods on `SunbeamServer`.
- **stdio transport:** Now uses `rmcp::transport::stdio()` via `ServiceExt::serve()` instead of manual line-by-line JSON-RPC parsing.
- **HTTP transport:** Replaced custom POST/GET/DELETE handlers with `StreamableHttpService` + `LocalSessionManager`. Auth moved to an actix-web middleware wrapping the `/mcp` scope.
- **Tests:** `tests/mcp_tools.rs` and `tests/mcp_onboarding.rs` now call tool methods directly with typed `Parameters<T>` instead of routing through JSON dispatch.
- **README:** Updated CLI examples, architecture diagram, and tool signatures for the rmcp era.
- **AGENTS.md:** Refreshed technology stack, build commands, and module descriptions.

### Removed

- `src/mcp/protocol.rs` — superseded by rmcp's native wire types.
- `SunbeamServer::invoke_tool` and `SunbeamServer::list_all_tools` — routing is now entirely handled by the generated `ToolRouter`.
- `SessionStore` — replaced by rmcp's `LocalSessionManager`.
- `process_mcp_line` and associated stdin/stdout loop in `main.rs`.
- `MCP_SESSION_TTL_HOURS` environment variable — session lifetime is now managed by rmcp.
- Outdated example files: `examples/debug_stdio.rs`, `examples/usearch_empty.rs`, `examples/usearch_test.rs`.

## [0.1.0] — 2025-03-15

### Added

- Initial release: semantic memory server with SQLite persistence, FTS5 full-text search, and HNSW vector indexing via usearch.
- MCP stdio and HTTP transports with manual JSON-RPC framing.
- File watcher with automatic ingestion of code, text, and PDF files.
- Git repository scanning with branch/commit tracking.
- Source URN taxonomy for provenance tracking.
- Bearer token and OIDC JWT authentication for HTTP mode.
