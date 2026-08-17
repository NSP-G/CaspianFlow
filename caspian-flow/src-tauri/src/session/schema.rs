//! SQLite schema + migration for the P21 session store.
//!
//! Schema version is tracked via `PRAGMA user_version` (idiomatic SQLite),
//! independent of `config::settings::CURRENT_SCHEMA_VERSION` (which governs
//! settings.yaml, not this database).
//!
//! `MIGRATIONS[i]` upgrades the DB from version `i` to `i+1`. `init_db` is
//! idempotent: safe to call on every open.

use crate::session::error::{SessionError, SessionResult};

/// Target SQLite schema version.
pub const CURRENT_SCHEMA: u32 = 1;

/// SQL that brings the DB up to `CURRENT_SCHEMA`, applied incrementally.
pub static MIGRATIONS: &[&str] = &[
    // v0 -> v1: initial four-table schema + FTS5 virtual table.
    r#"
    CREATE TABLE IF NOT EXISTS sessions (
        id          TEXT PRIMARY KEY,
        agent_id    TEXT NOT NULL,
        user_id     TEXT DEFAULT 'default',
        title       TEXT,
        created_at  INTEGER NOT NULL,
        updated_at  INTEGER NOT NULL,
        metadata    TEXT,
        status      TEXT DEFAULT 'active'
    );
    CREATE INDEX IF NOT EXISTS idx_sessions_agent   ON sessions(agent_id);
    CREATE INDEX IF NOT EXISTS idx_sessions_updated ON sessions(updated_at DESC);

    CREATE TABLE IF NOT EXISTS messages (
        id           INTEGER PRIMARY KEY AUTOINCREMENT,
        session_id   TEXT NOT NULL,
        role         TEXT NOT NULL,
        content      TEXT NOT NULL,
        tool_calls   TEXT,
        tool_call_id TEXT,
        created_at   INTEGER NOT NULL,
        token_count  INTEGER DEFAULT 0,
        FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
    );
    CREATE INDEX IF NOT EXISTS idx_messages_session ON messages(session_id, created_at);

    CREATE TABLE IF NOT EXISTS workflow_runs (
        id              TEXT PRIMARY KEY,
        session_id      TEXT NOT NULL,
        workflow_name   TEXT NOT NULL,
        workflow_version TEXT,
        input           TEXT,
        output          TEXT,
        status          TEXT NOT NULL,
        started_at      INTEGER NOT NULL,
        finished_at     INTEGER,
        error           TEXT,
        cache_hit       INTEGER DEFAULT 0,
        FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
    );
    CREATE INDEX IF NOT EXISTS idx_workflow_runs_session ON workflow_runs(session_id);
    CREATE INDEX IF NOT EXISTS idx_workflow_runs_status  ON workflow_runs(status);

    CREATE TABLE IF NOT EXISTS agent_calls (
        id                 TEXT PRIMARY KEY,
        session_id         TEXT NOT NULL,
        agent_id           TEXT NOT NULL,
        user_input         TEXT NOT NULL,
        assistant_response TEXT,
        skills_used        TEXT,
        matched_skill      TEXT,
        confidence         REAL,
        latency_ms         INTEGER,
        created_at         INTEGER NOT NULL,
        FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
    );
    CREATE INDEX IF NOT EXISTS idx_agent_calls_session ON agent_calls(session_id);

    CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(content);
    "#,
];

/// Idempotently initialize the DB:
/// 1. enable WAL + foreign keys,
/// 2. run pending migrations up to `CURRENT_SCHEMA`,
/// 3. run `PRAGMA integrity_check`.
pub fn init_db(conn: &rusqlite::Connection) -> SessionResult<()> {
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(SessionError::Sqlite)?;
    conn.pragma_update(None, "foreign_keys", "ON")
        .map_err(SessionError::Sqlite)?;

    let version: u32 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(SessionError::Sqlite)?;

    for (i, migration) in MIGRATIONS.iter().enumerate() {
        let target = (i + 1) as u32;
        if version < target {
            conn.execute_batch(migration)
                .map_err(|e| SessionError::Migration {
                    version: target,
                    reason: e.to_string(),
                })?;
            conn.pragma_update(None, "user_version", target)
                .map_err(SessionError::Sqlite)?;
        }
    }

    let check: String = conn
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .map_err(SessionError::Sqlite)?;
    if check != "ok" {
        return Err(SessionError::Integrity(check));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_init_db_sets_wal_and_version() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("s.db");
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
        for t in [
            "sessions",
            "messages",
            "workflow_runs",
            "agent_calls",
            "messages_fts",
        ] {
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
}
