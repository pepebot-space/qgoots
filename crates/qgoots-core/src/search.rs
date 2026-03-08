use duckdb::{Connection, Result, params};

#[derive(Debug, serde::Serialize)]
pub struct SearchResult {
    pub score: f64,
    pub collection: String,
    pub path: String,
    pub title: String,
    pub snippet: String,
}

pub fn bm25_search(conn: &Connection, query: &str, limit: usize) -> Result<Vec<SearchResult>> {
    let mut stmt = conn.prepare(
        r#"
        SELECT d.id, d.title, d.path, d.collection,
            fts_main_documents.match_bm25(d.id, ?) AS score,
            substring(d.content, 1, 200) as snippet
        FROM documents d
        WHERE score IS NOT NULL
        ORDER BY score DESC
        LIMIT ?
        "#
    )?;

    let results = stmt.query_map(params![query, limit as i32], |row| {
        Ok(SearchResult {
            title: row.get(1).unwrap_or_default(),
            path: row.get(2)?,
            collection: row.get(3)?,
            score: row.get(4)?,
            snippet: row.get(5)?,
        })
    })?;

    let mut out = Vec::new();
    for res in results {
        out.push(res?);
    }
    
    Ok(out)
}

// Vector search and Hybrid search would go here, taking vector dimensions
// and using DuckDB array_cosine_distance on the HNSW index.
