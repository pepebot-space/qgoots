use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::ptr;
use qgoots_core::{init_db, ingest_document};
use qgoots_core::search::bm25_search as core_bm25;

#[repr(C)]
pub struct QgootsSearchResult {
    pub title: *mut c_char,
    pub snippet: *mut c_char,
    pub score: f64,
}

#[repr(C)]
pub struct QgootsSearchResults {
    pub results: *mut QgootsSearchResult,
    pub len: usize,
}

#[no_mangle]
pub extern "C" fn qgoots_init(db_path: *const c_char) -> bool {
    if db_path.is_null() { return false; }
    let c_str = unsafe { CStr::from_ptr(db_path) };
    let path = match c_str.to_str() {
        Ok(s) => s,
        Err(_) => return false,
    };
    
    init_db(path, 768).is_ok()
}

#[no_mangle]
pub extern "C" fn qgoots_ingest(
    db_path: *const c_char,
    collection: *const c_char,
    path: *const c_char,
    content: *const c_char,
) -> bool {
    let dp = unsafe { CStr::from_ptr(db_path) }.to_string_lossy();
    let col = unsafe { CStr::from_ptr(collection) }.to_string_lossy();
    let p = unsafe { CStr::from_ptr(path) }.to_string_lossy();
    let c = unsafe { CStr::from_ptr(content) }.to_string_lossy();
    
    if let Ok(conn) = duckdb::Connection::open(&dp.into_owned()) {
        if ingest_document(&conn, &col, &p, &c, 512, 50).is_ok() {
            return true;
        }
    }
    false
}

#[no_mangle]
pub extern "C" fn qgoots_search_bm25(
    db_path: *const c_char,
    query: *const c_char,
    limit: usize,
) -> QgootsSearchResults {
    let empty = QgootsSearchResults { results: ptr::null_mut(), len: 0 };
    
    let dp = unsafe { CStr::from_ptr(db_path) }.to_string_lossy();
    let q = unsafe { CStr::from_ptr(query) }.to_string_lossy();
    
    if let Ok(conn) = duckdb::Connection::open(&dp.into_owned()) {
        if let Ok(results) = core_bm25(&conn, &q, limit) {
            let mut c_results = Vec::with_capacity(results.len());
            for r in results {
                c_results.push(QgootsSearchResult {
                    title: CString::new(r.title).unwrap().into_raw(),
                    snippet: CString::new(r.snippet).unwrap().into_raw(),
                    score: r.score,
                });
            }
            c_results.shrink_to_fit();
            let ptr = c_results.as_mut_ptr();
            let len = c_results.len();
            std::mem::forget(c_results);
            return QgootsSearchResults { results: ptr, len };
        }
    }
    
    empty
}

#[no_mangle]
pub extern "C" fn qgoots_free_results(res: QgootsSearchResults) {
    if res.results.is_null() { return; }
    let results = unsafe { Vec::from_raw_parts(res.results, res.len, res.len) };
    for r in results {
        unsafe {
            if !r.title.is_null() { let _ = CString::from_raw(r.title); }
            if !r.snippet.is_null() { let _ = CString::from_raw(r.snippet); }
        }
    }
}
