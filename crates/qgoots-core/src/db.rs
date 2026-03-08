use duckdb::{Connection, Result};

pub fn init_db(path: &str, vector_dim: usize) -> Result<Connection> {
    let conn = Connection::open(path)?;

    // Install and load extensions needed for vector and full-text search
    conn.execute_batch(
        r#"
        INSTALL vss;
        LOAD vss;
        INSTALL fts;
        LOAD fts;
        SET hnsw_enable_experimental_persistence = true;
        "#,
    )?;

    // Create collections table
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS collections (
            name       VARCHAR PRIMARY KEY,
            path       VARCHAR NOT NULL,
            pattern    VARCHAR NOT NULL DEFAULT '**/*.md',
            context    VARCHAR,
            created_at TIMESTAMP DEFAULT current_timestamp,
            updated_at TIMESTAMP DEFAULT current_timestamp
        );
        "#,
    )?;

    // Create documents table
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS documents (
            id           VARCHAR PRIMARY KEY,
            collection   VARCHAR NOT NULL REFERENCES collections(name),
            path         VARCHAR NOT NULL,
            title        VARCHAR,
            content_hash VARCHAR NOT NULL,
            content      TEXT NOT NULL,
            byte_size    INTEGER NOT NULL,
            active       BOOLEAN DEFAULT true,
            indexed_at   TIMESTAMP DEFAULT current_timestamp,
            modified_at  TIMESTAMP,
            UNIQUE(collection, path)
        );
        "#,
    )?;

    // Create chunks table
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS chunks (
            id          VARCHAR PRIMARY KEY,
            doc_id      VARCHAR NOT NULL REFERENCES documents(id),
            seq         INTEGER NOT NULL,
            content     TEXT NOT NULL,
            start_pos   INTEGER NOT NULL,
            end_pos     INTEGER NOT NULL,
            token_count INTEGER,
            UNIQUE(doc_id, seq)
        );
        "#,
    )?;

    // Create embeddings table and its index
    // Using formatting to insert the dynamic dimension
    let embed_sql = format!(
        r#"
        CREATE TABLE IF NOT EXISTS embeddings (
            chunk_id   VARCHAR PRIMARY KEY REFERENCES chunks(id),
            model      VARCHAR NOT NULL,
            vec        FLOAT[{}],
            created_at TIMESTAMP DEFAULT current_timestamp
        );

        -- HNSW index for vector similarity search
        CREATE INDEX IF NOT EXISTS idx_embeddings_vec ON embeddings USING HNSW (vec)
        WITH (metric = 'cosine');
        "#,
        vector_dim
    );
    conn.execute_batch(&embed_sql)?;

    // Create FTS Index macro - DuckDB creates a macro rather than an index object for FTS
    // Actually PRAGMA create_fts_index(...)
    // we use IF NOT EXISTS logic implicitly or recreate if needed.
    // The spec says:
    // PRAGMA create_fts_index('documents', 'id', 'title', 'content', stemmer = 'porter', stopwords = 'english', overwrite = 1);
    
    // We only create this if documents table is empty or after an insert
    // I will write a function to reset the FTS index later during ingestion.

    // Create contexts table
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS contexts (
            path    VARCHAR PRIMARY KEY,
            context TEXT NOT NULL,
            created_at TIMESTAMP DEFAULT current_timestamp
        );
        "#,
    )?;

    Ok(conn)
}

pub fn recreate_fts_index(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        PRAGMA create_fts_index(
            'documents',
            'id',
            'title', 'content',
            stemmer = 'porter',
            stopwords = 'english',
            overwrite = 1
        );
        "#,
    )
}
