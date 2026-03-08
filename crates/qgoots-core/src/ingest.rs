use crate::db;
use crate::chunker;
use duckdb::{Connection, params};
use sha2::{Digest, Sha256};
use std::time::SystemTime;

pub fn compute_sha256(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

pub fn ingest_document(
    conn: &Connection,
    collection: &str,
    path: &str,
    content: &str,
    target_tokens: usize,
    overlap: usize,
) -> anyhow::Result<()> {
    let content_hash = compute_sha256(content);
    let id = compute_sha256(&format!("{}:{}", collection, path));
    
    // Check if unchanged
    let mut stmt = conn.prepare("SELECT content_hash FROM documents WHERE id = ?")?;
    let mut rows = stmt.query(params![&id])?;
    if let Some(row) = rows.next()? {
        let existing_hash: String = row.get(0)?;
        if existing_hash == content_hash {
            return Ok(());
        }
    }

    // Upsert Document
    let title = path.split('/').last().unwrap_or("Untitled").to_string(); // simple title
    let byte_size = content.len() as i32;
    
    conn.execute(
        r#"
        INSERT INTO documents (id, collection, path, title, content_hash, content, byte_size, active)
        VALUES (?, ?, ?, ?, ?, ?, ?, true)
        ON CONFLICT(collection, path) DO UPDATE SET
            title = excluded.title,
            content_hash = excluded.content_hash,
            content = excluded.content,
            byte_size = excluded.byte_size,
            active = true,
            indexed_at = now();
        "#,
        params![id, collection, path, title, content_hash, content, byte_size],
    )?;

    // Delete old chunks
    conn.execute("DELETE FROM chunks WHERE doc_id = ?", params![id])?;

    // Create chunks
    let chunks = chunker::chunk_markdown(content, target_tokens, overlap);
    for (seq, chunk) in chunks.into_iter().enumerate() {
        let chunk_id = format!("{}:{}", id, seq);
        conn.execute(
            r#"
            INSERT INTO chunks (id, doc_id, seq, content, start_pos, end_pos, token_count)
            VALUES (?, ?, ?, ?, ?, ?, ?)
            "#,
            params![
                chunk_id,
                id,
                seq as i32,
                chunk.content,
                chunk.start_pos as i32,
                chunk.end_pos as i32,
                chunk.token_count as i32
            ],
        )?;
    }

    Ok(())
}
