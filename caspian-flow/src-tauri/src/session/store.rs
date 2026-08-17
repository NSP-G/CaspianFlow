//! SQLite-backed implementation of the `SessionStore` trait (P21).
//!
//! Concurrency model: a single `Arc<parking_lot::Mutex<Connection>>` serializes
//! all access. This is the design's "single-writer" decision — reads in WAL
//! would be non-blocking with separate connections, but the design explicitly
//! chose serialization, and it is well within the <2ms write / <10ms read budget
//! for these small ops.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{Datelike, Duration, Local};
use parking_lot::Mutex;
use rusqlite::{params, Connection};
use uuid::Uuid;

use crate::config::CaspianPaths;
use crate::session::error::{SessionError, SessionResult};
use crate::session::schema::init_db;
use crate::session::types::*;

/// Current unix timestamp in seconds.
fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Generate a UUID v4 string.
fn new_uuid() -> String {
    Uuid::new_v4().to_string()
}

/// Serialize a JSON value to its textual form (or SQL NULL when `None`).
fn json_to_text(v: &Option<serde_json::Value>) -> Option<String> {
    v.as_ref().map(|v| v.to_string())
}

/// Parse a stored JSON text back into a value (None when the column is NULL).
fn text_to_json(s: Option<&str>) -> Option<serde_json::Value> {
    match s {
        Some(s) => serde_json::from_str(s).ok(),
        None => None,
    }
}

/// Serialize an enum to its snake_case string (for `status` / `role` columns).
fn enum_to_str<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|j| j.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// The session-store contract. The SQLite backend is the default implementation,
/// but the trait keeps the backend swappable (design principle #3).
pub trait SessionStore: Send + Sync {
    // ---- session CRUD ----
    fn create_session(&self, agent_id: &str, title: Option<&str>) -> SessionResult<Session>;
    fn get_session(&self, id: &str) -> SessionResult<Option<Session>>;
    fn list_sessions(&self, agent_id: Option<&str>, limit: usize) -> SessionResult<Vec<Session>>;
    fn update_session_title(&self, id: &str, title: &str) -> SessionResult<()>;
    /// Soft delete: flips `status` to `deleted`. Does NOT trigger FK cascade.
    fn delete_session(&self, id: &str) -> SessionResult<()>;
    fn archive_session(&self, id: &str) -> SessionResult<()>;

    // ---- message CRUD ----
    fn append_message(&self, session_id: &str, msg: Message) -> SessionResult<u64>;
    fn get_messages(
        &self,
        session_id: &str,
        limit: usize,
        before: Option<i64>,
    ) -> SessionResult<Vec<Message>>;

    // ---- workflow run records ----
    fn record_workflow_run(&self, run: WorkflowRunRecord) -> SessionResult<()>;
    fn list_workflow_runs(&self, session_id: &str) -> SessionResult<Vec<WorkflowRunRecord>>;

    // ---- agent call records ----
    fn record_agent_call(&self, call: AgentCallRecord) -> SessionResult<()>;
    fn list_agent_calls(&self, session_id: &str) -> SessionResult<Vec<AgentCallRecord>>;

    // ---- P21 extensions (gap resolutions) ----
    /// Full-text search messages within a session (FTS5 — design 7.3).
    fn search_messages(
        &self,
        session_id: &str,
        query: &str,
        limit: usize,
    ) -> SessionResult<Vec<Message>>;
    /// Hard delete: physically removes the session, triggering `ON DELETE CASCADE`
    /// for messages / workflow_runs / agent_calls. Used by acceptance #6.
    fn purge_session(&self, id: &str) -> SessionResult<()>;
    /// `VACUUM INTO` backup with 7-day rotation. Returns the backup path.
    fn backup_with_rotation(&self) -> SessionResult<PathBuf>;
    /// Run `PRAGMA integrity_check`; `Ok(())` iff the result is `"ok"`.
    fn integrity_check(&self) -> SessionResult<()>;
}

/// SQLite implementation of [`SessionStore`].
pub struct SqliteSessionStore {
    conn: Arc<Mutex<Connection>>,
    db_path: PathBuf,
    backups_dir: PathBuf,
}

impl SqliteSessionStore {
    /// Open (creating + initializing if needed) the session DB at `db_path`.
    pub fn open(db_path: &Path) -> SessionResult<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(db_path)?;
        init_db(&conn)?;
        let backups_dir = db_path
            .parent()
            .map(|p| p.join("backups"))
            .unwrap_or_else(|| PathBuf::from("backups"));
        std::fs::create_dir_all(&backups_dir)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            db_path: db_path.to_path_buf(),
            backups_dir,
        })
    }

    /// Open from CaspianFlow paths (`~/.caspian/sessions/sessions.db`).
    pub fn from_paths(paths: &CaspianPaths) -> SessionResult<Self> {
        Self::open(&paths.sessions.join("sessions.db"))
    }

    /// The on-disk database path.
    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    /// The backup directory.
    pub fn backups_dir(&self) -> &Path {
        &self.backups_dir
    }

    /// Drop old backups beyond the 7-day retention window.
    fn rotate_backups(&self) -> SessionResult<()> {
        let cutoff = (Local::now() - Duration::days(7)).date_naive();
        for entry in std::fs::read_dir(&self.backups_dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            if let Some(date_str) = name
                .strip_prefix("sessions_")
                .and_then(|s| s.strip_suffix(".db"))
            {
                if let Ok(d) = chrono::NaiveDate::parse_from_str(date_str, "%Y%m%d") {
                    if d < cutoff {
                        let _ = std::fs::remove_file(entry.path());
                    }
                }
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Row mappers
// ---------------------------------------------------------------------------

fn row_to_session(row: &rusqlite::Row) -> rusqlite::Result<Session> {
    let status = match row.get::<_, String>(7)?.as_str() {
        "archived" => SessionStatus::Archived,
        "deleted" => SessionStatus::Deleted,
        _ => SessionStatus::Active,
    };
    Ok(Session {
        id: row.get(0)?,
        agent_id: row.get(1)?,
        user_id: row.get(2)?,
        title: row.get(3)?,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
        metadata: text_to_json(row.get::<_, Option<String>>(6)?.as_deref())
            .unwrap_or(serde_json::Value::Null),
        status,
    })
}

fn row_to_message(row: &rusqlite::Row) -> rusqlite::Result<Message> {
    let role = match row.get::<_, String>(2)?.as_str() {
        "assistant" => MessageRole::Assistant,
        "system" => MessageRole::System,
        "tool" => MessageRole::Tool,
        _ => MessageRole::User,
    };
    Ok(Message {
        id: row.get(0)?,
        session_id: row.get(1)?,
        role,
        content: row.get(3)?,
        tool_calls: text_to_json(row.get::<_, Option<String>>(4)?.as_deref()),
        tool_call_id: row.get(5)?,
        created_at: row.get(6)?,
        token_count: row.get(7)?,
    })
}

fn row_to_workflow_run(row: &rusqlite::Row) -> rusqlite::Result<WorkflowRunRecord> {
    let status = match row.get::<_, String>(6)?.as_str() {
        "running" => WorkflowRunStatus::Running,
        "completed" => WorkflowRunStatus::Completed,
        "failed" => WorkflowRunStatus::Failed,
        "terminated" => WorkflowRunStatus::Terminated,
        _ => WorkflowRunStatus::Pending,
    };
    Ok(WorkflowRunRecord {
        id: row.get(0)?,
        session_id: row.get(1)?,
        workflow_name: row.get(2)?,
        workflow_version: row.get(3)?,
        input: text_to_json(row.get::<_, Option<String>>(4)?.as_deref()),
        output: text_to_json(row.get::<_, Option<String>>(5)?.as_deref()),
        status,
        started_at: row.get(7)?,
        finished_at: row.get(8)?,
        error: row.get(9)?,
        cache_hit: row.get::<_, i64>(10)? != 0,
    })
}

fn row_to_agent_call(row: &rusqlite::Row) -> rusqlite::Result<AgentCallRecord> {
    let skills_used: Vec<String> = text_to_json(row.get::<_, Option<String>>(5)?.as_deref())
        .and_then(|v| v.as_array().map(|a| a.to_vec()))
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    Ok(AgentCallRecord {
        id: row.get(0)?,
        session_id: row.get(1)?,
        agent_id: row.get(2)?,
        user_input: row.get(3)?,
        assistant_response: row.get(4)?,
        skills_used,
        matched_skill: row.get(6)?,
        confidence: row.get(7)?,
        latency_ms: row.get(8)?,
        created_at: row.get(9)?,
    })
}

// ---------------------------------------------------------------------------
// Trait implementation
// ---------------------------------------------------------------------------

impl SessionStore for SqliteSessionStore {
    fn create_session(&self, agent_id: &str, title: Option<&str>) -> SessionResult<Session> {
        let id = new_uuid();
        let now = now_secs();
        let session = Session {
            id: id.clone(),
            agent_id: agent_id.to_string(),
            user_id: "default".to_string(),
            title: title.map(str::to_string),
            created_at: now,
            updated_at: now,
            metadata: serde_json::Value::Object(Default::default()),
            status: SessionStatus::Active,
        };
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO sessions (id, agent_id, user_id, title, created_at, updated_at, metadata, status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                session.id,
                session.agent_id,
                session.user_id,
                session.title,
                session.created_at,
                session.updated_at,
                session.metadata.to_string(),
                enum_to_str(&session.status),
            ],
        )?;
        Ok(session)
    }

    fn get_session(&self, id: &str) -> SessionResult<Option<Session>> {
        let conn = self.conn.lock();
        let res = conn.query_row(
            "SELECT id, agent_id, user_id, title, created_at, updated_at, metadata, status
             FROM sessions WHERE id = ?1",
            params![id],
            row_to_session,
        );
        match res {
            Ok(s) => Ok(Some(s)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(SessionError::Sqlite(e)),
        }
    }

    fn list_sessions(&self, agent_id: Option<&str>, limit: usize) -> SessionResult<Vec<Session>> {
        let conn = self.conn.lock();
        let mut out = Vec::new();
        match agent_id {
            Some(a) => {
                let mut stmt = conn.prepare(
                    "SELECT id, agent_id, user_id, title, created_at, updated_at, metadata, status \
                     FROM sessions WHERE agent_id = ?1 ORDER BY updated_at DESC LIMIT ?2",
                )?;
                let rows = stmt.query_map(params![a, limit as i64], row_to_session)?;
                for r in rows {
                    out.push(r?);
                }
            }
            None => {
                let mut stmt = conn.prepare(
                    "SELECT id, agent_id, user_id, title, created_at, updated_at, metadata, status \
                     FROM sessions ORDER BY updated_at DESC LIMIT ?1",
                )?;
                let rows = stmt.query_map(params![limit as i64], row_to_session)?;
                for r in rows {
                    out.push(r?);
                }
            }
        }
        Ok(out)
    }

    fn update_session_title(&self, id: &str, title: &str) -> SessionResult<()> {
        let conn = self.conn.lock();
        let n = conn.execute(
            "UPDATE sessions SET title = ?1, updated_at = ?2 WHERE id = ?3",
            params![title, now_secs(), id],
        )?;
        if n == 0 {
            return Err(SessionError::NotFound(id.to_string()));
        }
        Ok(())
    }

    fn delete_session(&self, id: &str) -> SessionResult<()> {
        let conn = self.conn.lock();
        let n = conn.execute(
            "UPDATE sessions SET status = 'deleted', updated_at = ?1 WHERE id = ?2",
            params![now_secs(), id],
        )?;
        if n == 0 {
            return Err(SessionError::NotFound(id.to_string()));
        }
        Ok(())
    }

    fn archive_session(&self, id: &str) -> SessionResult<()> {
        let conn = self.conn.lock();
        let n = conn.execute(
            "UPDATE sessions SET status = 'archived', updated_at = ?1 WHERE id = ?2",
            params![now_secs(), id],
        )?;
        if n == 0 {
            return Err(SessionError::NotFound(id.to_string()));
        }
        Ok(())
    }

    fn append_message(&self, session_id: &str, msg: Message) -> SessionResult<u64> {
        let created_at = if msg.created_at > 0 {
            msg.created_at
        } else {
            now_secs()
        };
        let role = enum_to_str(&msg.role);
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO messages (session_id, role, content, tool_calls, tool_call_id, created_at, token_count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                session_id,
                role,
                msg.content,
                json_to_text(&msg.tool_calls),
                msg.tool_call_id,
                created_at,
                msg.token_count,
            ],
        )?;
        let rowid = tx.last_insert_rowid();
        // Keep the FTS index in sync (design 7.3).
        tx.execute(
            "INSERT INTO messages_fts (content, rowid) VALUES (?1, ?2)",
            params![msg.content, rowid],
        )?;
        tx.commit()?;
        Ok(rowid as u64)
    }

    fn get_messages(
        &self,
        session_id: &str,
        limit: usize,
        before: Option<i64>,
    ) -> SessionResult<Vec<Message>> {
        let conn = self.conn.lock();
        let mut out = Vec::new();
        match before {
            Some(b) => {
                let mut stmt = conn.prepare(
                    "SELECT id, session_id, role, content, tool_calls, tool_call_id, created_at, token_count \
                     FROM messages WHERE session_id = ?1 AND id < ?2 ORDER BY id ASC LIMIT ?3",
                )?;
                let rows = stmt.query_map(params![session_id, b, limit as i64], row_to_message)?;
                for r in rows {
                    out.push(r?);
                }
            }
            None => {
                let mut stmt = conn.prepare(
                    "SELECT id, session_id, role, content, tool_calls, tool_call_id, created_at, token_count \
                     FROM messages WHERE session_id = ?1 ORDER BY id ASC LIMIT ?2",
                )?;
                let rows = stmt.query_map(params![session_id, limit as i64], row_to_message)?;
                for r in rows {
                    out.push(r?);
                }
            }
        }
        Ok(out)
    }

    fn record_workflow_run(&self, run: WorkflowRunRecord) -> SessionResult<()> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT OR REPLACE INTO workflow_runs \
             (id, session_id, workflow_name, workflow_version, input, output, status, started_at, finished_at, error, cache_hit) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            params![
                run.id,
                run.session_id,
                run.workflow_name,
                run.workflow_version,
                json_to_text(&run.input),
                json_to_text(&run.output),
                enum_to_str(&run.status),
                run.started_at,
                run.finished_at,
                run.error,
                run.cache_hit as i64,
            ],
        )?;
        Ok(())
    }

    fn list_workflow_runs(&self, session_id: &str) -> SessionResult<Vec<WorkflowRunRecord>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, session_id, workflow_name, workflow_version, input, output, status, started_at, finished_at, error, cache_hit \
             FROM workflow_runs WHERE session_id = ?1 ORDER BY started_at DESC",
        )?;
        let rows = stmt.query_map(params![session_id], row_to_workflow_run)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    fn record_agent_call(&self, call: AgentCallRecord) -> SessionResult<()> {
        let skills_json =
            serde_json::to_string(&call.skills_used).unwrap_or_else(|_| "[]".to_string());
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO agent_calls \
             (id, session_id, agent_id, user_input, assistant_response, skills_used, matched_skill, confidence, latency_ms, created_at) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![
                call.id,
                call.session_id,
                call.agent_id,
                call.user_input,
                call.assistant_response,
                skills_json,
                call.matched_skill,
                call.confidence,
                call.latency_ms,
                call.created_at,
            ],
        )?;
        Ok(())
    }

    fn list_agent_calls(&self, session_id: &str) -> SessionResult<Vec<AgentCallRecord>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, session_id, agent_id, user_input, assistant_response, skills_used, matched_skill, confidence, latency_ms, created_at \
             FROM agent_calls WHERE session_id = ?1 ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map(params![session_id], row_to_agent_call)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    fn search_messages(
        &self,
        session_id: &str,
        query: &str,
        limit: usize,
    ) -> SessionResult<Vec<Message>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT m.id, m.session_id, m.role, m.content, m.tool_calls, m.tool_call_id, m.created_at, m.token_count \
             FROM messages_fts JOIN messages m ON m.id = messages_fts.rowid \
             WHERE messages_fts MATCH ?1 AND m.session_id = ?2 \
             ORDER BY messages_fts.rank LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![query, session_id, limit as i64], row_to_message)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    fn purge_session(&self, id: &str) -> SessionResult<()> {
        let conn = self.conn.lock();
        let n = conn.execute("DELETE FROM sessions WHERE id = ?1", params![id])?;
        if n == 0 {
            return Err(SessionError::NotFound(id.to_string()));
        }
        Ok(())
    }

    fn backup_with_rotation(&self) -> SessionResult<PathBuf> {
        // 1. Checkpoint WAL so the backup captures all committed data.
        {
            let conn = self.conn.lock();
            conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
        }
        let now = Local::now();
        let fname = format!(
            "sessions_{:04}{:02}{:02}.db",
            now.year(),
            now.month(),
            now.day()
        );
        let tmp = self.backups_dir.join(format!(".{fname}.tmp"));
        let dest = self.backups_dir.join(&fname);

        // VACUUM INTO requires the destination file to not already exist.
        let _ = std::fs::remove_file(&tmp);
        {
            let conn = self.conn.lock();
            let sql = format!(
                "VACUUM INTO '{}'",
                tmp.to_string_lossy().replace('\'', "''")
            );
            conn.execute_batch(&sql)
                .map_err(|e| SessionError::Backup(e.to_string()))?;
        }
        std::fs::rename(&tmp, &dest)?;

        // 2. Rotation: drop backups older than 7 days.
        self.rotate_backups()?;
        Ok(dest)
    }

    fn integrity_check(&self) -> SessionResult<()> {
        let conn = self.conn.lock();
        let res: String = conn.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
        if res != "ok" {
            return Err(SessionError::Integrity(res));
        }
        Ok(())
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::schema::CURRENT_SCHEMA;
    use crate::session::types::{
        AgentCallRecord, Message, MessageRole, SessionStatus, WorkflowRunRecord, WorkflowRunStatus,
    };

    fn temp_store() -> (tempfile::TempDir, SqliteSessionStore) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("sessions.db");
        let store = SqliteSessionStore::open(&db).unwrap();
        (dir, store)
    }

    // Acceptance #1: open creates db + enables WAL.
    #[test]
    fn test_open_creates_db_and_enables_wal() {
        let (dir, store) = temp_store();
        let db = store.db_path();
        assert!(db.exists());
        let conn = Connection::open(db).unwrap();
        let mode: String = conn
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode.to_lowercase(), "wal");
        let _ = dir;
    }

    // Acceptance #2: create session, append 3 messages, ordered query.
    #[test]
    fn test_create_session_append_3_messages_order() {
        let (_dir, store) = temp_store();
        let s = store.create_session("agent_a", Some("Chat")).unwrap();
        for i in 0..3 {
            let m = Message {
                id: 0,
                session_id: s.id.clone(),
                role: MessageRole::User,
                content: format!("msg {i}"),
                tool_calls: None,
                tool_call_id: None,
                created_at: 0,
                token_count: 1,
            };
            store.append_message(&s.id, m).unwrap();
        }
        let msgs = store.get_messages(&s.id, 100, None).unwrap();
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].content, "msg 0");
        assert_eq!(msgs[1].content, "msg 1");
        assert_eq!(msgs[2].content, "msg 2");
    }

    // Acceptance #3: two agents, filter by agent.
    #[test]
    fn test_list_sessions_by_agent() {
        let (_dir, store) = temp_store();
        let a = store.create_session("agent_a", None).unwrap();
        let _b = store.create_session("agent_b", None).unwrap();
        let _c = store.create_session("agent_a", None).unwrap();
        let a_sessions = store.list_sessions(Some("agent_a"), 100).unwrap();
        assert_eq!(a_sessions.len(), 2);
        assert!(a_sessions.iter().all(|s| s.agent_id == "agent_a"));
        let _ = a;
    }

    // Acceptance #4: record a workflow run.
    #[test]
    fn test_record_workflow_run() {
        let (_dir, store) = temp_store();
        let s = store.create_session("agent_a", None).unwrap();
        let run = WorkflowRunRecord {
            id: new_uuid(),
            session_id: s.id.clone(),
            workflow_name: "wf_demo".into(),
            workflow_version: Some("1.0".into()),
            input: Some(serde_json::json!({"x": 1})),
            output: Some(serde_json::json!({"y": 2})),
            status: WorkflowRunStatus::Completed,
            started_at: now_secs(),
            finished_at: Some(now_secs()),
            error: None,
            cache_hit: true,
        };
        store.record_workflow_run(run).unwrap();
        let runs = store.list_workflow_runs(&s.id).unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].workflow_name, "wf_demo");
        assert!(runs[0].cache_hit);
    }

    // Acceptance #5: record an agent call.
    #[test]
    fn test_record_agent_call() {
        let (_dir, store) = temp_store();
        let s = store.create_session("agent_a", None).unwrap();
        let call = AgentCallRecord {
            id: new_uuid(),
            session_id: s.id.clone(),
            agent_id: "agent_a".into(),
            user_input: "hello".into(),
            assistant_response: Some("hi".into()),
            skills_used: vec!["skill_a".into(), "skill_b".into()],
            matched_skill: Some("skill_a".into()),
            confidence: Some(0.92),
            latency_ms: Some(150),
            created_at: now_secs(),
        };
        store.record_agent_call(call).unwrap();
        let calls = store.list_agent_calls(&s.id).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].skills_used, vec!["skill_a", "skill_b"]);
        assert_eq!(calls[0].confidence, Some(0.92));
        assert_eq!(calls[0].latency_ms, Some(150));
    }

    // Acceptance #6: purge cascades to messages / runs / calls.
    #[test]
    fn test_purge_session_cascades() {
        let (_dir, store) = temp_store();
        let s = store.create_session("agent_a", None).unwrap();
        let m = Message {
            id: 0,
            session_id: s.id.clone(),
            role: MessageRole::User,
            content: "m".into(),
            tool_calls: None,
            tool_call_id: None,
            created_at: 0,
            token_count: 1,
        };
        store.append_message(&s.id, m).unwrap();
        let run = WorkflowRunRecord {
            id: new_uuid(),
            session_id: s.id.clone(),
            workflow_name: "wf".into(),
            workflow_version: None,
            input: None,
            output: None,
            status: WorkflowRunStatus::Completed,
            started_at: now_secs(),
            finished_at: Some(now_secs()),
            error: None,
            cache_hit: false,
        };
        store.record_workflow_run(run).unwrap();

        store.purge_session(&s.id).unwrap();

        assert!(store.get_session(&s.id).unwrap().is_none());
        assert!(store.get_messages(&s.id, 100, None).unwrap().is_empty());
        assert!(store.list_workflow_runs(&s.id).unwrap().is_empty());
    }

    // Acceptance #6 (soft-delete variant): delete_session keeps data, flips status.
    #[test]
    fn test_delete_session_soft() {
        let (_dir, store) = temp_store();
        let s = store.create_session("agent_a", None).unwrap();
        let m = Message {
            id: 0,
            session_id: s.id.clone(),
            role: MessageRole::User,
            content: "m".into(),
            tool_calls: None,
            tool_call_id: None,
            created_at: 0,
            token_count: 1,
        };
        store.append_message(&s.id, m).unwrap();
        store.delete_session(&s.id).unwrap();
        // Soft delete: session still present, status flipped, messages retained.
        let reloaded = store.get_session(&s.id).unwrap().unwrap();
        assert_eq!(reloaded.status, SessionStatus::Deleted);
        assert_eq!(store.get_messages(&s.id, 100, None).unwrap().len(), 1);
    }

    // Acceptance #7: 10 threads writing different sessions — no loss, no panic.
    #[test]
    fn test_concurrent_writes_no_loss() {
        let (_dir, store) = temp_store();
        let store = Arc::new(store);
        let n = 10usize;
        let handles: Vec<_> = (0..n)
            .map(|i| {
                let store = Arc::clone(&store);
                std::thread::spawn(move || {
                    let s = store.create_session(&format!("agent_{i}"), None).unwrap();
                    for j in 0..5 {
                        let m = Message {
                            id: 0,
                            session_id: s.id.clone(),
                            role: MessageRole::User,
                            content: format!("m{j}"),
                            tool_calls: None,
                            tool_call_id: None,
                            created_at: 0,
                            token_count: 1,
                        };
                        store.append_message(&s.id, m).unwrap();
                    }
                    s.id
                })
            })
            .collect();
        let ids: Vec<String> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(ids.len(), n);
        let all = store.list_sessions(None, 1000).unwrap();
        assert_eq!(all.len(), n);
        for id in &ids {
            assert_eq!(store.get_messages(id, 100, None).unwrap().len(), 5);
        }
    }

    // Acceptance #8: manual backup consistent with main db.
    #[test]
    fn test_backup_with_rotation_consistent() {
        let (_dir, store) = temp_store();
        let s = store.create_session("agent_a", None).unwrap();
        for i in 0..3 {
            let m = Message {
                id: 0,
                session_id: s.id.clone(),
                role: MessageRole::User,
                content: format!("msg {i}"),
                tool_calls: None,
                tool_call_id: None,
                created_at: 0,
                token_count: 1,
            };
            store.append_message(&s.id, m).unwrap();
        }
        let backup = store.backup_with_rotation().unwrap();
        assert!(backup.exists());

        // Open the backup and confirm it carries the same data.
        let backup_conn = Connection::open(&backup).unwrap();
        let count: i64 = backup_conn
            .query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 3);
        let session_count: i64 = backup_conn
            .query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(session_count, 1);
    }

    // 7.1: schema upgrade path — a v0 db is migrated to CURRENT_SCHEMA on open.
    #[test]
    fn test_schema_upgrade_from_v0_to_v1() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("sessions.db");
        // Hand-craft a v0 db (no tables, user_version = 0) then open via the store.
        {
            let conn = Connection::open(&db).unwrap();
            conn.pragma_update(None, "user_version", 0).unwrap();
        }
        let store = SqliteSessionStore::open(&db).unwrap();
        let conn = Connection::open(&db).unwrap();
        let v: u32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, CURRENT_SCHEMA);
        // Tables now exist.
        let exists: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='sessions'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(exists);
        let _ = store;
    }

    // 7.3 (FTS5): full-text search returns matching messages.
    #[test]
    fn test_search_messages_fts() {
        let (_dir, store) = temp_store();
        let s = store.create_session("agent_a", None).unwrap();
        for content in [
            "the quick brown fox",
            "lazy dog sleeps",
            "quick silver coin",
        ] {
            let m = Message {
                id: 0,
                session_id: s.id.clone(),
                role: MessageRole::User,
                content: content.into(),
                tool_calls: None,
                tool_call_id: None,
                created_at: 0,
                token_count: 1,
            };
            store.append_message(&s.id, m).unwrap();
        }
        let hits = store.search_messages(&s.id, "quick", 100).unwrap();
        assert_eq!(hits.len(), 2, "two messages contain 'quick'");
        let none = store
            .search_messages(&s.id, "nonexistentterm", 100)
            .unwrap();
        assert!(none.is_empty());
    }

    // Edge: get_session on unknown id returns None (not error).
    #[test]
    fn test_get_session_missing() {
        let (_dir, store) = temp_store();
        assert!(store.get_session("does_not_exist").unwrap().is_none());
    }

    // Edge: update/delete/purge on missing id errors with NotFound.
    #[test]
    fn test_not_found_errors() {
        let (_dir, store) = temp_store();
        assert!(matches!(
            store.update_session_title("nope", "t"),
            Err(SessionError::NotFound(_))
        ));
        assert!(matches!(
            store.delete_session("nope"),
            Err(SessionError::NotFound(_))
        ));
        assert!(matches!(
            store.purge_session("nope"),
            Err(SessionError::NotFound(_))
        ));
    }

    // Edge: before-pagination returns only older messages.
    #[test]
    fn test_get_messages_before_pagination() {
        let (_dir, store) = temp_store();
        let s = store.create_session("agent_a", None).unwrap();
        for i in 0..5 {
            let m = Message {
                id: 0,
                session_id: s.id.clone(),
                role: MessageRole::User,
                content: format!("m{i}"),
                tool_calls: None,
                tool_call_id: None,
                created_at: 0,
                token_count: 1,
            };
            store.append_message(&s.id, m).unwrap();
        }
        let all = store.get_messages(&s.id, 100, None).unwrap();
        assert_eq!(all.len(), 5);
        let first_id = all[0].id;
        let last_id = all[4].id;
        let before = store.get_messages(&s.id, 100, Some(first_id)).unwrap();
        assert!(before.is_empty(), "nothing older than the first message");
        // `before` returns rows strictly older than the cursor id.
        let older_than_last = store.get_messages(&s.id, 100, Some(last_id)).unwrap();
        assert_eq!(
            older_than_last.len(),
            4,
            "paging before the newest message returns all but the newest"
        );
    }
}
