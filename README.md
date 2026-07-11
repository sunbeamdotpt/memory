# sunbeam-memory

A personal semantic memory server for AI assistants. Store facts, code snippets, notes, and documents with vector embeddings — then let your AI search them by meaning, not just keywords.

Also watches directories and auto-ingests files (code, PDFs, text) so your memory stays in sync with your projects.

Works as a local stdio MCP server (zero config, Claude Desktop) or as a remote HTTP server so you can access your memory from any machine.

Built on the official [`rmcp`](https://github.com/modelcontextprotocol/rust-sdk) Rust SDK for native MCP protocol support. Also exposes a ConnectRPC (protobuf over HTTP/2) API for programmatic access from scripts and services.

---

## Install

**Requirements:** Rust 1.75+ — the first build downloads the BGE embedding model (~130 MB).

```bash
git clone https://github.com/your-org/sunbeam-memory
cd sunbeam-memory/mcp-server
cargo build --release
# binary at target/release/sunbeam-memory
```

Or run directly without a permanent binary:

```bash
cargo run --release -- http --port 3456
```

---

## Local use with Claude Desktop

The simplest setup: Claude Desktop talks to the server over stdio. No network, no auth.

Add to `~/Library/Application Support/Claude/claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "memory": {
      "command": "/path/to/sunbeam-memory"
    }
  }
}
```

The server stores data in the platform data directory by default:
- **macOS:** `~/Library/Application Support/sunbeam/memory`
- **Linux:** `~/.local/share/sunbeam/memory`
- **Windows:** `%APPDATA%/sunbeam/memory`

Set `MCP_MEMORY_BASE_DIR` to change it:

```json
{
  "mcpServers": {
    "memory": {
      "command": "/path/to/sunbeam-memory",
      "env": {
        "MCP_MEMORY_BASE_DIR": "/Users/you/.local/share/sunbeam-memory"
      }
    }
  }
}
```

---

## Remote use (HTTP mode)

Run the server on a VPS or home server so you can access your memory from any machine or AI client.

**1. Generate a token:**

```bash
openssl rand -hex 32
# e.g. a3f8c2e1b4d7...
```

**2. Start the server with the token:**

```bash
MCP_AUTH_TOKEN=a3f8c2e1b4d7... cargo run --release -- http --port 3456
```

With `MCP_AUTH_TOKEN` set, the server binds to `0.0.0.0` and requires `Authorization: Bearer <token>` on every request.

**3. Configure Claude Desktop (or any MCP client) to use the remote server:**

```json
{
  "mcpServers": {
    "memory": {
      "type": "http",
      "url": "http://your-server:3456/mcp",
      "headers": {
        "Authorization": "Bearer a3f8c2e1b4d7..."
      }
    }
  }
}
```

**Tip:** Put a reverse proxy (nginx, Caddy) in front with TLS so your token travels over HTTPS.

---

## Run as a systemd user daemon (Kimi Code / local HTTP)

You can keep the HTTP server running in the background as a user-scoped systemd service. This is the easiest way to connect Kimi Code (or any local MCP client) over HTTP without managing a terminal window.

**1. Create the service file**

`~/.config/systemd/user/sunbeam-memory.service`:

```ini
[Unit]
Description=Sunbeam Memory MCP Server
After=network.target

[Service]
Type=simple
ExecStart=/usr/local/bin/sunbeam-memory http --port 3456
Restart=on-failure
Environment="MCP_MEMORY_BASE_DIR=%h/.local/share/sunbeam/memory"

[Install]
WantedBy=default.target
```

Adjust `ExecStart` to the path of your `sunbeam-memory` binary (for example, `ExecStart=%h/development/sunbeam/memory/target/release/sunbeam-memory http --port 3456` if you are running from a local build).

**2. Start and enable the service**

```bash
systemctl --user daemon-reload
systemctl --user enable sunbeam-memory.service
systemctl --user start sunbeam-memory.service
systemctl --user status sunbeam-memory.service
```

**3. Configure Kimi Code**

Add the server to `~/.kimi-code/mcp.json`:

```json
{
  "mcpServers": {
    "sunbeam-memory": {
      "url": "http://127.0.0.1:3456/mcp"
    }
  }
}
```

Because no `MCP_AUTH_TOKEN` is set, the server binds to `127.0.0.1` only and does not require authentication, which is safe for a local daemon.

### OIDC / OAuth2 authentication

If you already have an OIDC provider (Keycloak, Auth0, Dex, Kratos+Hydra, etc.), you can use it instead of a raw token. The server fetches the JWKS at startup and validates RS256/ES256 JWTs on every request.

```bash
MCP_OIDC_ISSUER=https://auth.example.com \
MCP_OIDC_AUDIENCE=sunbeam-memory \       # optional — leave out to skip aud check
  cargo run --release -- http --port 3456
```

Your MCP client then gets a token from the provider and passes it as a Bearer token:

```json
{
  "mcpServers": {
    "memory": {
      "type": "http",
      "url": "http://your-server:3456/mcp",
      "headers": {
        "Authorization": "Bearer <access_token>"
      }
    }
  }
}
```

OIDC takes priority over `MCP_AUTH_TOKEN` if both are set.

### Environment variables

| Variable | Default | Description |
|---|---|---|
| `MCP_MEMORY_BASE_DIR` | platform data dir | Where the SQLite database and model cache are stored |
| `MCP_AUTH_TOKEN` | _(unset)_ | Simple bearer token for remote hosting. Unset = localhost-only |
| `MCP_OIDC_ISSUER` | _(unset)_ | OIDC issuer URL. When set, validates JWT bearer tokens via JWKS |
| `MCP_OIDC_AUDIENCE` | _(unset)_ | Expected `aud` claim. Leave unset to skip audience validation |
| `MCP_STDIO_KEEPALIVE_SECONDS` | `30` | Interval between MCP `ping` requests over stdio; `0` disables |
| `MCP_SSE_KEEPALIVE_SECONDS` | `15` | Interval between SSE keep-alive comments in HTTP mode; `0` disables |
| `MCP_SESSION_KEEPALIVE_SECONDS` | `300` | Idle timeout before an inactive HTTP session is closed; `0` disables |

---

## Tools

### Memory tools

#### `store_fact`
Embed and store a piece of text. Returns the fact ID.

```
content    (required) Text to store
namespace  (optional) Logical group — e.g. "code", "notes", "docs". Default: "default"
source     (optional) smem URN identifying where this came from (see below)
```

#### `search_facts`
Semantic search — finds content by meaning, not exact words.

```
query      (required) What you're looking for
limit      (optional) Max results. Default: 10
namespace  (optional) Restrict search to one namespace
```

#### `update_fact`
Update an existing fact in place. Keeps the same ID, re-embeds the new content.

```
id         (required) Fact ID from store_fact or search_facts
content    (required) New text content
source     (optional) New smem URN
```

#### `delete_fact`
Delete a fact by ID.

```
id         (required) Fact ID
```

#### `list_facts`
List facts in a namespace, newest first. Supports date filtering.

```
namespace  (optional) Namespace to list. Default: "default"
limit      (optional) Max results. Default: 50
from       (optional) Only show facts stored on or after this time (RFC 3339 or Unix timestamp)
to         (optional) Only show facts stored on or before this time
```

### Indexer tools (file watching & ingestion)

Point the server at directories, git repos, or individual files and it will automatically ingest them into searchable memory. PDFs are extracted page-by-page. Binary files are skipped. Git state is tracked so edits on the same branch update facts in place, while new branches create new facts.

#### `add_watch_target`
Start watching a path. Supports globs (`*` `?` `[...]`). Respects `.gitignore` when scanning git repositories.

```
path       (required) Directory, file, or glob pattern to watch
namespace  (optional) Namespace for ingested facts. Default: "default"
type       (optional) "file" | "directory" | "git_repo". Default: inferred from path
```

Examples:
- `add_watch_target` with `path: "/home/me/project/src"`
- `add_watch_target` with `path: "/home/me/docs/**/*.pdf"`, `namespace: "docs"`
- `add_watch_target` with `path: "/home/me/notes/*.{md,txt}"`, `namespace: "notes"`
- `add_watch_target` with `path: "/home/me/project"`, `type: "git_repo"`, `namespace: "project"`

#### `remove_watch_target`
Stop watching a target by ID.

```
target_id  (required) ID returned by add_watch_target
```

#### `list_watch_targets`
List all active watch targets. No inputs.

#### `sync_watch_target`
Force a full re-scan of a target right now.

```
target_id  (required) ID of the target to sync
```

#### `get_index_progress`
Check ingestion progress for a target: files pending, completed, failed, and current file.

```
target_id  (required) ID of the target
```

#### `restore_stale_fact`
Restore a stale (soft-deleted) fact so it appears in search again.

```
id  (required) Fact ID to restore
```

### URN tools

#### `build_source_urn`
Build a valid smem URN from components. Use this before passing `source` to `store_fact`.

```
content_type  (required) code | doc | web | data | note | conf
origin        (required) git | fs | https | http | db | api | manual
locator       (required) Origin-specific path (see describe_urn_schema)
fragment      (optional) Line reference: L42 or L10-L30
```

#### `parse_source_urn`
Parse and validate a smem URN. Returns structured components or an error.

```
urn  (required) The URN to parse, e.g. urn:smem:code:fs:/path/to/file.rs#L10
```

#### `describe_urn_schema`
Returns the full smem URN taxonomy: content types, origins, locator shapes, and examples. No inputs.

### Observability tools

#### `get_recent_errors`
List recent ingestion or system errors so you can be informed of failures.

```
component  (optional) Filter to a specific component (e.g. "indexer", "extractor")
limit      (optional) Max errors to return. Default: 10
```

#### `resolve_error`
Mark a logged error as resolved so it no longer appears in `get_recent_errors`.

```
error_id   (required) ID of the error to resolve
```

---

## Source URNs

Every fact can carry a `source` URN that records where it came from:

```
urn:smem:<type>:<origin>:<locator>[#<fragment>]
```

**Types:** `code` `doc` `web` `data` `note` `conf`

**Origins and locator shapes:**

| Origin | Locator | Example |
|--------|---------|---------|
| `fs` | `[hostname:]<absolute-path>` | `urn:smem:code:fs:/home/me/project/main.rs#L10-L30` |
| `git` | `<host>:<org>:<repo>:<branch>:<path>` | `urn:smem:code:git:github.com:org:repo:main:src/lib.rs` |
| `https` | `<host>/<path>` | `urn:smem:doc:https:docs.example.com/guide` |
| `db` | `<driver>/<host>/<db>/<table>/<pk>` | `urn:smem:data:db:postgres/localhost/app/users/42` |
| `api` | `<host>/<path>` | `urn:smem:data:api:api.example.com/v1/items/99` |
| `manual` | `<label>` | `urn:smem:note:manual:meeting-2026-03-04` |

Use `build_source_urn` to construct one without memorising the format. Use `describe_urn_schema` for the full spec.

---

## Data

Facts are stored in a SQLite database (`semantic.db`) in `MCP_MEMORY_BASE_DIR`. The embedding model is cached by fastembed on first run.

To back up your memory: copy the `semantic.db` file. It's self-contained.

---

## Architecture

```
┌─────────────────┐     ┌─────────────────┐
│  MCP client     │     │  gRPC/HTTP      │
│  (stdio / HTTP) │     │  client         │
└────────┬────────┘     └────────┬────────┘
         │                       │
         ▼                       ▼
  mcp/server.rs           connect/service.rs
  (rmcp tools)            (ConnectRPC proto)
         │                       │
         └───────────┬───────────┘
                     ▼
              core/service.rs    ← validation, orchestration
                     │
              memory/service.rs  ← embed, search, CRUD
                     │
         ┌───────────┴───────────┐
         ▼                       ▼
  semantic/store.rs        indexer/
  (HNSW + SQLite)          (watcher, scanner, PDF)
```

**MCP Protocol:** [MCP 2025-06-18](https://spec.modelcontextprotocol.io/specification/2025-06-18/) via [rmcp](https://github.com/modelcontextprotocol/rust-sdk) (stdio + Streamable HTTP).  
**Programmatic API:** ConnectRPC (`sunbeam.memory.v1.MemoryService`) via protobuf + HTTP/2.

**Embeddings:** BGE-Base-English-v1.5 via [fastembed](https://github.com/Anush008/fastembed-rs), 768 dimensions, ~130 MB model download on first run.

**Vector search:** [usearch](https://github.com/unum-cloud/usearch) HNSW index with cosine similarity, persisted as a blob inside SQLite.

**Search strategy:** Fused BM25 + vector search via Reciprocal Rank Fusion (RRF).
