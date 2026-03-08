package qgoots

/*
#cgo LDFLAGS: -L../c/target/release -lqgoots_c
#include <stdlib.h>
#include <stdbool.h>

typedef struct {
    char* title;
    char* snippet;
    double score;
} QgootsSearchResult;

typedef struct {
    QgootsSearchResult* results;
    size_t len;
} QgootsSearchResults;

bool qgoots_init(const char* db_path);
bool qgoots_ingest(const char* db_path, const char* collection, const char* path, const char* content);
QgootsSearchResults qgoots_search_bm25(const char* db_path, const char* query, size_t limit);
void qgoots_free_results(QgootsSearchResults res);
*/
import "C"
import (
	"errors"
	"unsafe"
)

type Engine struct {
	dbPath string
}

type SearchResult struct {
	Title   string
	Snippet string
	Score   float64
}

func NewEngine(dbPath string) (*Engine, error) {
	cDbPath := C.CString(dbPath)
	defer C.free(unsafe.Pointer(cDbPath))

	if !C.qgoots_init(cDbPath) {
		return nil, errors.New("failed to init db")
	}

	return &Engine{dbPath: dbPath}, nil
}

func (e *Engine) Ingest(collection, path, content string) error {
	cDbPath := C.CString(e.dbPath)
	cCol := C.CString(collection)
	cPath := C.CString(path)
	cContent := C.CString(content)

	defer C.free(unsafe.Pointer(cDbPath))
	defer C.free(unsafe.Pointer(cCol))
	defer C.free(unsafe.Pointer(cPath))
	defer C.free(unsafe.Pointer(cContent))

	if !C.qgoots_ingest(cDbPath, cCol, cPath, cContent) {
		return errors.New("ingestion failed")
	}
	return nil
}

func (e *Engine) SearchBM25(query string, limit int) ([]SearchResult, error) {
	cDbPath := C.CString(e.dbPath)
	cQuery := C.CString(query)
	defer C.free(unsafe.Pointer(cDbPath))
	defer C.free(unsafe.Pointer(cQuery))

	cRes := C.qgoots_search_bm25(cDbPath, cQuery, C.size_t(limit))
	defer C.qgoots_free_results(cRes)

	if cRes.results == nil {
		return nil, nil // Or an error based on your design
	}

	results := make([]SearchResult, int(cRes.len))
	slice := unsafe.Slice(cRes.results, int(cRes.len))

	for i, r := range slice {
		results[i] = SearchResult{
			Title:   C.GoString(r.title),
			Snippet: C.GoString(r.snippet),
			Score:   float64(r.score),
		}
	}

	return results, nil
}
