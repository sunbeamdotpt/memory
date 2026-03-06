# sunbeam-memory

A personal semantic memory server for AI assistants. Store facts, code snippets, notes, and documents with vector embeddings — then let your AI search them by meaning, not just keywords.

Works as a local stdio MCP server (zero config, Claude Desktop) or as a remote HTTP server so you can access your memory from any machine.

---

## Install

**Requirements:** Rust 1.75+ — the first build downloads the BGE embedding model (~130 MB).

```bash
git clone https://github.com/your-org/sunbeam-memory
cd sunbeam-memory/mcp-server
cargo build --release
# binary at target/release/mcp-server
```

Or run directly without a permanent binary:

```bash
cargo run -- --http 3456
```

---

## Local use with Claude Desktop

The simplest setup: Claude Desktop talks to the server over stdio. No network, no auth.

Add to `~/Library/Application Support/Claude/claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "memory": {
      "command": "/path/to/mcp-server"
    }
  }
}
```

The server stores data in `./data/memory` by default. Set `MCP_MEMORY_BASE_DIR` to change it:

```json
{
  "mcpServers": {
    "memory": {
      "command": "/path/to/mcp-server",
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
MCP_AUTH_TOKEN=a3f8c2e1b4d7... cargo run --release -- --http 3456
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

### OIDC / OAuth2 authentication

If you already have an OIDC provider (Keycloak, Auth0, Dex, Kratos+Hydra, etc.), you can use it instead of a raw token. The server fetches the JWKS at startup and validates RS256/ES256 JWTs on every request.

```bash
MCP_OIDC_ISSUER=https://auth.example.com \
MCP_OIDC_AUDIENCE=sunbeam-memory \       # optional — leave out to skip aud check
  cargo run --release -- --http 3456
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
| `MCP_MEMORY_BASE_DIR` | `./data/memory` | Where the SQLite database and model cache are stored |
| `MCP_AUTH_TOKEN` | _(unset)_ | Simple bearer token for remote hosting. Unset = localhost-only |
| `MCP_OIDC_ISSUER` | _(unset)_ | OIDC issuer URL. When set, validates JWT bearer tokens via JWKS |
| `MCP_OIDC_AUDIENCE` | _(unset)_ | Expected `aud` claim. Leave unset to skip audience validation |

---

## Tools

### `store_fact`
Embed and store a piece of text. Returns the fact ID.

```
content    (required) Text to store
namespace  (optional) Logical group — e.g. "code", "notes", "docs". Default: "default"
source     (optional) smem URN identifying where this came from (see below)
```

### `search_facts`
Semantic search — finds content by meaning, not exact words.

```
query      (required) What you're looking for
limit      (optional) Max results. Default: 10
namespace  (optional) Restrict search to one namespace
```

### `update_fact`
Update an existing fact in place. Keeps the same ID, re-embeds the new content.

```
id         (required) Fact ID from store_fact or search_facts
content    (required) New text content
source     (optional) New smem URN
```

### `delete_fact`
Delete a fact by ID.

```
id         (required) Fact ID
```

### `list_facts`
List facts in a namespace, newest first. Supports date filtering.

```
namespace  (optional) Namespace to list. Default: "default"
limit      (optional) Max results. Default: 50
from       (optional) Only show facts stored on or after this time (RFC 3339 or Unix timestamp)
to         (optional) Only show facts stored on or before this time
```

### `build_source_urn`
Build a valid smem URN from components. Use this before passing `source` to `store_fact`.

```
content_type  (required) code | doc | web | data | note | conf
origin        (required) git | fs | https | http | db | api | manual
locator       (required) Origin-specific path (see describe_urn_schema)
fragment      (optional) Line reference: L42 or L10-L30
```

### `parse_source_urn`
Parse and validate a smem URN. Returns structured components or an error.

```
urn  (required) The URN to parse, e.g. urn:smem:code:fs:/path/to/file.rs#L10
```

### `describe_urn_schema`
Returns the full smem URN taxonomy: content types, origins, locator shapes, and examples. No inputs.

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
| `git` | `<host>/<org>/<repo>/<ref>/<path>` | `urn:smem:code:git:github.com/org/repo/main/src/lib.rs` |
| `https` | `<host>/<path>` | `urn:smem:doc:https:docs.example.com/guide` |
| `db` | `<driver>/<host>/<db>/<table>/<pk>` | `urn:smem:data:db:postgres/localhost/app/users/42` |
| `api` | `<host>/<path>` | `urn:smem:data:api:api.example.com/v1/items/99` |
| `manual` | `<label>` | `urn:smem:note:manual:meeting-2026-03-04` |

Use `build_source_urn` to construct one without memorising the format. Use `describe_urn_schema` for the full spec.

---

## Data

Facts are stored in a SQLite database in `MCP_MEMORY_BASE_DIR` (default `./data/memory/semantic.db`). The embedding model is cached by fastembed on first run.

To back up your memory: copy the `semantic.db` file. It's self-contained.

---

## Architecture

```
Claude / MCP client
      │
      │  stdio (local)  or  HTTP POST /mcp  (remote)
      ▼
 mcp/server.rs        ← JSON-RPC dispatch, tool handlers
      │
 memory/service.rs    ← embed content, business logic
      │
 semantic/store.rs    ← cosine similarity index (in-memory)
 semantic/db.rs       ← SQLite persistence (facts + embeddings)
```

Embeddings: BGE-Base-English-v1.5 via [fastembed](https://github.com/Anush008/fastembed-rs), 768 dimensions, ~130 MB model download on first run.
