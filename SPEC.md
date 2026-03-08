# qgoots - Specification

> Local semantic memory engine powered by DuckDB. Inspired by [tobi/qmd](https://github.com/tobi/qmd).

## Overview

**qgoots** is a CLI tool that indexes markdown documents (notes, meeting transcripts, knowledge bases) and provides hybrid search combining BM25 full-text search and vector semantic search using DuckDB as the single storage backend. Everything runs locally — no cloud APIs required for storage and search.

The name "qgoots" stands for **Q**uery **G**oots — a local memory system powered by DuckDB.

---

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│                      CLI (clap)                         │
│  qgoots add | index | search | vsearch | query | get   │
└──────────────────────┬──────────────────────────────────┘
                       │
┌──────────────────────▼──────────────────────────────────┐
│                   Core Engine                           │
│  ┌─────────┐  ┌──────────┐  ┌────────────┐            │
│  │ Indexer  │  │ Searcher │  │ Embedder   │            │
│  │ (ingest) │  │ (hybrid) │  │ (vectors)  │            │
│  └────┬─────┘  └────┬─────┘  └─────┬──────┘            │
│       │              │              │                   │
│  ┌────▼──────────────▼──────────────▼──────┐            │
│  │              DuckDB                      │            │
│  │  ┌─────────┐  ┌──────┐  ┌───────────┐  │            │
│  │  │ FTS ext │  │ JSON │  │ VSS ext   │  │            │
│  │  │ (BM25)  │  │      │  │ (HNSW)    │  │            │
│  │  └─────────┘  └──────┘  └───────────┘  │            │
│  └─────────────────────────────────────────┘            │
└─────────────────────────────────────────────────────────┘
```

### Language & Dependencies

| Component | Choice | Rationale |
|-----------|--------|-----------|
| Language | **Rust** | Performance, single binary distribution, strong typing |
| Database | **DuckDB** (via `duckdb-rs`) | Embedded OLAP, extensions for FTS + VSS |
| Vector search | **DuckDB VSS extension** | HNSW indexes, cosine/L2/IP distance |
| Full-text search | **DuckDB FTS extension** | BM25 scoring built-in |
| Embeddings | External provider (configurable) | Ollama, OpenAI, or local GGUF via candle |
| CLI framework | **clap** | Standard Rust CLI library |
| Config | **TOML** | Rust-native config format |

---

## Data Model

### Database: `~/.local/share/qgoots/qgoots.duckdb`

#### Table: `collections`

```sql
CREATE TABLE collections (
    name       VARCHAR PRIMARY KEY,
    path       VARCHAR NOT NULL,
    pattern    VARCHAR NOT NULL DEFAULT '**/*.md',
    context    VARCHAR,              -- description for LLM context
    created_at TIMESTAMP DEFAULT current_timestamp,
    updated_at TIMESTAMP DEFAULT current_timestamp
);
```

#### Table: `documents`

```sql
CREATE TABLE documents (
    id           VARCHAR PRIMARY KEY,  -- SHA-256 hash of (collection + path)
    collection   VARCHAR NOT NULL REFERENCES collections(name),
    path         VARCHAR NOT NULL,     -- relative path within collection
    title        VARCHAR,
    content_hash VARCHAR NOT NULL,     -- SHA-256 of content body
    content      TEXT NOT NULL,
    byte_size    INTEGER NOT NULL,
    active       BOOLEAN DEFAULT true,
    indexed_at   TIMESTAMP DEFAULT current_timestamp,
    modified_at  TIMESTAMP,           -- filesystem mtime
    UNIQUE(collection, path)
);
```

#### Table: `chunks`

```sql
CREATE TABLE chunks (
    id          VARCHAR PRIMARY KEY,  -- "{doc_id}:{seq}"
    doc_id      VARCHAR NOT NULL REFERENCES documents(id),
    seq         INTEGER NOT NULL,     -- chunk sequence number
    content     TEXT NOT NULL,
    start_pos   INTEGER NOT NULL,     -- byte offset in original document
    end_pos     INTEGER NOT NULL,
    token_count INTEGER,
    UNIQUE(doc_id, seq)
);
```

#### Table: `embeddings`

```sql
CREATE TABLE embeddings (
    chunk_id   VARCHAR PRIMARY KEY REFERENCES chunks(id),
    model      VARCHAR NOT NULL,     -- embedding model identifier
    vec        FLOAT[{DIM}],         -- dimension depends on model
    created_at TIMESTAMP DEFAULT current_timestamp
);

-- HNSW index for vector similarity search
CREATE INDEX idx_embeddings_vec ON embeddings USING HNSW (vec)
WITH (metric = 'cosine');
```

> **Note**: `{DIM}` is determined at init time based on the configured embedding model (e.g., 384 for all-MiniLM-L6-v2, 768 for nomic-embed-text, 1536 for OpenAI text-embedding-3-small).

#### FTS Index

```sql
-- DuckDB FTS extension
PRAGMA create_fts_index(
    'documents',
    'id',
    'title', 'content',
    stemmer = 'porter',
    stopwords = 'english',
    overwrite = 1
);
```

### Contexts (hierarchical metadata)

```sql
CREATE TABLE contexts (
    path    VARCHAR PRIMARY KEY,  -- "global", "collection_name", or "collection_name/sub/path"
    context TEXT NOT NULL,
    created_at TIMESTAMP DEFAULT current_timestamp
);
```

Contexts are hierarchical — a document at `notes/work/q4-plan.md` inherits contexts from:
1. `global` (if set)
2. `notes` (collection context)
3. `notes/work` (path context)

Contexts are joined with `\n\n` and returned alongside search results to give LLMs domain information.

---

## Chunking Strategy

Documents are split into chunks for embedding. The chunking respects markdown structure:

| Break Point | Priority Score |
|-------------|---------------|
| `# H1` | 100 |
| `## H2` | 90 |
| `### H3` / code fence | 80 |
| `#### H4` | 70 |
| `---` (horizontal rule) | 60 |
| `##### H5` / `###### H6` | 50 |
| Paragraph break (`\n\n`) | 20 |
| List item | 5 |
| Newline | 1 |

**Parameters**:
- Target chunk size: **512 tokens** (configurable)
- Overlap: **15%** (~77 tokens)
- Never split inside code fences
- Fallback: token-based chunking if no structural breaks found

---

## Search Modes

### 1. `search` — BM25 Full-Text Only

Fast keyword-based search using DuckDB FTS extension.

```sql
SELECT id, title, path, collection,
    fts_main_documents.match_bm25(id, '{query}') AS score
FROM documents
WHERE score IS NOT NULL
ORDER BY score DESC
LIMIT {n};
```

### 2. `vsearch` — Vector Semantic Only

Cosine similarity search using HNSW index.

```sql
SELECT c.doc_id, d.title, d.path, d.collection,
    array_cosine_distance(e.vec, {query_vec}::FLOAT[{DIM}]) AS distance
FROM embeddings e
JOIN chunks c ON e.chunk_id = c.id
JOIN documents d ON c.doc_id = d.id
WHERE d.active = true
ORDER BY array_cosine_distance(e.vec, {query_vec}::FLOAT[{DIM}])
LIMIT {n};
```

Score = `1.0 - distance` (range 0.0 to 1.0).

### 3. `query` — Hybrid Search (BM25 + Vector + RRF)

Combines both search modes using **Reciprocal Rank Fusion (RRF)**:

```
RRF_score(doc) = Σ (weight_i / (k + rank_i))
```

Where:
- `k = 60` (standard RRF constant)
- BM25 results get weight **1.0**
- Vector results get weight **1.0**
- Top-rank bonus: +0.05 for rank 1, +0.02 for rank 2-3

Pipeline:
1. Run BM25 search → get top 50 candidates
2. Run vector search → get top 50 candidates
3. Merge via RRF scoring
4. Deduplicate by document (keep best score)
5. Return top N results

---

## Embedding Provider

Configurable via `~/.config/qgoots/config.toml`:

```toml
[embedding]
provider = "ollama"          # "ollama" | "openai" | "local"
model = "nomic-embed-text"   # model name
dimension = 768              # vector dimension
base_url = "http://localhost:11434"  # for ollama

# Alternative: OpenAI
# provider = "openai"
# model = "text-embedding-3-small"
# dimension = 1536
# api_key_env = "OPENAI_API_KEY"
```

**Embedding format** (nomic-style task prefix):
- Query: `search_query: {query}`
- Document: `search_document: {text}`

For models that don't use task prefixes, raw text is sent directly.

---

## CLI Interface

```
qgoots — Local semantic memory engine powered by DuckDB

USAGE:
    qgoots <COMMAND> [OPTIONS]

COMMANDS:
    init                    Initialize database and install extensions
    collection              Manage document collections
      add <path>            Register a directory as collection
        --name <name>       Collection name (default: directory name)
        --pattern <glob>    File pattern (default: **/*.md)
      list                  List all collections with stats
      remove <name>         Remove a collection
    index                   Index/re-index documents
      [collection]          Specific collection (default: all)
      --force               Force re-index even if unchanged
    embed                   Generate embeddings for indexed documents
      [collection]          Specific collection (default: all)
      --force               Force re-embed all chunks
    search <query>          BM25 full-text search
      -n <count>            Max results (default: 10)
      -c <collection>       Filter by collection
      --min-score <f>       Minimum score threshold
    vsearch <query>         Vector semantic search
      -n <count>            Max results (default: 10)
      -c <collection>       Filter by collection
    query <query>           Hybrid search (BM25 + vector + RRF)
      -n <count>            Max results (default: 10)
      -c <collection>       Filter by collection
      --min-score <f>       Minimum score threshold
    get <path>              Retrieve a document by path or ID
      --full                Show full content
      --chunks              Show individual chunks
    context                 Manage hierarchical contexts
      add [path] <text>     Add context for path (or global)
      list                  List all contexts
      remove <path>         Remove context
    stats                   Show database statistics
    cleanup                 Remove orphaned data, compact indexes
    serve                   Start MCP server (stdio)
      --http                Use HTTP transport
      --port <n>            HTTP port (default: 3945)

GLOBAL OPTIONS:
    --db <path>             Custom database path
    --config <path>         Custom config path
    --format <fmt>          Output format: text | json | csv | md
    --verbose               Verbose output
    --version               Show version
```

---

## Output Formats

All search commands support `--format`:

### text (default)
```
[0.87] notes/work/q4-plan.md
  Q4 Planning Document
  ...first 200 chars of best matching chunk...

[0.72] meetings/2024-12-standup.md
  Daily Standup Dec 2024
  ...first 200 chars of best matching chunk...
```

### json
```json
{
  "results": [
    {
      "score": 0.87,
      "collection": "notes",
      "path": "work/q4-plan.md",
      "title": "Q4 Planning Document",
      "snippet": "...",
      "context": "Work-related notes"
    }
  ],
  "query": "q4 planning goals",
  "mode": "hybrid",
  "total": 2,
  "elapsed_ms": 45
}
```

---

## MCP Server Interface

qgoots exposes a full MCP (Model Context Protocol) server for integration with AI assistants (Claude Code, Claude Desktop, Cursor, etc.). Built with the official Rust MCP SDK (`rmcp` crate).

### Transports

| Transport | Command | Use Case |
|-----------|---------|----------|
| **stdio** (default) | `qgoots serve` | Claude Code, IDE integrations |
| **Streamable HTTP** | `qgoots serve --http --port 3945` | Multi-client, remote access |

### Server Capabilities

```json
{
  "capabilities": {
    "tools": { "listChanged": true },
    "resources": { "subscribe": true, "listChanged": true },
    "prompts": { "listChanged": true },
    "logging": {}
  }
}
```

### Tools

Tools are the primary interface for AI agents to interact with qgoots.

#### `qgoots_search` — BM25 Full-Text Search

Fast keyword-based search.

```json
{
  "name": "qgoots_search",
  "description": "Search documents using BM25 full-text search. Best for exact keyword matches.",
  "inputSchema": {
    "type": "object",
    "required": ["query"],
    "properties": {
      "query":      { "type": "string",  "description": "Search query terms" },
      "collection": { "type": "string",  "description": "Filter by collection name" },
      "n":          { "type": "integer", "description": "Max results (default: 10)", "default": 10 },
      "min_score":  { "type": "number",  "description": "Minimum score threshold (0.0-1.0)" }
    }
  }
}
```

#### `qgoots_vsearch` — Vector Semantic Search

Embedding-based cosine similarity search via HNSW index.

```json
{
  "name": "qgoots_vsearch",
  "description": "Search documents using vector semantic similarity. Best for meaning-based queries.",
  "inputSchema": {
    "type": "object",
    "required": ["query"],
    "properties": {
      "query":      { "type": "string",  "description": "Natural language query" },
      "collection": { "type": "string",  "description": "Filter by collection name" },
      "n":          { "type": "integer", "description": "Max results (default: 10)", "default": 10 }
    }
  }
}
```

#### `qgoots_query` — Hybrid Search (Recommended)

Combines BM25 + vector search with Reciprocal Rank Fusion. Best overall quality.

```json
{
  "name": "qgoots_query",
  "description": "Hybrid search combining keyword and semantic matching. Recommended for most queries.",
  "inputSchema": {
    "type": "object",
    "required": ["query"],
    "properties": {
      "query":      { "type": "string",  "description": "Search query (natural language or keywords)" },
      "collection": { "type": "string",  "description": "Filter by collection name" },
      "n":          { "type": "integer", "description": "Max results (default: 10)", "default": 10 },
      "min_score":  { "type": "number",  "description": "Minimum score threshold (0.0-1.0)" }
    }
  }
}
```

#### `qgoots_get` — Retrieve Document

Fetch a single document by path, virtual URI, or short hash ID.

```json
{
  "name": "qgoots_get",
  "description": "Retrieve a document by path, qgoots:// URI, or short hash ID (#abc123).",
  "inputSchema": {
    "type": "object",
    "required": ["path"],
    "properties": {
      "path":   { "type": "string",  "description": "Document path, URI (qgoots://collection/path), or hash (#abc123)" },
      "full":   { "type": "boolean", "description": "Return full content (default: true)", "default": true },
      "chunks": { "type": "boolean", "description": "Return individual chunks instead of full content" }
    }
  }
}
```

#### `qgoots_list` — List Documents

```json
{
  "name": "qgoots_list",
  "description": "List documents in the index, optionally filtered by collection or glob pattern.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "collection": { "type": "string", "description": "Filter by collection name" },
      "pattern":    { "type": "string", "description": "Glob pattern to filter paths (e.g. '*.md')" },
      "limit":      { "type": "integer", "description": "Max results (default: 100)", "default": 100 }
    }
  }
}
```

#### `qgoots_collections` — List Collections

```json
{
  "name": "qgoots_collections",
  "description": "List all registered collections with document counts and paths.",
  "inputSchema": { "type": "object", "properties": {} }
}
```

#### `qgoots_context` — Get Context

```json
{
  "name": "qgoots_context",
  "description": "Get the hierarchical context metadata for a given path. Returns inherited contexts.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "path": { "type": "string", "description": "Path to resolve context for (e.g. 'notes/work'). Omit for global context." }
    }
  }
}
```

#### `qgoots_stats` — Database Statistics

```json
{
  "name": "qgoots_stats",
  "description": "Show database statistics: document count, chunk count, embedding coverage, storage size.",
  "inputSchema": { "type": "object", "properties": {} }
}
```

#### `qgoots_ingest` — Index a Single Document

For AI agents to programmatically add documents to memory.

```json
{
  "name": "qgoots_ingest",
  "description": "Index a single document into qgoots memory. Creates chunks and embeddings automatically.",
  "inputSchema": {
    "type": "object",
    "required": ["collection", "path", "content"],
    "properties": {
      "collection": { "type": "string", "description": "Target collection name" },
      "path":       { "type": "string", "description": "Document path within collection" },
      "content":    { "type": "string", "description": "Markdown content to index" },
      "title":      { "type": "string", "description": "Document title (auto-detected from # heading if omitted)" }
    }
  }
}
```

### Resources

Resources expose documents as readable URIs for LLM context injection.

#### Static Resources

| URI | Name | Description |
|-----|------|-------------|
| `qgoots://stats` | Database Stats | Current index statistics |
| `qgoots://collections` | Collections | List of all collections |

#### Resource Templates (dynamic)

| URI Template | Name | Description |
|--------------|------|-------------|
| `qgoots://collections/{collection}` | Collection | Document listing for a collection |
| `qgoots://doc/{collection}/{path}` | Document | Full document content |
| `qgoots://context/{path}` | Context | Hierarchical context for a path |

**Example resource read:**

Request: `resources/read` with `uri: "qgoots://doc/notes/work/q4-plan.md"`

Response:
```json
{
  "contents": [
    {
      "uri": "qgoots://doc/notes/work/q4-plan.md",
      "mimeType": "text/markdown",
      "text": "# Q4 Planning Document\n\n..."
    }
  ]
}
```

### Prompts

Prompt templates for common workflows.

#### `search_and_summarize`

```json
{
  "name": "search_and_summarize",
  "description": "Search the knowledge base and produce a summary of relevant findings.",
  "arguments": [
    { "name": "query", "description": "What to search for", "required": true },
    { "name": "collection", "description": "Limit search to a collection", "required": false }
  ]
}
```

Returns a prompt message sequence:
1. **system**: Context from matched collections
2. **user**: "Search for: {query}" + top results with snippets
3. **assistant**: "I'll analyze the search results and provide a summary."

#### `remember`

```json
{
  "name": "remember",
  "description": "Store a piece of information in the knowledge base for later retrieval.",
  "arguments": [
    { "name": "content", "description": "The information to remember (markdown)", "required": true },
    { "name": "title", "description": "Short title for the memory", "required": true },
    { "name": "collection", "description": "Collection to store in (default: 'memories')", "required": false }
  ]
}
```

#### `recall`

```json
{
  "name": "recall",
  "description": "Search memory for previously stored information relevant to a topic.",
  "arguments": [
    { "name": "topic", "description": "What to recall", "required": true }
  ]
}
```

### Rust Implementation

Using `rmcp` v1.1 with derive macros:

```rust
use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::router::tool::ToolRouter,
    handler::server::router::prompt::PromptRouter,
    handler::server::wrapper::Parameters,
    model::*,
    schemars, tool, tool_handler, tool_router,
    prompt, prompt_handler, prompt_router,
    service::RequestContext,
    transport::stdio,
    ErrorData as McpError, RoleServer,
};

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SearchParams {
    /// Search query terms
    pub query: String,
    /// Filter by collection name
    pub collection: Option<String>,
    /// Max results (default: 10)
    pub n: Option<i32>,
    /// Minimum score threshold (0.0-1.0)
    pub min_score: Option<f64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct IngestParams {
    /// Target collection name
    pub collection: String,
    /// Document path within collection
    pub path: String,
    /// Markdown content to index
    pub content: String,
    /// Document title
    pub title: Option<String>,
}

#[derive(Clone)]
pub struct QgootsServer {
    db: Arc<DuckDbPool>,
    embedder: Arc<dyn Embedder>,
    tool_router: ToolRouter<Self>,
    prompt_router: PromptRouter<Self>,
}

#[tool_router]
#[prompt_router]
impl QgootsServer {
    #[tool(description = "Hybrid search combining keyword and semantic matching. Recommended for most queries.")]
    async fn qgoots_query(
        &self,
        Parameters(params): Parameters<SearchParams>,
    ) -> Result<CallToolResult, McpError> {
        let results = self.hybrid_search(&params).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![
            Content::text(serde_json::to_string_pretty(&results).unwrap())
        ]))
    }

    #[tool(description = "Search documents using BM25 full-text search. Best for exact keyword matches.")]
    async fn qgoots_search(
        &self,
        Parameters(params): Parameters<SearchParams>,
    ) -> Result<CallToolResult, McpError> {
        let results = self.bm25_search(&params).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![
            Content::text(serde_json::to_string_pretty(&results).unwrap())
        ]))
    }

    #[tool(description = "Search documents using vector semantic similarity.")]
    async fn qgoots_vsearch(
        &self,
        Parameters(params): Parameters<SearchParams>,
    ) -> Result<CallToolResult, McpError> {
        let results = self.vector_search(&params).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![
            Content::text(serde_json::to_string_pretty(&results).unwrap())
        ]))
    }

    #[tool(description = "Index a document into qgoots memory. Creates chunks and embeddings automatically.")]
    async fn qgoots_ingest(
        &self,
        Parameters(params): Parameters<IngestParams>,
    ) -> Result<CallToolResult, McpError> {
        let result = self.ingest_document(&params).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![
            Content::text(format!("Indexed: {} ({} chunks)", params.path, result.chunk_count))
        ]))
    }

    // ... other tools ...

    #[prompt(name = "search_and_summarize")]
    async fn search_and_summarize(
        &self,
        Parameters(args): Parameters<SearchAndSummarizeArgs>,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<GetPromptResult, McpError> {
        let results = self.hybrid_search(&SearchParams {
            query: args.query.clone(),
            collection: args.collection,
            n: Some(5),
            min_score: None,
        }).await.map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let context = results.iter()
            .map(|r| format!("## {} (score: {:.2})\n{}", r.title, r.score, r.snippet))
            .collect::<Vec<_>>()
            .join("\n\n");

        Ok(GetPromptResult::new(vec![
            PromptMessage::new_text(PromptMessageRole::User, format!(
                "Based on the following search results for \"{}\", provide a comprehensive summary:\n\n{}",
                args.query, context
            )),
        ]))
    }

    #[prompt(name = "recall")]
    async fn recall(
        &self,
        Parameters(args): Parameters<RecallArgs>,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<GetPromptResult, McpError> {
        let results = self.hybrid_search(&SearchParams {
            query: args.topic.clone(),
            collection: None,
            n: Some(10),
            min_score: Some(0.3),
        }).await.map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let memories = results.iter()
            .map(|r| format!("---\n**{}** ({})\n{}", r.title, r.path, r.snippet))
            .collect::<Vec<_>>()
            .join("\n\n");

        Ok(GetPromptResult::new(vec![
            PromptMessage::new_text(PromptMessageRole::User, format!(
                "Here are relevant memories about \"{}\":\n\n{}\n\nUse these to inform your response.",
                args.topic, memories
            )),
        ]))
    }
}

#[tool_handler]
#[prompt_handler]
impl ServerHandler for QgootsServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .enable_prompts()
                .build(),
        )
        .with_server_info(Implementation::new("qgoots", env!("CARGO_PKG_VERSION")))
        .with_instructions(
            "qgoots is a local semantic memory engine. Use qgoots_query for hybrid search, \
             qgoots_ingest to store new memories, qgoots_get to retrieve documents."
        )
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        let mut resources = vec![
            RawResource::new("qgoots://stats", "Database Stats").no_annotation(),
            RawResource::new("qgoots://collections", "Collections").no_annotation(),
        ];
        // Add dynamic collection resources
        for coll in self.list_collections().await? {
            resources.push(
                RawResource::new(
                    format!("qgoots://collections/{}", coll.name),
                    format!("Collection: {}", coll.name),
                ).no_annotation()
            );
        }
        Ok(ListResourcesResult { resources, next_cursor: None, meta: None })
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, McpError> {
        let uri = &request.uri;
        // Parse qgoots:// URIs and return appropriate content
        // ...
        todo!()
    }
}
```

### MCP Server Entry Point

```rust
// In main.rs, under the `serve` subcommand:
async fn serve(args: &ServeArgs) -> Result<()> {
    let server = QgootsServer::new(&args.db_path, &args.config_path).await?;

    if args.http {
        // Streamable HTTP transport
        let service = StreamableHttpService::new(move || server.clone());
        let app = axum::Router::new()
            .route("/mcp", service.into_axum_handler())
            .route("/health", axum::routing::get(|| async { "ok" }));
        let listener = tokio::net::TcpListener::bind(
            format!("127.0.0.1:{}", args.port)
        ).await?;
        eprintln!("qgoots MCP server listening on http://127.0.0.1:{}/mcp", args.port);
        axum::serve(listener, app).await?;
    } else {
        // stdio transport (default)
        eprintln!("qgoots MCP server running on stdio");
        let service = server.serve(stdio()).await?;
        service.waiting().await?;
    }
    Ok(())
}
```

### Client Configuration

#### Claude Code (`~/.claude/mcp_servers.json`)

```json
{
  "qgoots": {
    "command": "qgoots",
    "args": ["serve"],
    "type": "stdio"
  }
}
```

#### Claude Desktop (`claude_desktop_config.json`)

```json
{
  "mcpServers": {
    "qgoots": {
      "command": "qgoots",
      "args": ["serve"]
    }
  }
}
```

#### HTTP Mode (multi-client)

```json
{
  "qgoots": {
    "url": "http://localhost:3945/mcp",
    "type": "streamable-http"
  }
}
```

### MCP Data Flow

```
AI Assistant (Claude, etc.)
    │
    ├─ tools/call "qgoots_query" { query: "auth flow" }
    │   └─► QgootsServer::qgoots_query()
    │       ├─ BM25 search (DuckDB FTS)
    │       ├─ Vector search (DuckDB VSS/HNSW)
    │       ├─ RRF merge
    │       └─► CallToolResult { content: [{ text: "..." }] }
    │
    ├─ tools/call "qgoots_ingest" { collection: "memories", path: "...", content: "..." }
    │   └─► QgootsServer::qgoots_ingest()
    │       ├─ Chunk document
    │       ├─ Generate embeddings (Ollama/OpenAI)
    │       ├─ Insert into DuckDB
    │       └─► CallToolResult { content: [{ text: "Indexed: ..." }] }
    │
    ├─ resources/read { uri: "qgoots://doc/notes/plan.md" }
    │   └─► QgootsServer::read_resource()
    │       └─► ReadResourceResult { contents: [{ text: "# Plan\n..." }] }
    │
    └─ prompts/get "recall" { topic: "auth" }
        └─► QgootsServer::recall()
            ├─ Hybrid search for "auth"
            └─► GetPromptResult { messages: [...] }
```

---

## File Layout

```
~/.config/qgoots/
  config.toml              # Embedding provider, preferences
~/.local/share/qgoots/
  qgoots.duckdb            # Main database (DuckDB)
```

Respects `XDG_CONFIG_HOME` and `XDG_DATA_HOME` environment variables.

---

## Initialization Flow

```
qgoots init
```

1. Create config directory and default `config.toml`
2. Create data directory
3. Open DuckDB database
4. Install and load extensions: `INSTALL vss; LOAD vss; INSTALL fts; LOAD fts;`
5. Create tables (`collections`, `documents`, `chunks`, `embeddings`, `contexts`)
6. Verify embedding provider connectivity (e.g., ping Ollama)
7. Print summary

---

## Indexing Flow

```
qgoots collection add ~/notes --name notes
qgoots index
qgoots embed
```

### `index` (text ingestion):

1. For each collection, glob matching files
2. For each file:
   - Compute SHA-256 of content
   - Skip if `content_hash` matches existing record
   - Parse title from first `# heading` or filename
   - Upsert into `documents` table
   - Chunk document using markdown-aware splitter
   - Upsert chunks into `chunks` table
3. Mark deleted files as `active = false`
4. Rebuild FTS index: `PRAGMA create_fts_index('documents', ..., overwrite = 1)`

### `embed` (vector generation):

1. Query chunks without embeddings (or all if `--force`)
2. Batch chunks (e.g., 32 at a time)
3. Call embedding provider
4. Insert into `embeddings` table
5. Rebuild HNSW index if needed

---

## BM25 Score Normalization

Raw FTS5 BM25 scores are normalized to 0.0-1.0 range:

```
normalized = |raw_score| / (1 + |raw_score|)
```

This is monotonic and query-independent:
- Strong match (-10) → 0.91
- Medium match (-2) → 0.67
- Weak match (-0.5) → 0.33

---

## Configuration Reference

`~/.config/qgoots/config.toml`:

```toml
[general]
default_format = "text"       # text | json | csv | md
default_results = 10          # default -n value

[embedding]
provider = "ollama"
model = "nomic-embed-text"
dimension = 768
base_url = "http://localhost:11434"
batch_size = 32
# For task-prefix models:
query_prefix = "search_query: "
document_prefix = "search_document: "

[chunking]
target_tokens = 512
overlap_pct = 15

[search]
rrf_k = 60                   # RRF constant
bm25_weight = 1.0
vector_weight = 1.0
default_min_score = 0.0

[mcp]
port = 3945
```

---

## Rust Crate Dependencies (expected)

```toml
[dependencies]
duckdb = { version = "1", features = ["bundled"] }
clap = { version = "4", features = ["derive"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
schemars = "1.0"                # JSON Schema generation (required by rmcp)
toml = "0.8"
sha2 = "0.10"
glob = "0.3"
tokio = { version = "1", features = ["full"] }
reqwest = { version = "0.12", features = ["json"] }  # for Ollama/OpenAI API
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

# MCP Server
rmcp = { version = "1.1", features = [
    "server",
    "macros",
    "schemars",
    "transport-io",                       # stdio transport
    "transport-streamable-http-server",   # HTTP transport
] }
axum = "0.8"                   # HTTP server (for MCP streamable HTTP transport)
```

---

## Design Decisions & Rationale

### Why DuckDB instead of SQLite + sqlite-vec?

| Aspect | SQLite + sqlite-vec | DuckDB |
|--------|-------------------|--------|
| Vector search | sqlite-vec virtual table (limited) | HNSW index (standard ANN algorithm) |
| Full-text search | FTS5 (excellent) | FTS extension (BM25, adequate) |
| Columnar analytics | No | Yes — fast aggregations, stats |
| JSON support | json1 extension | Native JSON type + rich functions |
| Single binary | Needs extension loading | Extensions bundled or auto-installed |
| Concurrent reads | WAL mode | Native MVCC |

DuckDB provides both VSS and FTS in a single embedded database, reducing architectural complexity.

### Why not run LLMs locally for embedding?

qgoots delegates embedding to external providers (Ollama, OpenAI) rather than bundling GGUF models. This keeps the binary small, avoids GPU/GGUF dependency complexity in Rust, and lets users choose their preferred model. Ollama is the recommended default since it's also fully local.

### Content-Addressable Storage

Documents are keyed by SHA-256 hash of `(collection, path)` for stable IDs. Content changes are detected by comparing `content_hash` (SHA-256 of body). This enables efficient incremental indexing — only changed files are re-processed.

---

## Future Considerations

- **Query expansion**: Add LLM-based query expansion for hybrid search (like qmd's typed variants)
- **Re-ranking**: Optional LLM re-ranking of top candidates
- **Watch mode**: Filesystem watcher for auto-indexing on file changes
- **Multi-index**: Support named indexes for separate knowledge bases
- **Import/export**: Backup and restore collections
- **Web UI**: Optional local web interface for search

---

## References

- [tobi/qmd](https://github.com/tobi/qmd) — Inspiration for architecture and search pipeline
- [DuckDB VSS Extension](https://duckdb.org/docs/stable/core_extensions/vss) — HNSW vector search
- [DuckDB FTS Extension](https://duckdb.org/docs/stable/core_extensions/full_text_search) — BM25 full-text search
- [DuckDB JSON](https://duckdb.org/docs/stable/data/json/overview) — Native JSON support
- [MCP Specification](https://modelcontextprotocol.io/specification/2025-03-26) — Model Context Protocol v2025-03-26
- [rmcp crate](https://crates.io/crates/rmcp) — Official Rust MCP SDK v1.1
- [modelcontextprotocol/rust-sdk](https://github.com/modelcontextprotocol/rust-sdk) — Rust SDK source
- [Reciprocal Rank Fusion](https://plg.uwaterloo.ca/~gvcormac/cormacksigir09-rrf.pdf) — RRF paper
