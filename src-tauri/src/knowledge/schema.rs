//! SQLite schema + migration for the P22 knowledge base.
//!
//! Schema version is tracked via `PRAGMA user_version` (idiomatic SQLite),
//! independent of `config::settings::CURRENT_SCHEMA_VERSION` (which governs
//! settings.yaml, not this database). `MIGRATIONS[i]` upgrades the DB from
//! version `i` to `i+1`. `init_db` is idempotent: safe to call on every open.
//!
//! ## Design decisions (R2 / D8)
//!
//! - `chunks.id` is an `INTEGER PRIMARY KEY` so it is the table's `rowid`. The
//!   FTS5 index joins on this `rowid` (contentless `content=''` mode).
//! - The `bigram` column is **deliberately absent** (R2): it is a pure function
//!   of `content` and is recomputed on write and on delete. This avoids storing
//!   the same text three times (content + bigram column + FTS shadow) and cuts
//!   storage ~67% at the ≤1000-doc scale.
//! - `chunks_fts` is **contentless** (`content=''`) — verified to require an
//!   explicit `'delete'` command on document removal (external-content mode does
//!   NOT auto-sync, a trap identical to P21's `content_rowid`).

use crate::knowledge::error::{KnowledgeError, KnowledgeResult};

/// Target SQLite schema version for the knowledge base.
pub const CURRENT_SCHEMA: u32 = 2;

/// SQL that brings the DB up to `CURRENT_SCHEMA`, applied incrementally.
pub static MIGRATIONS: &[&str] = &[
    // v0 -> v1: documents + chunks + contentless FTS5 index.
    r#"
    CREATE TABLE IF NOT EXISTS documents (
        id           TEXT PRIMARY KEY,
        name         TEXT NOT NULL,
        path         TEXT NOT NULL,
        content_hash TEXT NOT NULL,
        imported_at  INTEGER NOT NULL,
        total_chunks INTEGER DEFAULT 0
    );
    CREATE INDEX IF NOT EXISTS idx_documents_hash ON documents(content_hash);

    CREATE TABLE IF NOT EXISTS chunks (
        id           INTEGER PRIMARY KEY,   -- rowid alias; join key for chunks_fts
        document_id  TEXT NOT NULL,
        chunk_index  INTEGER NOT NULL,
        content      TEXT NOT NULL,
        char_start   INTEGER,
        char_end     INTEGER,
        FOREIGN KEY (document_id) REFERENCES documents(id) ON DELETE CASCADE
    );
    CREATE INDEX IF NOT EXISTS idx_chunks_document ON chunks(document_id, chunk_index);

    CREATE VIRTUAL TABLE IF NOT EXISTS chunks_fts USING fts5(bigram, content='');
    "#,
    // v1 -> v2 (P23): add `embedding BLOB` to chunks for semantic retrieval, and a
    // contentless FTS5 index over `documents.name` for filename discovery
    // (exact-match, orthogonal to the semantic path). Idempotent on re-open.
    r#"
    ALTER TABLE chunks ADD COLUMN embedding BLOB;
    CREATE VIRTUAL TABLE IF NOT EXISTS documents_fts USING fts5(name, content='');
    "#,
];

/// Idempotently initialize the DB:
/// 1. enable WAL + foreign keys,
/// 2. run pending migrations up to `CURRENT_SCHEMA`,
/// 3. run `PRAGMA integrity_check`.
pub fn init_db(conn: &rusqlite::Connection) -> KnowledgeResult<()> {
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(KnowledgeError::Sqlite)?;
    conn.pragma_update(None, "foreign_keys", "ON")
        .map_err(KnowledgeError::Sqlite)?;

    let version: u32 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(KnowledgeError::Sqlite)?;

    for (i, migration) in MIGRATIONS.iter().enumerate() {
        let target = (i + 1) as u32;
        if version < target {
            conn.execute_batch(migration)
                .map_err(|e| KnowledgeError::Migration {
                    version: target,
                    reason: e.to_string(),
                })?;
            conn.pragma_update(None, "user_version", target)
                .map_err(KnowledgeError::Sqlite)?;
        }
    }

    let check: String = conn
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .map_err(KnowledgeError::Sqlite)?;
    if check != "ok" {
        return Err(KnowledgeError::Integrity(check));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_init_db_sets_wal_and_version() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("knowledge.db");
        let conn = rusqlite::Connection::open(&db).unwrap();
        init_db(&conn).unwrap();
        let mode: String = conn
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode.to_lowercase(), "wal");
        let v: u32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, CURRENT_SCHEMA);
    }

    #[test]
    fn test_init_db_is_idempotent() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        // Second call must not error (IF NOT EXISTS guards + same version).
        init_db(&conn).unwrap();
    }

    #[test]
    fn test_all_tables_created() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        for t in ["documents", "chunks", "chunks_fts"] {
            let exists: bool = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type IN ('table','fts5') AND name = ?1",
                    [t],
                    |r| r.get(0),
                )
                .unwrap();
            assert!(exists, "table {t} should exist");
        }
    }

    #[test]
    fn test_schema_upgrade_from_v0_to_v1() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("knowledge.db");
        {
            let conn = rusqlite::Connection::open(&db).unwrap();
            conn.pragma_update(None, "user_version", 0).unwrap();
        }
        init_db(&rusqlite::Connection::open(&db).unwrap()).unwrap();
        let conn = rusqlite::Connection::open(&db).unwrap();
        let v: u32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, CURRENT_SCHEMA);
        let exists: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='documents'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(exists);
    }
}
