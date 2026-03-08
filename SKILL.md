---
description: qgoots Semantic Memory Agent Skill
---

# qgoots Skill

This repository implements **qgoots**, a local semantic memory engine powered by DuckDB. 
It supports both Full-Text Search (BM25) and Vector Similarity Search (VSS) locally without the need for cloud APIs.

As an agent operating inside this workspace, you can utilize the `qgoots-cli` to perform knowledge retrieval and management tasks.

## Commands

### Initialization
Before doing anything, ensure the database is initialized:
```bash
cargo run -p qgoots-cli -- init
```

### Managing Collections
A collection maps a physical directory on disk to a namespace in the database.
```bash
# Add a new collection for markdown files
cargo run -p qgoots-cli -- collection add ./docs --name my-docs --pattern "**/*.md"

# List tracked collections
cargo run -p qgoots-cli -- collection list
```

### Indexing
After adding a collection (or modifying files), re-run the indexer:
```bash
cargo run -p qgoots-cli -- index
```

### Searching
You can perform searches to retrieve contextual information. By default, it returns the top 10 best matching snippets.
```bash
# BM25 Full Text Search
cargo run -p qgoots-cli -- search "DuckDB configuration"

# Vector Search (if embeddings are configured and generated via Ollama)
cargo run -p qgoots-cli -- vsearch "How does chunking work?"

# Hybrid Search (BM25 + Semantic)
cargo run -p qgoots-cli -- query "How does chunking work in DuckDB?"
```

## MCP Capabilities
`qgoots` acts natively as an MCP server. If configured as an MCP tool in your runtime (via `qgoots-cli serve`), you can directly invoke the `qgoots_*` server tools available in your context.
