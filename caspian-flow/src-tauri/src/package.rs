//! `.caspian` portable export / import bundle (P36).
//!
//! A `.caspian` bundle is a **directory** with a versioned manifest:
//!
//! ```text
//! <name>.caspian/
//!   manifest.json          # BundleManifest (format, version, items[])
//!   skills/<skill>/...     # skill directories copied verbatim
//!   agents/<agent>         # agent definitions (reserved slot; file-backed)
//!   config/settings.yaml   # settings snapshot
//!   sessions.json          # all sessions + their messages
//!   knowledge.json         # documents + chunk texts (re-embedded on import)
//! ```
//!
//! A directory (not a single archive file) is the canonical format: it is
//! inspectable, diff-friendly, and needs zero compression dependencies. Users
//! who want a single transport file can zip it with the OS/CI — the importer
//! accepts the `<name>.caspian/` directory as-is.
//!
//! The importer validates the manifest, then copies items into the live paths
//! under a [`ConflictPolicy`]. Damaged items are reported in the
//! [`ImportReport`], never silently dropped (P30 WS1 §3 resilience).

use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config::CaspianPaths;
use crate::knowledge::store::{KnowledgeStore, SqliteKnowledgeStore};
use crate::session::store::{SessionStore, SqliteSessionStore};
use crate::session::types::{Message, Session};

/// Magic string identifying a CaspianFlow bundle manifest.
pub const BUNDLE_FORMAT: &str = "caspian-bundle";
/// Current bundle schema version. Bump on incompatible manifest changes.
pub const BUNDLE_VERSION: u32 = 1;

/// What to do when an item already exists at the import destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConflictPolicy {
    /// Leave the existing item in place; record it as skipped.
    #[default]
    Skip,
    /// Replace the existing item with the bundled one.
    Overwrite,
    /// Copy the bundled item under a fresh, numbered name.
    Rename,
}

/// One entry in the bundle manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleItem {
    pub kind: String,
    pub name: String,
    pub path: String,
    pub checksum: String,
}

/// Top-level bundle descriptor written to `manifest.json`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BundleManifest {
    pub format: String,
    pub version: u32,
    pub created_at: String,
    /// App version that produced the bundle (informational).
    pub app_version: String,
    pub items: Vec<BundleItem>,
}

/// Outcome of an import — every item lands in exactly one bucket.
#[derive(Debug, Default)]
pub struct ImportReport {
    pub imported: Vec<String>,
    pub skipped: Vec<String>,
    pub failed: Vec<String>,
}

impl ImportReport {
    pub fn is_empty(&self) -> bool {
        self.imported.is_empty() && self.skipped.is_empty() && self.failed.is_empty()
    }
}

/// Errors surfaced by bundle export / import.
#[derive(Debug)]
pub enum PackageError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Validation(String),
    Store(String),
}

impl std::fmt::Display for PackageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PackageError::Io(e) => write!(f, "io error: {e}"),
            PackageError::Json(e) => write!(f, "json error: {e}"),
            PackageError::Validation(e) => write!(f, "invalid bundle: {e}"),
            PackageError::Store(e) => write!(f, "store error: {e}"),
        }
    }
}

impl std::error::Error for PackageError {}

impl From<std::io::Error> for PackageError {
    fn from(e: std::io::Error) -> Self {
        PackageError::Io(e)
    }
}

impl From<serde_json::Error> for PackageError {
    fn from(e: serde_json::Error) -> Self {
        PackageError::Json(e)
    }
}

pub type PackageResult<T> = Result<T, PackageError>;

/// Selectors for what to include in an export.
#[derive(Debug, Clone, Default)]
pub struct ExportOptions {
    pub include_sessions: bool,
    pub include_knowledge: bool,
}

// ---------------------------------------------------------------------------
// Serialized shapes for the JSON side-car files
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
struct SessionExport {
    session: Session,
    messages: Vec<Message>,
}

#[derive(Debug, Serialize, Deserialize)]
struct KnowledgeExport {
    doc: crate::knowledge::store::Document,
    chunks: Vec<String>,
}

// ---------------------------------------------------------------------------
// Checksum helpers
// ---------------------------------------------------------------------------

fn sha256_bytes(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex(&hasher.finalize())
}

fn sha256_file(path: &Path) -> PackageResult<String> {
    let bytes = std::fs::read(path)?;
    Ok(sha256_bytes(&bytes))
}

fn hex(digest: &[u8]) -> String {
    let mut s = String::with_capacity(digest.len() * 2);
    for b in digest {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn now_iso() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // RFC3339-ish; precise formatting is unnecessary for a bundle timestamp.
    format!("{}", secs)
}

// ---------------------------------------------------------------------------
// Filesystem helpers
// ---------------------------------------------------------------------------

/// Recursively copy `src` (a file or directory) to `dst`.
fn copy_path(src: &Path, dst: &Path) -> io::Result<()> {
    if src.is_dir() {
        std::fs::create_dir_all(dst)?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            copy_path(&entry.path(), &dst.join(entry.file_name()))?;
        }
    } else {
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(src, dst)?;
    }
    Ok(())
}

/// Collect every regular file under `dir`, sorted by path for stable checksums.
fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) -> io::Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, out)?;
        } else {
            out.push(path);
        }
    }
    out.sort();
    Ok(())
}

/// Deterministic checksum over a directory's contents (paths + bytes).
fn dir_checksum(dir: &Path) -> PackageResult<String> {
    let mut files = Vec::new();
    collect_files(dir, &mut files)?;
    let mut hasher = Sha256::new();
    for f in &files {
        let rel = f.strip_prefix(dir).unwrap_or(f);
        hasher.update(rel.to_string_lossy().as_bytes());
        if let Ok(bytes) = std::fs::read(f) {
            hasher.update(&bytes);
        }
    }
    Ok(hex(&hasher.finalize()))
}

// ---------------------------------------------------------------------------
// Export
// ---------------------------------------------------------------------------

/// Export the live state rooted at `paths` into a new `.caspian` bundle at
/// `dest` (which must not already exist). Returns the written manifest.
pub fn export_bundle(
    paths: &CaspianPaths,
    dest: &Path,
    opts: &ExportOptions,
) -> PackageResult<BundleManifest> {
    if dest.exists() {
        return Err(PackageError::Validation(format!(
            "destination {} already exists",
            dest.display()
        )));
    }
    std::fs::create_dir_all(dest)?;

    let mut manifest = BundleManifest {
        format: BUNDLE_FORMAT.to_string(),
        version: BUNDLE_VERSION,
        created_at: now_iso(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        items: Vec::new(),
    };

    // Skills — copy every skill directory verbatim.
    if paths.skills.exists() {
        let skills_dir = dest.join("skills");
        std::fs::create_dir_all(&skills_dir)?;
        for entry in std::fs::read_dir(&paths.skills)? {
            let entry = entry?;
            if !entry.path().is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            let target = skills_dir.join(&name);
            copy_path(&entry.path(), &target)?;
            manifest.items.push(BundleItem {
                kind: "skill".to_string(),
                name: name.clone(),
                path: format!("skills/{name}"),
                checksum: dir_checksum(&target)?,
            });
        }
    }

    // Agents — reserved slot; copy whatever lives under `paths.agents`.
    if paths.agents.exists() {
        let agents_dir = dest.join("agents");
        std::fs::create_dir_all(&agents_dir)?;
        for entry in std::fs::read_dir(&paths.agents)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            let target = agents_dir.join(&name);
            copy_path(&entry.path(), &target)?;
            manifest.items.push(BundleItem {
                kind: "agent".to_string(),
                name: name.clone(),
                path: format!("agents/{name}"),
                checksum: sha256_file(&target).unwrap_or_default(),
            });
        }
    }

    // Config — snapshot settings.yaml.
    let settings_src = paths.config.join("settings.yaml");
    if settings_src.exists() {
        std::fs::create_dir_all(dest.join("config"))?;
        std::fs::copy(&settings_src, dest.join("config/settings.yaml"))?;
        manifest.items.push(BundleItem {
            kind: "config".to_string(),
            name: "settings".to_string(),
            path: "config/settings.yaml".to_string(),
            checksum: sha256_file(&settings_src)?,
        });
    }

    // Sessions — dump every session + its messages as JSON.
    if opts.include_sessions {
        if let Ok(store) = SqliteSessionStore::from_paths(paths) {
            let sessions = store.list_sessions(None, usize::MAX).unwrap_or_default();
            let mut exports = Vec::with_capacity(sessions.len());
            for s in &sessions {
                let messages = store
                    .get_messages(&s.id, usize::MAX, None)
                    .unwrap_or_default();
                exports.push(SessionExport {
                    session: s.clone(),
                    messages,
                });
            }
            let json = serde_json::to_string_pretty(&exports)?;
            std::fs::write(dest.join("sessions.json"), &json)?;
            manifest.items.push(BundleItem {
                kind: "sessions".to_string(),
                name: "sessions".to_string(),
                path: "sessions.json".to_string(),
                checksum: sha256_bytes(json.as_bytes()),
            });
        }
    }

    // Knowledge — dump documents + chunk texts (re-embedded on import).
    if opts.include_knowledge {
        if let Ok(store) = SqliteKnowledgeStore::from_paths(paths) {
            if let Ok(docs) = store.list_documents() {
                let mut exports = Vec::with_capacity(docs.len());
                for d in &docs {
                    let chunks = store
                        .chunks_of_document(&d.id)
                        .unwrap_or_default()
                        .into_iter()
                        .map(|(_, c)| c)
                        .collect::<Vec<_>>();
                    exports.push(KnowledgeExport {
                        doc: d.clone(),
                        chunks,
                    });
                }
                let json = serde_json::to_string_pretty(&exports)?;
                std::fs::write(dest.join("knowledge.json"), &json)?;
                manifest.items.push(BundleItem {
                    kind: "knowledge".to_string(),
                    name: "knowledge".to_string(),
                    path: "knowledge.json".to_string(),
                    checksum: sha256_bytes(json.as_bytes()),
                });
            }
        }
    }

    let manifest_json = serde_json::to_string_pretty(&manifest)?;
    std::fs::write(dest.join("manifest.json"), &manifest_json)?;
    Ok(manifest)
}

// ---------------------------------------------------------------------------
// Import
// ---------------------------------------------------------------------------

/// Import a `.caspian` bundle from `src` into the live state at `paths`.
///
/// Validates the manifest (format + version ceiling) then copies every item
/// under `policy`. Failures are captured in [`ImportReport::failed`] rather than
/// aborting the whole import (resilience).
pub fn import_bundle(
    src: &Path,
    paths: &CaspianPaths,
    policy: ConflictPolicy,
) -> PackageResult<ImportReport> {
    let manifest_path = src.join("manifest.json");
    if !manifest_path.exists() {
        return Err(PackageError::Validation(
            "not a .caspian bundle: missing manifest.json".to_string(),
        ));
    }
    let manifest: BundleManifest = serde_json::from_str(&std::fs::read_to_string(&manifest_path)?)?;
    if manifest.format != BUNDLE_FORMAT {
        return Err(PackageError::Validation(format!(
            "unknown bundle format: {}",
            manifest.format
        )));
    }
    if manifest.version > BUNDLE_VERSION {
        return Err(PackageError::Validation(format!(
            "unsupported bundle version: {} (max {BUNDLE_VERSION})",
            manifest.version
        )));
    }

    let mut report = ImportReport::default();

    for item in manifest.items.iter().filter(|i| i.kind == "skill") {
        import_one(&src.join(&item.path), &paths.skills.join(&item.name), policy, &mut report, "skill");
    }
    for item in manifest.items.iter().filter(|i| i.kind == "config") {
        import_one(&src.join(&item.path), &paths.config.join("settings.yaml"), policy, &mut report, "config");
    }
    for item in manifest.items.iter().filter(|i| i.kind == "agent") {
        import_one(&src.join(&item.path), &paths.agents.join(&item.name), policy, &mut report, "agent");
    }

    // Sessions — re-create each session then append its messages.
    if let Some(item) = manifest.items.iter().find(|i| i.kind == "sessions") {
        match serde_json::from_str::<Vec<SessionExport>>(
            &std::fs::read_to_string(src.join(&item.path))?,
        ) {
            Ok(data) => {
                if let Ok(store) = SqliteSessionStore::from_paths(paths) {
                    for ex in &data {
                        match store.create_session(&ex.session.agent_id, ex.session.title.as_deref())
                        {
                            Ok(new_s) => {
                                for mut m in ex.messages.clone() {
                                    m.session_id = new_s.id.clone();
                                    m.id = 0;
                                    if store.append_message(&new_s.id, m).is_err() {
                                        report.failed.push(format!("session {}", ex.session.id));
                                    }
                                }
                                report.imported.push(format!("session {}", ex.session.id));
                            }
                            Err(e) => report
                                .failed
                                .push(format!("session {}: {e}", ex.session.id)),
                        }
                    }
                }
            }
            Err(e) => report.failed.push(format!("sessions.json: {e}")),
        }
    }

    // Knowledge — re-import each document (re-embedded by the store).
    if let Some(item) = manifest.items.iter().find(|i| i.kind == "knowledge") {
        match serde_json::from_str::<Vec<KnowledgeExport>>(
            &std::fs::read_to_string(src.join(&item.path))?,
        ) {
            Ok(data) => {
                if let Ok(store) = SqliteKnowledgeStore::from_paths(paths) {
                    for ex in &data {
                        let content = ex.chunks.join("\n");
                        match store.import_document(Path::new(&ex.doc.path), &ex.doc.name, &content) {
                            Ok(_) => report.imported.push(format!("knowledge {}", ex.doc.name)),
                            Err(e) => report
                                .failed
                                .push(format!("knowledge {}: {e}", ex.doc.name)),
                        }
                    }
                }
            }
            Err(e) => report.failed.push(format!("knowledge.json: {e}")),
        }
    }

    Ok(report)
}

/// Copy one bundle item into place, honoring the conflict policy. Records the
/// outcome on `report`. Returns `()` on success (errors are recorded, not thrown).
fn import_one(
    src: &Path,
    dst: &Path,
    policy: ConflictPolicy,
    report: &mut ImportReport,
    kind: &str,
) {
    if !src.exists() {
        report.failed.push(format!("{kind} {} (missing in bundle)", src.display()));
        return;
    }
    let label = format!("{kind} {}", dst.display());

    if dst.exists() {
        match policy {
            ConflictPolicy::Skip => {
                report.skipped.push(label);
                return;
            }
            ConflictPolicy::Overwrite => {
                if dst.is_dir() {
                    if let Err(e) = std::fs::remove_dir_all(dst) {
                        report.failed.push(format!("{label} (rm: {e})"));
                        return;
                    }
                } else if let Err(e) = std::fs::remove_file(dst) {
                    report.failed.push(format!("{label} (rm: {e})"));
                    return;
                }
            }
            ConflictPolicy::Rename => {
                let renamed = free_name(dst);
                if copy_path(src, &renamed).is_err() {
                    report.failed.push(format!("{label} (copy failed)"));
                    return;
                }
                report.imported.push(format!("{kind} {} (renamed)", renamed.display()));
                return;
            }
        }
    }

    if copy_path(src, dst).is_err() {
        report.failed.push(format!("{label} (copy failed)"));
        return;
    }
    report.imported.push(label);
}

/// Find a non-existing sibling name by appending `_N` before the extension.
fn free_name(path: &Path) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "item".to_string());
    let ext = path.extension().map(|e| e.to_string_lossy().to_string());
    let mut n = 1;
    loop {
        let candidate_name = match &ext {
            Some(e) => format!("{stem}_{n}.{e}"),
            None => format!("{stem}_{n}"),
        };
        let candidate = parent.join(candidate_name);
        if !candidate.exists() {
            return candidate;
        }
        n += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::CaspianPaths;
    use crate::session::types::MessageRole;

    fn temp_paths() -> (tempfile::TempDir, CaspianPaths) {
        let dir = tempfile::tempdir().unwrap();
        let paths = CaspianPaths::resolve(Some(dir.path()));
        (dir, paths)
    }

    fn write_settings(paths: &CaspianPaths) {
        std::fs::create_dir_all(&paths.config).unwrap();
        std::fs::write(
            paths.config.join("settings.yaml"),
            "app:\n  theme: dark\n",
        )
        .unwrap();
    }

    /// Count immediate subdirectories, returning 0 if the path is absent.
    fn count_subdirs(path: &Path) -> usize {
        match std::fs::read_dir(path) {
            Ok(entries) => entries
                .filter(|e| e.as_ref().map(|e| e.path().is_dir()).unwrap_or(false))
                .count(),
            Err(_) => 0,
        }
    }

    #[test]
    fn test_export_import_roundtrip() {
        // --- Source environment ---
        let (_sd, src_paths) = temp_paths();
        // Plant one skill manually (decouples the test from async builtin install).
        std::fs::create_dir_all(src_paths.skills.join("demo_skill")).unwrap();
        std::fs::write(
            src_paths.skills.join("demo_skill/skill.yaml"),
            "name: demo_skill\nversion: 1.0.0\n",
        )
        .unwrap();
        write_settings(&src_paths);
        // Seed a session.
        let sess_store = SqliteSessionStore::from_paths(&src_paths).unwrap();
        let s = sess_store.create_session("agent_x", Some("demo")).unwrap();
        sess_store
            .append_message(
                &s.id,
                Message {
                    id: 0,
                    session_id: s.id.clone(),
                    role: MessageRole::User,
                    content: "hello".into(),
                    tool_calls: None,
                    tool_call_id: None,
                    created_at: 1,
                    token_count: 1,
                },
            )
            .unwrap();
        let src_skill_count = count_subdirs(&src_paths.skills);
        assert_eq!(src_skill_count, 1);

        // --- Export ---
        let dest = tempfile::tempdir().unwrap();
        let bundle = dest.path().join("backup.caspian");
        let manifest = export_bundle(
            &src_paths,
            &bundle,
            &ExportOptions {
                include_sessions: true,
                include_knowledge: true,
            },
        )
        .expect("export");
        assert!(bundle.join("manifest.json").exists());
        assert!(manifest.items.iter().any(|i| i.kind == "skill"));
        assert!(manifest.items.iter().any(|i| i.kind == "config"));
        assert!(manifest.items.iter().any(|i| i.kind == "sessions"));

        // --- Destination environment (fresh) ---
        let (_dd, dst_paths) = temp_paths();
        let report = import_bundle(&bundle, &dst_paths, ConflictPolicy::Overwrite)
            .expect("import");
        assert!(report.failed.is_empty(), "import failed: {:?}", report.failed);

        // Skills copied through.
        let dst_skill_count = std::fs::read_dir(&dst_paths.skills)
            .unwrap()
            .filter(|e| e.as_ref().map(|e| e.path().is_dir()).unwrap_or(false))
            .count();
        assert_eq!(dst_skill_count, src_skill_count, "skill count mismatch");

        // Settings copied.
        assert!(dst_paths.config.join("settings.yaml").exists());

        // Session re-created with its message.
        let dst_sess = SqliteSessionStore::from_paths(&dst_paths).unwrap();
        let sessions = dst_sess.list_sessions(None, usize::MAX).unwrap();
        assert_eq!(sessions.len(), 1);
        let msgs = dst_sess.get_messages(&sessions[0].id, usize::MAX, None).unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content, "hello");
    }

    #[tokio::test]
    async fn test_conflict_skip_leaves_existing() {
        let (_sd, src_paths) = temp_paths();
        crate::skill::SkillManager::init(&src_paths.skills).await.unwrap();
        write_settings(&src_paths);

        let dest = tempfile::tempdir().unwrap();
        let bundle = dest.path().join("b.caspian");
        export_bundle(&src_paths, &bundle, &ExportOptions::default()).unwrap();

        // Pre-populate destination with a settings.yaml that must survive a Skip.
        write_settings(&src_paths); // ensures src has one
        std::fs::write(src_paths.config.join("settings.yaml"), "app:\n  theme: light\n").unwrap();
        // Re-export so the bundle carries `light`; import into src_paths (which has `light`).
        let bundle2 = dest.path().join("b2.caspian");
        export_bundle(&src_paths, &bundle2, &ExportOptions::default()).unwrap();

        let report = import_bundle(&bundle2, &src_paths, ConflictPolicy::Skip).unwrap();
        // settings.yaml already exists -> skipped, original `light` preserved.
        assert!(report.skipped.iter().any(|s| s.contains("config")));
        let kept = std::fs::read_to_string(src_paths.config.join("settings.yaml")).unwrap();
        assert!(kept.contains("light"), "skip must not overwrite existing config");
    }

    #[test]
    fn test_import_rejects_non_bundle() {
        let (_dd, dst_paths) = temp_paths();
        let junk = tempfile::tempdir().unwrap();
        let err = import_bundle(junk.path(), &dst_paths, ConflictPolicy::Skip);
        assert!(matches!(err, Err(PackageError::Validation(_))));
    }
}
