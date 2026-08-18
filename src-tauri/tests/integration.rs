//! Integration tests for CaspianFlow — cross-module flows exercised through the
//! public crate API (not unit tests of internal helpers).
//!
//! These deliberately run against real on-disk state (temp dirs) so they catch
//! wiring regressions that unit tests isolated to one module would miss.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

use caspian_flow::config::CaspianPaths;
use caspian_flow::package::{export_bundle, import_bundle, ConflictPolicy, ExportOptions};
use caspian_flow::self_healing::{
    degrade_network_skills, embedding_model_available, network_available, SelfHealingManager,
};
use caspian_flow::skill::SkillManager;
use rusqlite::Connection;
use tokio::runtime::Runtime;

/// Build an isolated `~/.caspian` tree under a unique temp dir so no test
/// touches the developer's real home directory.
fn temp_paths() -> (PathBuf, CaspianPaths) {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("caspian-it-{}-{}", std::process::id(), n));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let paths = CaspianPaths::resolve(Some(&dir));
    paths.ensure_dirs().unwrap();
    (dir, paths)
}

fn make_healthy_db(path: &std::path::Path) {
    let _ = std::fs::remove_file(path);
    let conn = Connection::open(path).unwrap();
    conn.execute_batch("CREATE TABLE t(id INTEGER PRIMARY KEY, v TEXT); INSERT INTO t(v) VALUES('x');")
        .unwrap();
}

/// P37 + skill module: a fresh init installs all 17 builtin skills and surfaces
/// them in the registry.
#[test]
fn integration_skill_manager_installs_builtins() {
    let rt = Runtime::new().unwrap();
    let (_dir, paths) = temp_paths();
    rt.block_on(async {
        let mgr = SkillManager::init(&paths.skills).await.unwrap();
        let reg = mgr.registry();
        let total = reg.count();
        let enabled = reg.count_enabled();
        assert!(
            total >= 17,
            "expected >=17 builtin skills in registry, got {total}"
        );
        assert!(
            enabled >= 17,
            "expected >=17 enabled builtin skills, got {enabled}"
        );
    });
}

/// P38 + real SQLite: create a healthy sessions.db, back it up, corrupt it,
/// then restore from backup and confirm it is healthy again.
#[test]
fn integration_self_healing_backup_restore_roundtrip() {
    let (_dir, paths) = temp_paths();
    let db = paths.sessions.join("sessions.db");
    make_healthy_db(&db);

    let mgr = SelfHealingManager::new(paths.clone());
    let backup = mgr.create_backup(&db).unwrap();
    assert!(backup.exists(), "backup file should exist");

    // Corrupt the live db.
    std::fs::write(&db, b"not a sqlite database at all").unwrap();
    assert!(
        mgr.check_database(&db).is_err(),
        "corrupted db should fail integrity check"
    );

    // Restore from the backup.
    mgr.restore_from_backup(&db).unwrap();
    assert!(
        mgr.check_database(&db).is_ok(),
        "restored db should pass integrity check"
    );
}

/// P36 + package module: install builtin skills, export a bundle, then import it
/// into a fresh tree and confirm the skills round-trip.
#[test]
fn integration_package_export_import_roundtrip() {
    let rt = Runtime::new().unwrap();
    let (_dir, paths) = temp_paths();
    // Install builtin skills so the export has real content to carry.
    rt.block_on(async {
        SkillManager::init(&paths.skills).await.unwrap();
    });

    let dest = _dir.join("bundle.caspian");
    let opts = ExportOptions {
        include_sessions: true,
        include_knowledge: true,
    };
    let manifest = export_bundle(&paths, &dest, &opts).unwrap();
    assert!(
        dest.join("manifest.json").exists(),
        "bundle must contain manifest.json"
    );

    let (_dir2, paths2) = temp_paths();
    let report = import_bundle(&dest, &paths2, ConflictPolicy::Skip).unwrap();
    // All 17 builtin skills should have been imported into the fresh tree.
    assert!(
        report.imported.len() >= 17,
        "expected >=17 skills imported, got {}",
        report.imported.len()
    );
    assert_eq!(
        report.failed.len(),
        0,
        "no items should fail on a fresh import"
    );
    // The manifest must describe at least as many items as were imported
    // (some categories may add placeholder entries for empty data stores).
    assert!(manifest.items.len() >= report.imported.len());
}

/// P37 + P38: offline degradation disables network skills while keeping local
/// ones enabled — verified through the real SkillManager registry.
#[test]
fn integration_degrade_network_skills_offline() {
    let rt = Runtime::new().unwrap();
    let (_dir, paths) = temp_paths();
    rt.block_on(async {
        let mgr = SkillManager::init(&paths.skills).await.unwrap();
        let reg = mgr.registry();
        let before = reg.count_enabled();

        // network_available() may be true or false in CI; degradation logic is
        // what we assert, independent of the actual result.
        let _reachable = network_available();

        let disabled = degrade_network_skills(reg);
        assert!(
            disabled >= 1,
            "at least the network skills (web-fetcher/http_request) should be disabled"
        );
        assert_eq!(
            reg.count_enabled(),
            before - disabled,
            "enabled count should drop by exactly the number disabled"
        );
    });
}

/// P38 graceful degradation: the embedding-model probe correctly reports
/// presence/absence of a cached fastembed model directory.
#[test]
fn integration_embedding_model_probe() {
    let (_dir, paths) = temp_paths();
    assert!(
        !embedding_model_available(&paths.cache),
        "empty cache should report no model"
    );
    std::fs::create_dir_all(paths.cache.join("models--Qwen--embed-multilingual")).unwrap();
    assert!(
        embedding_model_available(&paths.cache),
        "cache with a models-- dir should report a model present"
    );
}

/// P40 performance budget: startup self-healing checks must be cheap on a fresh
/// install (no databases yet) and must never block.
#[test]
fn integration_startup_checks_perf_budget() {
    let (_dir, paths) = temp_paths();
    let mgr = SelfHealingManager::new(paths);
    let start = Instant::now();
    let report = mgr.run_startup_checks().unwrap();
    let elapsed = start.elapsed();
    assert!(
        elapsed.as_millis() < 2000,
        "startup checks took {}ms (budget 2000ms)",
        elapsed.as_millis()
    );
    assert!(!report.has_issues(), "fresh install should report no issues");
}
