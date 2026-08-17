//! SQLite-backed implementation of the `KnowledgeStore` trait (P22 + P23).
//!
//! Concurrency model mirrors P21: a single `Arc<parking_lot::Mutex<Connection>>`
//! serializes all access. Writes are small (≤1000 docs / ≤10000 chunks) and well
//! within the single-writer budget; FTS5 makes reads O(log n) regardless of scale.
//!
//! ## Chinese search (D8)
//!
//! Content is indexed as `bigram(content)` in a **contentless** FTS5 table
//! (`chunks_fts`, `content=''`). Queries are bigram-preprocessed and wrapped in
//! double quotes so user input cannot break FTS5 syntax. The `chunks.id`
//! `INTEGER PRIMARY KEY` serves as the FTS5 `rowid` join key.
//!
//! ## P23 additions
//!
//! - `chunks.embedding` (BLOB, little-endian f32) holds per-chunk vectors.
//! - `documents_fts` is a second contentless FTS5 table over `documents.name`
//!   (bigram-preprocessed) for filename discovery — exact match, not semantic.
//! - `search_semantic` is pure SQL + cosine: it takes an **already-embedded
//!   query vector** (the QA layer owns the `EmbeddingService` and does the
//!   string→vector step, per G1: storage layer never touches the embedder).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::knowledge::chunker::{
    bigram, chunk_text, fts_query, DEFAULT_CHUNK_OVERLAP, DEFAULT_CHUNK_SIZE,
};
use crate::knowledge::error::{KnowledgeError, KnowledgeResult};
use crate::knowledge::schema::init_db;
use crate::knowledge::similarity::top_k_by_similarity;

/// A document imported into the knowledge base.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct Document {
    /// UUID v4 primary key.
    pub id: String,
    /// Display name (usually the file name).
    pub name: String,
    /// Source path the document was imported from.
    pub path: String,
    /// SHA-256 (hex) of the raw content — used for idempotent re-import.
    pub content_hash: String,
    /// Unix timestamp (seconds) of import.
    pub imported_at: i64,
    /// Number of chunks the document was split into.
    pub total_chunks: usize,
}

/// A single retrieved chunk (the raw keyword match, before LLM post-processing).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct SearchResult {
    pub document_id: String,
    pub document_name: String,
    /// 0-based index of the chunk within its source document.
    pub chunk_index: usize,
    /// Full chunk text (for LLM context and display).
    pub content: String,
    /// Start char offset (inclusive) within the source document.
    pub char_start: usize,
    /// End char offset (exclusive) within the source document.
    pub char_end: usize,
}

/// A single retrieved chunk via semantic (vector) search.
///
/// Mirrors `SearchResult` (document_id + document_name + chunk_index locate the
/// chunk; no `chunk_id` — P22 addresses chunks by document + index). Adds the
/// cosine `similarity` score in `[-1, 1]`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct SemanticSearchResult {
    pub document_id: String,
    pub document_name: String,
    /// 0-based index of the chunk within its source document.
    pub chunk_index: usize,
    /// Full chunk text (for LLM context and display).
    pub content: String,
    /// Cosine similarity to the query vector, in `[-1, 1]`.
    pub similarity: f32,
}

/// A document matched by filename discovery (exact / bigram match on `name`).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct DocumentSearchResult {
    pub document_id: String,
    pub document_name: String,
}

/// The knowledge-store contract. The SQLite backend is the default
/// implementation, but the trait keeps the backend swappable.
///
/// P23 keeps the storage layer strictly SQL-only: it never embeds text. That is
/// why `search_semantic` takes an **already-computed query vector** rather than
/// a `&str` — the `KnowledgeQAService` (which holds the `EmbeddingService`)
/// does the string→vector step and passes the vector here (decision G1).
pub trait KnowledgeStore: Send + Sync {
    /// Import a document. Chunks it, builds the FTS5 index, and stores it.
    /// Re-importing identical content (same SHA-256) is a no-op that returns the
    /// existing `Document` (idempotency, acceptance #2).
    fn import_document(&self, path: &Path, name: &str, content: &str) -> KnowledgeResult<Document>;

    /// List all imported documents, newest first.
    fn list_documents(&self) -> KnowledgeResult<Vec<Document>>;

    /// Fetch a single document by id, or `None` if unknown.
    fn get_document(&self, id: &str) -> KnowledgeResult<Option<Document>>;

    /// Delete a document and all its chunks (cascade) plus the FTS5 rows.
    fn delete_document(&self, id: &str) -> KnowledgeResult<()>;

    /// Keyword search over all chunks. Returns up to `limit` chunks ranked by
    /// BM25 (`chunks_fts.rank`). Empty/whitespace queries return no results.
    fn search_only(&self, query: &str, limit: usize) -> KnowledgeResult<Vec<SearchResult>>;

    /// Semantic (vector) search. `query_vec` must already be embedded by the
    /// caller. Returns up to `top_k` chunks ranked by cosine similarity, reading
    /// only chunks whose `embedding` is not NULL. Chunks without an embedding
    /// are skipped.
    fn search_semantic(
        &self,
        query_vec: &[f32],
        top_k: usize,
    ) -> KnowledgeResult<Vec<SemanticSearchResult>>;

    /// Filename discovery: exact/bigram match over `documents.name`. Returns up
    /// to `limit` documents whose name matches the query.
    fn search_documents(
        &self,
        query: &str,
        limit: usize,
    ) -> KnowledgeResult<Vec<DocumentSearchResult>>;

    /// Write a chunk's embedding (little-endian f32 BLOB). Pure SQL.
    fn write_embedding(&self, chunk_id: i64, embedding: &[f32]) -> KnowledgeResult<()>;

    /// All chunks that have not yet been embedded (id, content), for batching.
    fn unembedded_chunks(&self) -> KnowledgeResult<Vec<(i64, String)>>;

    /// Count of chunks that have not yet been embedded.
    fn unembedded_chunk_count(&self) -> KnowledgeResult<usize>;

    /// Count of chunks that have an embedding (used by `ask(Auto)` to decide
    /// whether the semantic path is available).
    fn embedded_chunk_count(&self) -> KnowledgeResult<usize>;

    /// All chunks of a single document (id, content), for `embed_document`.
    fn chunks_of_document(&self, document_id: &str) -> KnowledgeResult<Vec<(i64, String)>>;

    /// True if at least one chunk has a non-null embedding. Used by the Auto
    /// retrieval strategy to decide between Semantic and Keyword.
    fn has_embeddings(&self) -> KnowledgeResult<bool>;

    /// Run `PRAGMA integrity_check`; `Ok(())` iff the result is `"ok"`.
    fn integrity_check(&self) -> KnowledgeResult<()>;
}

/// SQLite implementation of [`KnowledgeStore`].
pub struct SqliteKnowledgeStore {
    conn: Arc<Mutex<Connection>>,
    db_path: PathBuf,
}

impl SqliteKnowledgeStore {
    /// Open (creating + initializing if needed) the knowledge DB at `db_path`.
    pub fn open(db_path: &Path) -> KnowledgeResult<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(db_path)?;
        init_db(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            db_path: db_path.to_path_buf(),
        })
    }

    /// Open from CaspianFlow paths (`~/.caspian/knowledge/knowledge.db`).
    pub fn from_paths(paths: &crate::config::CaspianPaths) -> KnowledgeResult<Self> {
        Self::open(&paths.knowledge.join("knowledge.db"))
    }

    /// The on-disk database path.
    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    /// Rebuild `documents_fts` from scratch (clear + re-index every document
    /// name). Idempotent. Intended for recovering an upgraded v1→v2 database
    /// whose documents predate `documents_fts`; fresh imports index themselves
    /// in `import_document`, so this is normally unnecessary.
    ///
    /// `documents_fts` is a *contentless* FTS5 table, which rejects ordinary
    /// `DELETE` statements — each existing entry must be removed with the
    /// special `'delete'` command before re-inserting.
    pub fn reindex_document_names(&self) -> KnowledgeResult<usize> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        // Enumerate current documents (need their implicit integer rowid + name).
        let mut stmt = tx.prepare("SELECT rowid, name FROM documents")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
        let docs: Vec<(i64, String)> = rows.map(|r| r.unwrap()).collect();
        drop(stmt);

        // Scrub existing name entries (contentless: per-row 'delete' command).
        for (rowid, name) in &docs {
            let bg = bigram(name);
            tx.execute(
                "INSERT INTO documents_fts(documents_fts, rowid, name) VALUES('delete', ?1, ?2)",
                params![rowid, bg],
            )?;
        }
        // Re-insert all names.
        let mut count = 0usize;
        for (rowid, name) in &docs {
            let bg = bigram(name);
            tx.execute(
                "INSERT INTO documents_fts(rowid, name) VALUES (?1, ?2)",
                params![rowid, bg],
            )?;
            count += 1;
        }
        tx.commit()?;
        Ok(count)
    }
}

impl KnowledgeStore for SqliteKnowledgeStore {
    fn import_document(&self, path: &Path, name: &str, content: &str) -> KnowledgeResult<Document> {
        if content.trim().is_empty() {
            return Err(KnowledgeError::EmptyContent);
        }
        let content_hash = sha256_hex(content);

        // Idempotency: identical content already imported -> return existing.
        {
            let conn = self.conn.lock();
            if let Ok(existing) = conn.query_row(
                "SELECT id, name, path, content_hash, imported_at, total_chunks \
                 FROM documents WHERE content_hash = ?1",
                params![content_hash],
                row_to_document,
            ) {
                return Ok(existing);
            }
        }

        let id = Uuid::new_v4().to_string();
        let imported_at = now_secs();
        let chunks = chunk_text(content, DEFAULT_CHUNK_SIZE, DEFAULT_CHUNK_OVERLAP);

        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO documents (id, name, path, content_hash, imported_at, total_chunks)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                id,
                name,
                path.to_string_lossy().to_string(),
                content_hash,
                imported_at,
                chunks.len() as i64,
            ],
        )?;
        // Capture the document's implicit integer rowid — it is the join key for
        // the contentless `documents_fts` table.
        let doc_rowid = tx.last_insert_rowid();

        for (idx, chunk) in chunks.iter().enumerate() {
            tx.execute(
                "INSERT INTO chunks (document_id, chunk_index, content, char_start, char_end)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    id,
                    idx as i64,
                    chunk.content,
                    chunk.char_start as i64,
                    chunk.char_end as i64,
                ],
            )?;
            let rowid = tx.last_insert_rowid();
            let bg = bigram(&chunk.content);
            tx.execute(
                "INSERT INTO chunks_fts (rowid, bigram) VALUES (?1, ?2)",
                params![rowid, bg],
            )?;
        }

        // P23: index the document name for filename discovery. Sync, zero-model,
        // additive — does not alter import_document's contract or its P22
        // acceptance tests. (A document imported under v1 and never re-imported
        // after upgrade is covered by `reindex_document_names`.)
        let name_bg = bigram(name);
        tx.execute(
            "INSERT INTO documents_fts (rowid, name) VALUES (?1, ?2)",
            params![doc_rowid, name_bg],
        )?;

        tx.commit()?;

        Ok(Document {
            id,
            name: name.to_string(),
            path: path.to_string_lossy().to_string(),
            content_hash,
            imported_at,
            total_chunks: chunks.len(),
        })
    }

    fn list_documents(&self) -> KnowledgeResult<Vec<Document>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, name, path, content_hash, imported_at, total_chunks \
             FROM documents ORDER BY imported_at DESC",
        )?;
        let rows = stmt.query_map([], row_to_document)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    fn get_document(&self, id: &str) -> KnowledgeResult<Option<Document>> {
        let conn = self.conn.lock();
        let res = conn.query_row(
            "SELECT id, name, path, content_hash, imported_at, total_chunks \
             FROM documents WHERE id = ?1",
            params![id],
            row_to_document,
        );
        match res {
            Ok(d) => Ok(Some(d)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(KnowledgeError::Sqlite(e)),
        }
    }

    fn delete_document(&self, id: &str) -> KnowledgeResult<()> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;

        // Contentless FTS5 does NOT auto-remove rows on chunk deletion, so we
        // must issue the explicit 'delete' command with the (recomputed) bigram.
        let mut stmt = tx.prepare("SELECT id, content FROM chunks WHERE document_id = ?1")?;
        let rows = stmt.query_map(params![id], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (rowid, content) = row?;
            let bg = bigram(&content);
            tx.execute(
                "INSERT INTO chunks_fts(chunks_fts, rowid, bigram) VALUES('delete', ?1, ?2)",
                params![rowid, bg],
            )?;
        }
        drop(stmt);

        // P23: also scrub `documents_fts` for this document.
        let mut meta = tx.prepare("SELECT rowid, name FROM documents WHERE id = ?1")?;
        let meta_row = meta.query_row(params![id], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        });
        drop(meta);
        if let Ok((doc_rowid, name)) = meta_row {
            let name_bg = bigram(&name);
            tx.execute(
                "INSERT INTO documents_fts(documents_fts, rowid, name) VALUES('delete', ?1, ?2)",
                params![doc_rowid, name_bg],
            )?;
        }

        let n = tx.execute("DELETE FROM documents WHERE id = ?1", params![id])?;
        if n == 0 {
            return Err(KnowledgeError::NotFound(id.to_string()));
        }
        tx.commit()?;
        Ok(())
    }

    fn search_only(&self, query: &str, limit: usize) -> KnowledgeResult<Vec<SearchResult>> {
        // A whitespace-only query yields no bigram tokens -> no match possible.
        let fts = fts_query(query);
        if fts.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT c.document_id, c.chunk_index, c.content, c.char_start, c.char_end, d.name \
             FROM chunks_fts \
             JOIN chunks c ON c.id = chunks_fts.rowid \
             JOIN documents d ON d.id = c.document_id \
             WHERE chunks_fts MATCH ?1 \
             ORDER BY chunks_fts.rank \
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![fts, limit as i64], row_to_search_result)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    fn search_semantic(
        &self,
        query_vec: &[f32],
        top_k: usize,
    ) -> KnowledgeResult<Vec<SemanticSearchResult>> {
        if query_vec.is_empty() || top_k == 0 {
            return Ok(Vec::new());
        }
        let conn = self.conn.lock();
        // Read every embedded chunk with its document name for display.
        let mut stmt = conn.prepare(
            "SELECT c.id, c.document_id, d.name, c.chunk_index, c.content, c.embedding \
             FROM chunks c \
             JOIN documents d ON d.id = c.document_id \
             WHERE c.embedding IS NOT NULL",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,          // chunk id
                r.get::<_, String>(1)?,       // document_id
                r.get::<_, String>(2)?,       // document_name
                r.get::<_, i64>(3)? as usize, // chunk_index
                r.get::<_, String>(4)?,       // content
                r.get::<_, Vec<u8>>(5)?,      // embedding BLOB
            ))
        })?;

        let mut candidates: Vec<Vec<f32>> = Vec::new();
        let mut meta: Vec<(i64, String, String, usize, String)> = Vec::new();
        for row in rows {
            let (cid, did, dname, cidx, content, blob) = row?;
            match embedding_from_bytes(&blob) {
                Some(vec) => {
                    candidates.push(vec);
                    meta.push((cid, did, dname, cidx, content));
                }
                None => continue, // malformed blob (length not multiple of 4): skip.
            }
        }

        let top = top_k_by_similarity(query_vec, &candidates, top_k);
        let mut out = Vec::with_capacity(top.len());
        for (idx, sim) in top {
            let (_, did, dname, cidx, content) = &meta[idx];
            out.push(SemanticSearchResult {
                document_id: did.clone(),
                document_name: dname.clone(),
                chunk_index: *cidx,
                content: content.clone(),
                similarity: sim,
            });
        }
        Ok(out)
    }

    fn search_documents(
        &self,
        query: &str,
        limit: usize,
    ) -> KnowledgeResult<Vec<DocumentSearchResult>> {
        let fts = fts_query(query);
        if fts.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT d.id, d.name \
             FROM documents_fts \
             JOIN documents d ON d.rowid = documents_fts.rowid \
             WHERE documents_fts MATCH ?1 \
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![fts, limit as i64], |r| {
            Ok(DocumentSearchResult {
                document_id: r.get(0)?,
                document_name: r.get(1)?,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    fn write_embedding(&self, chunk_id: i64, embedding: &[f32]) -> KnowledgeResult<()> {
        let conn = self.conn.lock();
        conn.execute(
            "UPDATE chunks SET embedding = ?1 WHERE id = ?2",
            params![embedding_to_bytes(embedding), chunk_id],
        )?;
        Ok(())
    }

    fn unembedded_chunks(&self) -> KnowledgeResult<Vec<(i64, String)>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare("SELECT id, content FROM chunks WHERE embedding IS NULL")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    fn unembedded_chunk_count(&self) -> KnowledgeResult<usize> {
        let conn = self.conn.lock();
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM chunks WHERE embedding IS NULL",
            [],
            |r| r.get(0),
        )?;
        Ok(n as usize)
    }

    fn embedded_chunk_count(&self) -> KnowledgeResult<usize> {
        let conn = self.conn.lock();
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM chunks WHERE embedding IS NOT NULL",
            [],
            |r| r.get(0),
        )?;
        Ok(n as usize)
    }

    fn chunks_of_document(&self, document_id: &str) -> KnowledgeResult<Vec<(i64, String)>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, content FROM chunks WHERE document_id = ?1 ORDER BY chunk_index",
        )?;
        let rows = stmt.query_map(params![document_id], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    fn has_embeddings(&self) -> KnowledgeResult<bool> {
        let conn = self.conn.lock();
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM chunks WHERE embedding IS NOT NULL",
            [],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }

    fn integrity_check(&self) -> KnowledgeResult<()> {
        let conn = self.conn.lock();
        let res: String = conn.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
        if res != "ok" {
            return Err(KnowledgeError::Integrity(res));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Embedding (de)serialization — little-endian f32 BLOB.
//
// Dimension is recovered from the blob length (`len / 4`); no separate column
// is needed. `byte_len == dim * 4` is asserted implicitly by `from_bytes`.
// ---------------------------------------------------------------------------

/// Serialize an `f32` vector to a little-endian BLOB.
pub(crate) fn embedding_to_bytes(v: &[f32]) -> Vec<u8> {
    let mut b = Vec::with_capacity(v.len() * 4);
    for f in v {
        b.extend_from_slice(&f.to_le_bytes());
    }
    b
}

/// Deserialize a little-endian `f32` BLOB. Returns `None` if the length is not a
/// multiple of 4 (corrupt / wrong dimension) so callers can skip it safely.
pub(crate) fn embedding_from_bytes(b: &[u8]) -> Option<Vec<f32>> {
    if !b.len().is_multiple_of(4) {
        return None;
    }
    let n = b.len() / 4;
    let mut v = Vec::with_capacity(n);
    for i in 0..n {
        let mut arr = [0u8; 4];
        arr.copy_from_slice(&b[i * 4..i * 4 + 4]);
        v.push(f32::from_le_bytes(arr));
    }
    Some(v)
}

// ---------------------------------------------------------------------------
// Row mappers + helpers
// ---------------------------------------------------------------------------

fn row_to_document(row: &rusqlite::Row) -> rusqlite::Result<Document> {
    Ok(Document {
        id: row.get(0)?,
        name: row.get(1)?,
        path: row.get(2)?,
        content_hash: row.get(3)?,
        imported_at: row.get(4)?,
        total_chunks: row.get::<_, i64>(5)? as usize,
    })
}

fn row_to_search_result(row: &rusqlite::Row) -> rusqlite::Result<SearchResult> {
    Ok(SearchResult {
        document_id: row.get(0)?,
        chunk_index: row.get::<_, i64>(1)? as usize,
        content: row.get(2)?,
        char_start: row.get::<_, i64>(3)? as usize,
        char_end: row.get::<_, i64>(4)? as usize,
        document_name: row.get(5)?,
    })
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn sha256_hex(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> (tempfile::TempDir, SqliteKnowledgeStore) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("knowledge.db");
        let store = SqliteKnowledgeStore::open(&db).unwrap();
        (dir, store)
    }

    /// Fetch chunk ids for a document (test-only; reaches private `conn`).
    fn chunk_ids(store: &SqliteKnowledgeStore, document_id: &str) -> Vec<i64> {
        let conn = store.conn.lock();
        let mut stmt = conn
            .prepare("SELECT id FROM chunks WHERE document_id = ?1 ORDER BY chunk_index")
            .unwrap();
        let rows = stmt
            .query_map(params![document_id], |r| r.get::<_, i64>(0))
            .unwrap();
        rows.map(|r| r.unwrap()).collect()
    }

    const ZH_DOC: &str = "工作流引擎支持 DAG 拓扑排序，采用 petgraph 实现。\n\n缓存策略采用哈希键精确匹配，中间结果落盘复用。";
    const EN_DOC: &str = "The workflow engine supports DAG topological sorting via petgraph.\n\nThe cache uses exact hash-key matching for intermediate results.";
    // A long doc that chunks into many pieces (scale / performance test #10).
    fn long_doc() -> String {
        "工作流引擎支持 DAG 拓扑排序，采用 petgraph 实现。\n\n缓存策略采用哈希键精确匹配，中间结果落盘复用。\n\n"
            .repeat(400)
    }

    // Acceptance #1: import TXT -> documents row exists.
    #[test]
    fn test_import_txt_creates_document() {
        let (_dir, store) = temp_store();
        let doc = store
            .import_document(Path::new("/docs/readme.txt"), "readme.txt", ZH_DOC)
            .unwrap();
        assert_eq!(doc.name, "readme.txt");
        assert!(doc.total_chunks >= 1);
        let list = store.list_documents().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, doc.id);
    }

    // Acceptance #2: importing identical content twice is idempotent.
    #[test]
    fn test_import_idempotent() {
        let (_dir, store) = temp_store();
        let a = store
            .import_document(Path::new("/docs/a.txt"), "a.txt", ZH_DOC)
            .unwrap();
        let b = store
            .import_document(Path::new("/docs/a_copy.txt"), "a_copy.txt", ZH_DOC)
            .unwrap();
        assert_eq!(
            a.id, b.id,
            "identical content must map to the same document"
        );
        assert_eq!(store.list_documents().unwrap().len(), 1);
    }

    // Acceptance #3: Chinese retrieval works via bigram.
    #[test]
    fn test_search_chinese_bigram() {
        let (_dir, store) = temp_store();
        let doc = store
            .import_document(Path::new("/docs/wf.md"), "wf.md", ZH_DOC)
            .unwrap();
        let hits = store.search_only("工作流", 10).unwrap();
        assert!(!hits.is_empty(), "should find the doc by '工作流'");
        assert!(hits.iter().all(|h| h.document_id == doc.id));
        // A 2-char term that exists as a bigram token must also match.
        let hits2 = store.search_only("引擎", 10).unwrap();
        assert!(!hits2.is_empty(), "2-char term '引擎' should match");
    }

    // Acceptance #4: English retrieval works.
    #[test]
    fn test_search_english() {
        let (_dir, store) = temp_store();
        let doc = store
            .import_document(Path::new("/docs/wf.md"), "wf.md", EN_DOC)
            .unwrap();
        let hits = store.search_only("workflow", 10).unwrap();
        assert!(!hits.is_empty(), "should find the doc by 'workflow'");
        assert!(hits.iter().all(|h| h.document_id == doc.id));
    }

    // Acceptance #5: short 2-char Chinese term returns correct results.
    #[test]
    fn test_search_short_term() {
        let (_dir, store) = temp_store();
        store
            .import_document(Path::new("/docs/wf.md"), "wf.md", ZH_DOC)
            .unwrap();
        // "缓存" is a 2-char term present in the doc.
        let hits = store.search_only("缓存", 10).unwrap();
        assert!(!hits.is_empty(), "2-char term '缓存' should match");
        assert!(hits.iter().any(|h| h.content.contains("缓存")));
    }

    // Acceptance #9: deleting a document cascades to its chunks AND FTS rows.
    #[test]
    fn test_delete_cascade() {
        let (_dir, store) = temp_store();
        let doc = store
            .import_document(Path::new("/docs/wf.md"), "wf.md", ZH_DOC)
            .unwrap();
        // Sanity: a search finds it before deletion.
        assert!(!store.search_only("工作流", 10).unwrap().is_empty());

        store.delete_document(&doc.id).unwrap();

        // Document gone.
        assert!(store.get_document(&doc.id).unwrap().is_none());
        assert!(store.list_documents().unwrap().is_empty());
        // Chunks gone (cascade).
        let conn = store.conn.lock();
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM chunks WHERE document_id = ?1",
                [doc.id.clone()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 0, "chunks must be cascade-deleted");
        // FTS rows gone too (no orphaned index entries).
        let orphan: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM chunks_fts WHERE chunks_fts MATCH ?1",
                [fts_query("工作流")],
                |r| r.get(0),
            )
            .unwrap_or(0);
        assert_eq!(
            orphan, 0,
            "FTS index must not retain orphaned rows after delete"
        );
    }

    // P23: deleting a document also scrubs its `documents_fts` name entry.
    #[test]
    fn test_delete_cleans_documents_fts() {
        let (_dir, store) = temp_store();
        let doc = store
            .import_document(Path::new("/docs/wf.md"), "工作流引擎设计.md", ZH_DOC)
            .unwrap();
        assert!(!store.search_documents("引擎", 10).unwrap().is_empty());

        store.delete_document(&doc.id).unwrap();

        let found = store.search_documents("引擎", 10).unwrap();
        assert!(
            found.is_empty(),
            "documents_fts must not retain orphaned name after delete"
        );
    }

    // Acceptance #10: list documents returns the document list.
    #[test]
    fn test_list_documents() {
        let (_dir, store) = temp_store();
        store
            .import_document(Path::new("/a.txt"), "a.txt", ZH_DOC)
            .unwrap();
        store
            .import_document(Path::new("/b.txt"), "b.txt", EN_DOC)
            .unwrap();
        let list = store.list_documents().unwrap();
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn test_delete_missing_is_not_found() {
        let (_dir, store) = temp_store();
        assert!(matches!(
            store.delete_document("nope"),
            Err(KnowledgeError::NotFound(_))
        ));
    }

    #[test]
    fn test_empty_query_returns_nothing() {
        let (_dir, store) = temp_store();
        store
            .import_document(Path::new("/a.txt"), "a.txt", ZH_DOC)
            .unwrap();
        assert!(store.search_only("   ", 10).unwrap().is_empty());
    }

    #[test]
    fn test_multi_document_search_scoped_by_match() {
        let (_dir, store) = temp_store();
        let zh = store
            .import_document(Path::new("/zh.md"), "zh.md", ZH_DOC)
            .unwrap();
        store
            .import_document(Path::new("/en.md"), "en.md", EN_DOC)
            .unwrap();
        let hits = store.search_only("工作流", 10).unwrap();
        // Only the Chinese doc should match the Chinese query.
        assert!(!hits.is_empty());
        assert!(hits.iter().all(|h| h.document_id == zh.id));
    }

    // ── P23 semantic / filename tests ────────────────────────────────────────

    const DIM: usize = 4;

    fn synth(vec: &[f32]) -> Vec<f32> {
        vec.to_vec()
    }

    // Acceptance #1 (store-level): write_embedding persists a BLOB that reads
    // back as the same vector; IS NOT NULL count is correct.
    #[test]
    fn test_write_embedding_roundtrip() {
        let (_dir, store) = temp_store();
        let doc = store
            .import_document(Path::new("/d.md"), "d.md", ZH_DOC)
            .unwrap();
        let ids = chunk_ids(&store, &doc.id);
        assert!(!ids.is_empty());

        store
            .write_embedding(ids[0], &synth(&[1.0, 0.0, 0.0, 0.0]))
            .unwrap();

        let conn = store.conn.lock();
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM chunks WHERE embedding IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1, "exactly one chunk should be embedded");
        let blob: Vec<u8> = conn
            .query_row(
                "SELECT embedding FROM chunks WHERE id = ?1",
                params![ids[0]],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(blob.len(), DIM * 4);
        let back = embedding_from_bytes(&blob).unwrap();
        assert_eq!(back, vec![1.0, 0.0, 0.0, 0.0]);
    }

    // Acceptance #2: freshly imported doc reports unembedded chunks.
    #[test]
    fn test_unembedded_chunk_count() {
        let (_dir, store) = temp_store();
        let doc = store
            .import_document(Path::new("/d.md"), "d.md", ZH_DOC)
            .unwrap();
        let ids = chunk_ids(&store, &doc.id);
        assert!(
            store.unembedded_chunk_count().unwrap() >= ids.len(),
            "every chunk of a freshly imported doc is unembedded"
        );
        // After embedding everything it should be zero.
        for id in &ids {
            store.write_embedding(*id, &synth(&[0.1; DIM])).unwrap();
        }
        assert_eq!(store.unembedded_chunk_count().unwrap(), 0);
    }

    // Acceptance #3: search_semantic orders by descending cosine similarity.
    #[test]
    fn test_search_semantic_ordered_by_similarity() {
        let (_dir, store) = temp_store();
        let doc = store
            .import_document(Path::new("/d.md"), "d.md", &long_doc())
            .unwrap();
        let ids = chunk_ids(&store, &doc.id);
        assert!(ids.len() >= 2, "need >=2 chunks to rank");

        // Make chunk[0] align with the query, chunk[1] orthogonal.
        let query = synth(&[1.0, 0.0, 0.0, 0.0]);
        store.write_embedding(ids[0], &query).unwrap();
        store
            .write_embedding(ids[1], &synth(&[0.0, 1.0, 0.0, 0.0]))
            .unwrap();
        // Embed any remaining chunks with noise so they rank below.
        for id in &ids[2..] {
            store
                .write_embedding(*id, &synth(&[0.3, 0.3, 0.3, 0.3]))
                .unwrap();
        }

        let top = store.search_semantic(&query, ids.len()).unwrap();
        assert!(!top.is_empty());
        assert!(
            top[0].similarity > top[1].similarity,
            "must be ranked by similarity desc"
        );
        assert_eq!(
            top[0].chunk_index, 0,
            "most similar chunk (chunk_index 0) must be first"
        );
    }

    // Acceptance #4: search_semantic skips chunks without an embedding.
    #[test]
    fn test_search_semantic_skips_unembedded() {
        let (_dir, store) = temp_store();
        let doc = store
            .import_document(Path::new("/d.md"), "d.md", &long_doc())
            .unwrap();
        let ids = chunk_ids(&store, &doc.id);
        assert!(ids.len() >= 2);

        // Embed only the first chunk.
        store
            .write_embedding(ids[0], &synth(&[1.0, 0.0, 0.0, 0.0]))
            .unwrap();

        let query = synth(&[1.0, 0.0, 0.0, 0.0]);
        let top = store.search_semantic(&query, ids.len()).unwrap();
        assert_eq!(top.len(), 1, "only the embedded chunk may be returned");
        assert_eq!(top[0].chunk_index, 0);
    }

    // Acceptance #5: filename discovery finds a document by name substring.
    #[test]
    fn test_search_documents_by_name() {
        let (_dir, store) = temp_store();
        store
            .import_document(Path::new("/a.md"), "工作流引擎设计.md", ZH_DOC)
            .unwrap();
        store
            .import_document(Path::new("/b.md"), "缓存策略说明.md", EN_DOC)
            .unwrap();

        let hits = store.search_documents("引擎", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].document_name, "工作流引擎设计.md");

        let hits2 = store.search_documents("缓存", 10).unwrap();
        assert_eq!(hits2.len(), 1);
        assert_eq!(hits2[0].document_name, "缓存策略说明.md");
    }

    // Acceptance #6 (regression): keyword search unaffected by P23 schema.
    #[test]
    fn test_search_only_unchanged() {
        let (_dir, store) = temp_store();
        store
            .import_document(Path::new("/d.md"), "d.md", ZH_DOC)
            .unwrap();
        let hits = store.search_only("工作流", 10).unwrap();
        assert!(!hits.is_empty());
    }

    // Malformed (non-multiple-of-4) blobs are skipped, not panicked.
    #[test]
    fn test_search_semantic_skips_malformed_blob() {
        let (_dir, store) = temp_store();
        let doc = store
            .import_document(Path::new("/d.md"), "d.md", ZH_DOC)
            .unwrap();
        let ids = chunk_ids(&store, &doc.id);
        // Write a deliberately malformed 3-byte blob via raw SQL.
        store
            .conn
            .lock()
            .execute(
                "UPDATE chunks SET embedding = ?1 WHERE id = ?2",
                params![vec![1u8, 2, 3], ids[0]],
            )
            .unwrap();
        let query = synth(&[1.0, 0.0, 0.0, 0.0]);
        let top = store.search_semantic(&query, 10).unwrap();
        assert!(top.is_empty(), "malformed blob must be skipped");
    }

    // ── Acceptance #10 (scale / perf, ignored): real P95 of the semantic scan
    // over ~10k chunks. Uses SYNTHETIC embeddings (deterministic per chunk id)
    // so it needs no model weights — it measures the SQL read + cosine + Top-K
    // path that a real embedder would feed. Run manually:
    //   cargo test --lib knowledge::store::tests::test_scale_semantic_search_perf -- --ignored --nocapture
    // (M4: report the real number, never a fabricated baseline.)
    #[test]
    #[ignore]
    fn test_scale_semantic_search_perf() {
        let (_dir, store) = temp_store();
        let doc_count = 300; // each distinct doc chunks into ~35 pieces -> ~10k chunks
        for i in 0..doc_count {
            // Vary content per doc so import_document's SHA-256 idempotency does
            // not collapse all 300 into a single document.
            let content = format!("{}\n\n文档编号 {i}\n", long_doc());
            store
                .import_document(
                    Path::new(&format!("/d{i}.md")),
                    &format!("d{i}.md"),
                    &content,
                )
                .unwrap();
        }

        // Collect chunk ids and write synthetic embeddings of a fixed dimension.
        let ids: Vec<i64> = {
            let conn = store.conn.lock();
            let mut stmt = conn.prepare("SELECT id FROM chunks").unwrap();
            let rows = stmt.query_map([], |r| r.get::<_, i64>(0)).unwrap();
            rows.map(|r| r.unwrap()).collect()
        };
        let dim = 64usize;
        for (k, id) in ids.iter().enumerate() {
            let v: Vec<f32> = (0..dim)
                .map(|j| ((k * 31 + j) % 100) as f32 / 100.0)
                .collect();
            store.write_embedding(*id, &v).unwrap();
        }
        let n = ids.len();

        let q = vec![0.5f32; dim];
        // Warmup (page cache / first query).
        let _ = store.search_semantic(&q, 10).unwrap();

        let iters = 20usize;
        let mut times: Vec<std::time::Duration> = Vec::with_capacity(iters);
        for _ in 0..iters {
            let start = std::time::Instant::now();
            let top = store.search_semantic(&q, 10).unwrap();
            times.push(start.elapsed());
            assert_eq!(top.len(), 10, "must return top-10");
        }
        times.sort();
        let mean = times.iter().sum::<std::time::Duration>() / iters as u32;
        let p95 = times[(iters as f64 * 0.95) as usize];
        println!("P23 scale: {n} chunks, dim={dim}, iters={iters}");
        println!("  mean = {:?}, p95 = {:?}", mean, p95);
    }
}
