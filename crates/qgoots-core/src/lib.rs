pub mod db;
pub mod chunker;
pub mod search;
pub mod embedder;
pub mod ingest;

// Re-exports
pub use db::init_db;
pub use ingest::ingest_document;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ingest_and_search() {
        let conn = init_db("", 768).expect("Failed to init in-memory db");
        
        conn.execute(
            "INSERT INTO collections (name, path, pattern) VALUES (?, ?, ?)",
            duckdb::params!["test_collection", "/test", "**/*.md"],
        ).expect("Failed to add collection");
        
        let markdown = "# Hello qgoots\n\nThis is a test document.";
        ingest_document(&conn, "test_collection", "test.md", markdown, 512, 50)
            .expect("Ingestion failed");
        
        db::recreate_fts_index(&conn).expect("Failed to create FTS index");

        let results = search::bm25_search(&conn, "qgoots", 5).expect("Search failed");
        
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "test.md");
        assert!(results[0].snippet.contains("Hello qgoots"));
    }
}
