use pyo3::prelude::*;
use qgoots_core::init_db;
use qgoots_core::search::bm25_search as core_bm25;
use qgoots_core::ingest_document as core_ingest;

#[pyclass]
pub struct QgootsEngine {
    db_path: String,
}

#[pymethods]
impl QgootsEngine {
    #[new]
    fn new(db_path: String) -> PyResult<Self> {
        // Initialize DB to ensure schemas exist
        init_db(&db_path, 768).map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;
        Ok(QgootsEngine { db_path })
    }

    fn bm25_search(&self, query: String, limit: usize) -> PyResult<Vec<(String, String, Option<f64>)>> {
        let conn = duckdb::Connection::open(&self.db_path)
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;
        
        let results = core_bm25(&conn, &query, limit)
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;
        
        Ok(results.into_iter().map(|r| (r.title, r.snippet, Some(r.score))).collect())
    }

    fn ingest(&self, collection: String, path: String, content: String) -> PyResult<()> {
        let conn = duckdb::Connection::open(&self.db_path)
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;
        
        core_ingest(&conn, &collection, &path, &content, 512, 50)
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;
        Ok(())
    }
}

#[pymodule]
fn qgoots(_py: Python, m: &PyModule) -> PyResult<()> {
    m.add_class::<QgootsEngine>()?;
    Ok(())
}
