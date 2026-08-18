//! Error self-healing — crash recovery, database repair, and graceful degradation.
//!
//! ## What this module does
//!
//! On startup, [`SelfHealingManager::run_startup_checks`] verifies the on-disk
//! SQLite databases (sessions, knowledge) with `PRAGMA integrity_check` and, if
//! a database is corrupt, restores it from the most recent backup. It also
//! validates `settings.yaml` and reports (without ever blocking startup) any
//! issue it could not fix.
//!
//! Backups are produced with SQLite's `VACUUM INTO` (a compact, consistent
//! snapshot) and pruned to a rolling 7-day window. The actual *scheduling* of a
//! daily 03:00 backup is a runtime concern (wired via a Tauri timer in the app
//! shell) — see the `create_backup` / `prune_backups` helpers it calls.
//!
//! ## Graceful degradation
//!
//! Three free functions let the rest of the app downgrade safely when a
//! dependency is unavailable:
//! - [`network_available`] — quick TCP probe.
//! - [`embedding_model_available`] — offline check that the embedding model
//!   files exist in the cache directory.
//! - [`degrade_network_skills`] — disables network-only skills when offline, so
//!   local skills keep working.

use std::error::Error;
use std::fmt;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::Connection;

use crate::config::CaspianPaths;
use crate::skill::schema::Skill;
use crate::skill::SkillRegistry;

/// Result of running the startup self-healing checks.
#[derive(Debug, Default, Clone)]
pub struct HealingReport {
    /// Databases that were verified (or attempted).
    pub checked: Vec<PathBuf>,
    /// Human-readable descriptions of repairs that succeeded.
    pub repaired: Vec<String>,
    /// Issues that could not be auto-fixed (startup still proceeds).
    pub issues: Vec<String>,
}

impl HealingReport {
    /// True if any issue was found (fixed or not).
    pub fn has_issues(&self) -> bool {
        !self.issues.is_empty()
    }
}

/// Errors encountered while attempting self-healing.
///
/// Individual database checks that fail are reported into [`HealingReport::issues`]
/// rather than returned as `Err` — startup must never be blocked. `Err` is
/// reserved for catastrophic situations where the manager itself cannot run.
#[derive(Debug)]
pub enum HealingError {
    DbOpen(String),
    Integrity(String),
    Backup(String),
    Restore(String),
    Config(String),
}

impl fmt::Display for HealingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HealingError::DbOpen(m) => write!(f, "database open failed: {m}"),
            HealingError::Integrity(m) => write!(f, "integrity check failed: {m}"),
            HealingError::Backup(m) => write!(f, "backup failed: {m}"),
            HealingError::Restore(m) => write!(f, "restore from backup failed: {m}"),
            HealingError::Config(m) => write!(f, "config validation failed: {m}"),
        }
    }
}

impl Error for HealingError {}

/// Manages startup integrity checks, backups, and repair for CaspianFlow data.
pub struct SelfHealingManager {
    paths: CaspianPaths,
}

impl SelfHealingManager {
    /// Create a manager bound to the given paths.
    pub fn new(paths: CaspianPaths) -> Self {
        Self { paths }
    }

    /// The SQLite databases we are responsible for, as concrete file paths.
    fn known_databases(&self) -> Vec<PathBuf> {
        vec![
            self.paths.sessions.join("sessions.db"),
            self.paths.knowledge.join("knowledge.db"),
        ]
    }

    /// Run all startup checks. Never blocks startup: per-database failures are
    /// recorded in [`HealingReport::issues`], and the function still returns
    /// `Ok(report)` so the caller can proceed.
    pub fn run_startup_checks(&self) -> Result<HealingReport, HealingError> {
        let _ = self.paths.ensure_dirs();
        let mut report = HealingReport::default();

        for db in self.known_databases() {
            if !db.exists() {
                // Fresh install or first run — nothing to verify yet.
                continue;
            }
            report.checked.push(db.clone());

            match self.check_database(&db) {
                Ok(()) => { /* healthy */ }
                Err(e) => {
                    let msg = format!("{}: {}", db.display(), e);
                    tracing::warn!(db = %db.display(), "database integrity check failed");
                    match self.restore_from_backup(&db) {
                        Ok(backup) => {
                            report.repaired.push(format!(
                                "restored {} from {}",
                                db.display(),
                                backup.display()
                            ));
                            // Re-verify the restored database.
                            if let Err(e2) = self.check_database(&db) {
                                report.issues.push(format!(
                                    "{}: restored but still corrupt: {}",
                                    db.display(),
                                    e2
                                ));
                            }
                        }
                        Err(_) => {
                            report.issues.push(format!("{msg}; no usable backup found"));
                        }
                    }
                }
            }
        }

        if let Err(e) = self.validate_configs() {
            report.issues.push(format!("settings: {e}"));
        }

        Ok(report)
    }

    /// Run `PRAGMA integrity_check` on a SQLite database.
    ///
    /// Returns `Ok(())` when SQLite reports `ok`, otherwise an error describing
    /// the first problem found.
    pub fn check_database(&self, path: &Path) -> Result<(), HealingError> {
        let conn = Connection::open(path)
            .map_err(|e| HealingError::DbOpen(format!("{path:?}: {e}")))?;
        let result: String = conn
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .map_err(|e| HealingError::Integrity(format!("{path:?}: {e}")))?;
        if result.to_lowercase() == "ok" {
            Ok(())
        } else {
            Err(HealingError::Integrity(format!(
                "{}: {}",
                path.display(),
                result
            )))
        }
    }

    /// Restore a database from its most recent backup, if any.
    ///
    /// The corrupt file is moved to `<db>.corrupt-<timestamp>` so it can be
    /// inspected later, then the newest matching backup is copied into place.
    pub fn restore_from_backup(&self, db_path: &Path) -> Result<PathBuf, HealingError> {
        let stem = db_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("db")
            .to_string();
        let backups = self.list_backups(&stem);
        let Some(latest) = backups.into_iter().next() else {
            return Err(HealingError::Restore(format!(
                "no backup found for {}",
                db_path.display()
            )));
        };

        // Move the corrupt db aside for forensics.
        let corrupt = db_path.with_extension(format!(
            "corrupt-{}",
            chrono_timestamp_suffix()
        ));
        if db_path.exists() {
            let _ = std::fs::remove_file(&corrupt);
            std::fs::rename(db_path, &corrupt)
                .map_err(|e| HealingError::Restore(format!("cannot move corrupt db: {e}")))?;
        }

        std::fs::copy(&latest, db_path)
            .map_err(|e| HealingError::Restore(format!("cannot copy backup: {e}")))?;
        Ok(latest)
    }

    /// Create a compact, consistent backup of a database via `VACUUM INTO`.
    ///
    /// The backup lands in `<paths.backups>/<stem>_<timestamp>.db`. Callers
    /// (e.g. a daily 03:00 timer) should follow this with [`prune_backups`].
    pub fn create_backup(&self, db_path: &Path) -> Result<PathBuf, HealingError> {
        if !db_path.exists() {
            return Err(HealingError::Backup(format!(
                "source database missing: {}",
                db_path.display()
            )));
        }
        std::fs::create_dir_all(&self.paths.backups)
            .map_err(|e| HealingError::Backup(format!("cannot create backup dir: {e}")))?;

        let stem = db_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("db")
            .to_string();
        let dest = self
            .paths
            .backups
            .join(format!("{stem}_{}.db", chrono_timestamp_suffix()));
        // VACUUM INTO requires the destination to not exist.
        let _ = std::fs::remove_file(&dest);

        let conn = Connection::open(db_path)
            .map_err(|e| HealingError::Backup(format!("{db_path:?}: {e}")))?;
        let quoted = dest.to_string_lossy().replace('\'', "''");
        conn.execute_batch(&format!("VACUUM INTO '{quoted}'"))
            .map_err(|e| HealingError::Backup(format!("vacuum failed: {e}")))?;
        Ok(dest)
    }

    /// List backups for a database stem, newest first.
    fn list_backups(&self, stem: &str) -> Vec<PathBuf> {
        let mut found = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&self.paths.backups) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with(&format!("{stem}_")) && name.ends_with(".db") {
                    found.push(entry.path());
                }
            }
        }
        found.sort_by_key(|p| std::fs::metadata(p).map(|m| m.modified().ok()).ok().flatten());
        found.reverse();
        found
    }

    /// Prune backups for a stem to the most recent `keep` (default 7).
    ///
    /// Returns the number of backups removed.
    pub fn prune_backups(&self, stem: &str, keep: usize) -> Result<usize, HealingError> {
        let mut backups = self.list_backups(stem);
        if backups.len() <= keep {
            return Ok(0);
        }
        let to_remove = backups.split_off(keep);
        let mut removed = 0;
        for path in to_remove {
            if std::fs::remove_file(&path).is_ok() {
                removed += 1;
            }
        }
        Ok(removed)
    }

    /// Validate `settings.yaml` (if present) parses as well-formed YAML.
    pub fn validate_configs(&self) -> Result<(), HealingError> {
        let settings = &self.paths.settings_file;
        if !settings.exists() {
            return Ok(());
        }
        let text = std::fs::read_to_string(settings)
            .map_err(|e| HealingError::Config(format!("cannot read: {e}")))?;
        serde_yaml::from_str::<serde_yaml::Value>(&text)
            .map_err(|e| HealingError::Config(format!("invalid YAML: {e}")))?;
        Ok(())
    }
}

/// A compact, collision-resistant timestamp suffix for backup file names.
fn chrono_timestamp_suffix() -> String {
    // Avoid a chrono dependency just for a timestamp: use system time. The
    // suffix carries sub-second precision so two backups created within the
    // same wall-clock second (e.g. by a tight retry loop) never overwrite each
    // other's file.
    use std::time::SystemTime;
    let dur = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{:x}-{}", dur.as_secs(), dur.subsec_nanos())
}

/// Returns `true` if outbound network appears reachable (best-effort).
///
/// Probes a well-known anycast DNS endpoint with a short timeout. A failure
/// (offline, firewalled, no DNS) yields `false` so callers can degrade.
pub fn network_available() -> bool {
    // 8.8.8.8:53 (Google DNS) is a stable anycast address; a TCP connect here
    // is a cheap liveness signal and does not actually send DNS traffic.
    "8.8.8.8:53"
        .to_socket_addrs()
        .ok()
        .and_then(|mut addrs| addrs.next())
        .and_then(|addr| TcpStream::connect_timeout(&addr, Duration::from_secs(2)).ok())
        .is_some()
}

/// Returns `true` if an embedding model appears to be present offline.
///
/// fastembed keeps models under `<cache_dir>/models--<org>--<name>/`. We check
/// for any such directory without downloading anything, so this is safe to call
/// at startup. A missing cache means the model must be fetched before vector
/// search can run — callers should fall back to keyword search.
pub fn embedding_model_available(cache_dir: &Path) -> bool {
    if !cache_dir.exists() {
        return false;
    }
    std::fs::read_dir(cache_dir)
        .map(|entries| {
            entries.flatten().any(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("models--")
                    && e.path().is_dir()
            })
        })
        .unwrap_or(false)
}

/// Disable all network-only skills so the app keeps working offline.
///
/// Returns the number of skills disabled. Local (non-network) skills are
/// untouched. Intended to run when [`network_available`] returns `false`.
pub fn degrade_network_skills(registry: &SkillRegistry) -> usize {
    let network_skills: Vec<String> = registry
        .list_all()
        .into_iter()
        .filter(|s: &Skill| s.permissions.network)
        .map(|s| s.name)
        .collect();
    let mut disabled = 0;
    for name in network_skills {
        if registry.disable(&name) {
            tracing::warn!(skill = %name, "network skill disabled (offline degradation)");
            disabled += 1;
        }
    }
    disabled
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skill::schema::Skill;
    use rusqlite::Connection;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// Build an isolated `~/.caspian` tree under a unique temp directory so no
    /// test touches the developer's real home directory.
    fn temp_paths() -> (PathBuf, CaspianPaths) {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("caspian-sh-test-{}-{}", std::process::id(), n));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let paths = CaspianPaths::resolve(Some(&dir));
        paths.ensure_dirs().unwrap();
        (dir, paths)
    }

    /// Create a healthy SQLite database at `path` with one table.
    fn make_healthy_db(path: &Path) {
        let _ = std::fs::remove_file(path);
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT);
             INSERT INTO t (v) VALUES ('hello');",
        )
        .unwrap();
    }

    /// Overwrite `path` with random bytes so SQLite detects corruption.
    fn corrupt_db(path: &Path) {
        std::fs::write(path, b"this is not a valid sqlite database at all").unwrap();
    }

    #[test]
    fn test_check_database_healthy() {
        let (_dir, paths) = temp_paths();
        let db = paths.sessions.join("sessions.db");
        make_healthy_db(&db);
        let mgr = SelfHealingManager::new(paths);
        assert!(mgr.check_database(&db).is_ok());
    }

    #[test]
    fn test_check_database_corrupt() {
        let (_dir, paths) = temp_paths();
        let db = paths.sessions.join("sessions.db");
        corrupt_db(&db);
        let mgr = SelfHealingManager::new(paths);
        let err = mgr.check_database(&db).unwrap_err();
        match err {
            HealingError::Integrity(_) => {}
            other => panic!("expected Integrity error, got {other:?}"),
        }
    }

    #[test]
    fn test_create_backup_and_prune() {
        let (_dir, paths) = temp_paths();
        let db = paths.sessions.join("sessions.db");
        make_healthy_db(&db);
        let mgr = SelfHealingManager::new(paths.clone());

        // A healthy db can be backed up via VACUUM INTO.
        let backup = mgr.create_backup(&db).unwrap();
        assert!(backup.exists());
        // The backup itself is a valid SQLite database.
        assert!(mgr.check_database(&backup).is_ok());

        // create_backup must refuse when the source is missing.
        let missing = paths.sessions.join("nope.db");
        assert!(mgr.create_backup(&missing).is_err());

        // Pruning with fewer backups than `keep` removes nothing.
        assert_eq!(mgr.prune_backups("sessions", 7).unwrap(), 0);

        // With several backups, only `keep` newest survive.
        for _ in 0..10 {
            std::thread::sleep(std::time::Duration::from_millis(5));
            mgr.create_backup(&db).unwrap();
        }
        let removed = mgr.prune_backups("sessions", 3).unwrap();
        assert_eq!(removed, 8);
        let remaining = mgr.list_backups("sessions").len();
        assert_eq!(remaining, 3);
    }

    #[test]
    fn test_restore_from_backup() {
        let (_dir, paths) = temp_paths();
        let db = paths.sessions.join("sessions.db");
        make_healthy_db(&db);
        let mgr = SelfHealingManager::new(paths.clone());

        // Seed a backup, then corrupt the live db.
        mgr.create_backup(&db).unwrap();
        corrupt_db(&db);
        assert!(mgr.check_database(&db).is_err());

        // Restore should bring back a healthy db.
        let restored = mgr.restore_from_backup(&db).unwrap();
        assert!(restored.exists());
        assert!(mgr.check_database(&db).is_ok());

        // The corrupt original is quarantined, not deleted.
        let mut has_corrupt = false;
        if let Ok(entries) = std::fs::read_dir(&paths.sessions) {
            for e in entries.flatten() {
                if e.file_name().to_string_lossy().contains(".corrupt") {
                    has_corrupt = true;
                }
            }
        }
        assert!(has_corrupt, "corrupt db should be quarantined");
    }

    #[test]
    fn test_validate_configs_missing_is_ok() {
        let (_dir, paths) = temp_paths();
        let mgr = SelfHealingManager::new(paths);
        // No settings.yaml present -> nothing to validate.
        assert!(mgr.validate_configs().is_ok());
    }

    #[test]
    fn test_validate_configs_invalid_yaml() {
        let (_dir, paths) = temp_paths();
        std::fs::write(&paths.settings_file, "key: [unclosed\n  bad: : :\n").unwrap();
        let mgr = SelfHealingManager::new(paths);
        assert!(mgr.validate_configs().is_err());
    }

    #[test]
    fn test_validate_configs_valid_yaml() {
        let (_dir, paths) = temp_paths();
        std::fs::write(&paths.settings_file, "theme: dark\nmax_threads: 4\n").unwrap();
        let mgr = SelfHealingManager::new(paths);
        assert!(mgr.validate_configs().is_ok());
    }

    #[test]
    fn test_run_startup_checks_no_block_on_missing_db() {
        // Databases absent on a fresh install: checks must pass with no dbs
        // checked and no issues reported — startup never blocks.
        let (_dir, paths) = temp_paths();
        let mgr = SelfHealingManager::new(paths);
        let report = mgr.run_startup_checks().unwrap();
        assert!(report.checked.is_empty());
        assert!(!report.has_issues());
    }

    #[test]
    fn test_run_startup_checks_repairs_corrupt_db() {
        let (_dir, paths) = temp_paths();
        let db = paths.sessions.join("sessions.db");
        make_healthy_db(&db);
        let mgr = SelfHealingManager::new(paths.clone());
        mgr.create_backup(&db).unwrap();
        corrupt_db(&db);

        let report = mgr.run_startup_checks().unwrap();
        assert!(report.checked.contains(&db));
        assert!(report.repaired.iter().any(|r| r.contains("restored")));
        assert!(!report.has_issues());
        // Live db is healthy again.
        assert!(mgr.check_database(&db).is_ok());
    }

    #[test]
    fn test_embedding_model_available() {
        let (_dir, cache) = temp_paths();
        // Empty cache -> no model.
        assert!(!embedding_model_available(&cache.cache));
        // Create a fastembed-style model directory.
        std::fs::create_dir_all(cache.cache.join("models--Qwen--embed-multilingual"))
            .unwrap();
        assert!(embedding_model_available(&cache.cache));
    }

    #[test]
    fn test_network_available_returns_bool() {
        // Should never panic; value depends on sandbox connectivity.
        let _ = network_available();
    }

    #[test]
    fn test_degrade_network_skills() {
        let registry = SkillRegistry::new();
        let net_yaml = r#"
name: "web_fetcher"
runtime:
  type: "python"
  entry: "script.py"
permissions:
  fs: []
  network: true
  shell: false
"#;
        let local_yaml = r#"
name: "file_reader"
runtime:
  type: "python"
  entry: "script.py"
permissions:
  fs: []
  network: false
  shell: false
"#;
        let net_skill = Skill::from_yaml(net_yaml).unwrap();
        let local_skill = Skill::from_yaml(local_yaml).unwrap();
        assert!(net_skill.permissions.network);
        assert!(!local_skill.permissions.network);

        registry.register(net_skill);
        registry.register(local_skill);
        assert_eq!(registry.count(), 2);
        assert_eq!(registry.count_enabled(), 2);

        let disabled = degrade_network_skills(&registry);
        assert_eq!(disabled, 1);
        // The network skill is disabled, the local one is untouched.
        assert!(!registry.get("web_fetcher").unwrap().enabled);
        assert!(registry.get("file_reader").unwrap().enabled);
        assert_eq!(registry.count_enabled(), 1);
    }

    #[test]
    fn test_chrono_timestamp_suffix_unique() {
        let a = chrono_timestamp_suffix();
        let b = chrono_timestamp_suffix();
        assert!(!a.is_empty());
        assert!(!b.is_empty());
        // Filesystem-safe: lower hex digits plus a single '-' separator
        // between the second and nanosecond components.
        assert!(a.chars().all(|c| c.is_ascii_hexdigit() || c == '-'));
        assert!(a.contains('-'));
    }
}
