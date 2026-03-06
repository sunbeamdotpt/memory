# MCP Server Documentation

This directory contains all documentation for the MCP Server project.

## Project Phases

- **[Phase 2 - REST API Implementation](PHASE_2_COMPLETE.md)** - API endpoints and testing
- **[Phase 2 TDD Summary](PHASE_2_TDD_SUMMARY.md)** - Test-driven development approach
- **[Phase 3 - Semantic Memory Integration](PHASE_3_COMPLETE.md)** - Semantic search and memory
- **[TDD Final Summary](TDD_FINAL_SUMMARY.md)** - Complete TDD methodology
- **[TDD Summary](TDD_SUMMARY.md)** - Test-driven development overview
- **[Progress Tracker](PROGRESS.md)** - Development progress and milestones

## Quick Start

1. **Build the server**: `cargo build`
2. **Run the server**: `cargo run`
3. **Run tests**: `cargo nextest run`
4. **Test endpoints**: See API documentation below

## API Endpoints

- `GET /api/health` - Health check
- `POST /api/facts` - Add a fact
- `GET /api/facts/search` - Semantic search

## Architecture

- **Embedding Service**: Fastembed integration for vector embeddings
- **Semantic Store**: SQLite + HNSW for efficient search
- **Memory Service**: Core business logic
- **REST API**: Actix-Web endpoints

## Testing

All tests are located in the `tests/` directory:
- `tests/api_endpoints.rs` - API contract tests
- `tests/semantic_memory.rs` - Memory service tests
- `tests/embedding_tests.rs` - Embedding tests
- `tests/semantic_integration.rs` - Integration tests

Test data is stored in `tests/data/` directory.
