#![deny(clippy::all)]

use napi::bindgen_prelude::*;
use napi_derive::napi;
use qgoots_core::{init_db, ingest_document};
use qgoots_core::search::bm25_search as core_bm25;

#[napi]
pub struct QgootsEngine {
    db_path: String,
}

#[napi]
impl QgootsEngine {
    #[napi(constructor)]
    pub fn new(db_path: String) -> Result<Self> {
        init_db(&db_path, 768)
            .map_err(|e| Error::new(Status::GenericFailure, e.to_string()))?;
        Ok(Self { db_path })
    }

    #[napi]
    pub fn bm25_search(&self, query: String, limit: u32) -> Result<Vec<SearchResult>> {
        let conn = duckdb::Connection::open(&self.db_path)
            .map_err(|e| Error::new(Status::GenericFailure, e.to_string()))?;
        
        let results = core_bm25(&conn, &query, limit as usize)
            .map_err(|e| Error::new(Status::GenericFailure, e.to_string()))?;
        
        Ok(results.into_iter().map(|r| SearchResult {
            title: r.title,
            snippet: r.snippet,
            score: r.score,
        }).collect())
    }

    #[napi]
    pub fn ingest(&self, collection: String, path: String, content: String) -> Result<()> {
        let conn = duckdb::Connection::open(&self.db_path)
            .map_err(|e| Error::new(Status::GenericFailure, e.to_string()))?;
        
        ingest_document(&conn, &collection, &path, &content, 512, 50)
            .map_err(|e| Error::new(Status::GenericFailure, e.to_string()))?;
        Ok(())
    }
}

#[napi(object)]
pub struct SearchResult {
    pub title: String,
    pub snippet: String,
    pub score: f64,
}
