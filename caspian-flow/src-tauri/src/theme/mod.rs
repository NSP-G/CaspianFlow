//! Theme package loading + hot-reload (P31, Big Item A · A3).
//!
//! Reuses the P30 machinery:
//! - `DirWatcher` (from `crate::hot_reload`) watches `~/.caspian/themes/`.
//! - Issue reporting mirrors P30 `ScanReport` (`ScanIssue`) so the UI can tell
//!   the user *exactly* what is missing or broken ("X 主题包缺 manifest.yaml").
//!
//! A theme package is a directory under `paths.themes/<name>/` containing:
//! - `manifest.yaml` — `name` / `author` / `version` / `deps` / `description`
//! - `theme.css`     — pure CSS *variable overrides* (no JS), validated
//!
//! The CSS must only override design tokens (CSS custom properties). Hard
//! constraints enforced by [`validate_theme_css`]: no `!important`, bounded
//! selector complexity, bounded `backdrop-filter` usage, no `@import`.

use std::path::{Path, PathBuf};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

/// File name for the theme manifest inside a package directory.
pub const MANIFEST_FILE: &str = "manifest.yaml";
/// File name for the theme's CSS variable overrides.
pub const CSS_FILE: &str = "theme.css";
/// File storing the currently-active theme name (under `paths.themes/`).
const ACTIVE_FILE: &str = "_active.json";

/// Max `backdrop-filter` declarations allowed per theme (perf constraint).
const MAX_BACKDROP_FILTER: usize = 2;
/// Max descendant-combinator depth allowed per selector (perf constraint).
const MAX_SELECTOR_DEPTH: usize = 2;

/// Parsed `manifest.yaml` of a theme package.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeManifest {
    pub name: String,
    #[serde(default)]
    pub author: Option<String>,
    pub version: String,
    #[serde(default)]
    pub deps: Vec<String>,
    #[serde(default)]
    pub description: Option<String>,
}

/// Lightweight theme metadata returned to the UI.
#[derive(Debug, Clone, Serialize)]
pub struct ThemeMeta {
    pub name: String,
    pub author: Option<String>,
    pub version: String,
    /// Whether this theme is currently active.
    pub active: bool,
}

/// Why a theme package failed to load.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ThemeIssueKind {
    MissingManifest,
    ReadError,
    ParseError,
    ValidationError,
}

/// A single problem discovered while scanning a theme package.
#[derive(Debug, Clone, Serialize)]
pub struct ThemeIssue {
    pub kind: ThemeIssueKind,
    /// Filesystem path of the offending package (string for FFI safety).
    pub path: String,
    /// Package name if it could be determined.
    pub name: Option<String>,
    /// Human-readable reason (Chinese, user-comprehensible).
    pub reason: String,
}

/// Aggregate scan result: valid themes + issues.
#[derive(Debug, Clone, Serialize, Default)]
pub struct ThemeScanResult {
    pub themes: Vec<ThemeMeta>,
    pub issues: Vec<ThemeIssue>,
}

/// Result of `list_themes` — mirrors P30 `ModuleStatus` shape for the UI.
#[derive(Debug, Clone, Serialize, Default)]
pub struct ThemeListResult {
    pub themes: Vec<ThemeMeta>,
    pub issues: Vec<ThemeIssue>,
}

impl ThemeScanResult {
    pub fn has_issues(&self) -> bool {
        !self.issues.is_empty()
    }
}

/// Errors surfaced by explicit operations (apply / get_css) on a named theme.
#[derive(Debug, thiserror::Error)]
pub enum ThemeError {
    #[error("theme '{0}' not found")]
    NotFound(String),
    #[error("theme '{0}' is invalid: {1}")]
    Invalid(String, String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("yaml error: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("serialization error: {0}")]
    Serialization(String),
}

/// Scans + tracks theme packages under a directory.
///
/// Holds the currently-active theme name (persisted to `themes/_active.json`)
/// so the selection survives restarts. All scan failures become [`ThemeIssue`]s
/// — never `panic` (§3 resilience: a broken theme must not crash the app).
pub struct ThemeManager {
    dir: PathBuf,
    active: Mutex<Option<String>>,
}

impl ThemeManager {
    /// Create from the themes directory; loads any persisted active selection.
    pub fn new(dir: PathBuf) -> Self {
        let active = std::fs::read(dir.join(ACTIVE_FILE))
            .ok()
            .and_then(|b| serde_json::from_slice::<ActiveState>(&b).ok())
            .and_then(|s| s.active);
        Self {
            dir,
            active: Mutex::new(active),
        }
    }

    /// Directory backing this manager.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Scan all packages, returning valid themes + any issues.
    pub fn scan(&self) -> ThemeScanResult {
        let active = self.active.lock().clone();
        let mut result = ThemeScanResult::default();
        if !self.dir.exists() {
            return result;
        }

        let entries = match std::fs::read_dir(&self.dir) {
            Ok(e) => e,
            Err(e) => {
                result.issues.push(ThemeIssue {
                    kind: ThemeIssueKind::ReadError,
                    path: self.dir.to_string_lossy().to_string(),
                    name: None,
                    reason: format!("无法读取主题目录: {e}"),
                });
                return result;
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue; // skip _active.json etc.
            }
            let name = path
                .file_name()
                .expect("scanned entry has a file name")
                .to_string_lossy()
                .to_string();
            match Self::load_package(&path, &name) {
                Ok(meta) => {
                    let mut meta = meta;
                    meta.active = active.as_deref() == Some(&name);
                    result.themes.push(meta);
                }
                Err((kind, reason)) => result.issues.push(ThemeIssue {
                    kind,
                    path: path.to_string_lossy().to_string(),
                    name: Some(name),
                    reason,
                }),
            }
        }
        result
    }

    /// Load + validate a single package directory.
    fn load_package(
        path: &Path,
        _name: &str,
    ) -> Result<ThemeMeta, (ThemeIssueKind, String)> {
        let manifest_path = path.join(MANIFEST_FILE);
        if !manifest_path.exists() {
            return Err((
                ThemeIssueKind::MissingManifest,
                format!("主题包缺 {MANIFEST_FILE}"),
            ));
        }
        let manifest_str = std::fs::read_to_string(&manifest_path)
            .map_err(|e| (ThemeIssueKind::ReadError, format!("读取 manifest 失败: {e}")))?;
        let manifest: ThemeManifest = serde_yaml::from_str(&manifest_str)
            .map_err(|e| (ThemeIssueKind::ParseError, format!("manifest 解析失败: {e}")))?;

        let css_path = path.join(CSS_FILE);
        if !css_path.exists() {
            return Err((
                ThemeIssueKind::MissingManifest,
                format!("主题包缺 {CSS_FILE}"),
            ));
        }
        let css = std::fs::read_to_string(&css_path)
            .map_err(|e| (ThemeIssueKind::ReadError, format!("读取 {CSS_FILE} 失败: {e}")))?;
        validate_theme_css(&css)
            .map_err(|e| (ThemeIssueKind::ValidationError, e))?;

        Ok(ThemeMeta {
            name: manifest.name,
            author: manifest.author,
            version: manifest.version,
            active: false,
        })
    }

    /// List themes with the active flag set.
    pub fn list(&self) -> ThemeListResult {
        let scan = self.scan();
        ThemeListResult {
            themes: scan.themes,
            issues: scan.issues,
        }
    }

    /// Currently-active theme name, if any.
    pub fn active(&self) -> Option<String> {
        self.active.lock().clone()
    }

    /// Read a package's CSS (validated at load time).
    pub fn get_css(&self, name: &str) -> Result<String, ThemeError> {
        let path = self.dir.join(name).join(CSS_FILE);
        std::fs::read_to_string(&path).map_err(|_| ThemeError::NotFound(name.to_string()))
    }

    /// Activate a theme by name; persists the selection. Returns its CSS.
    pub fn apply(&self, name: &str) -> Result<String, ThemeError> {
        // Validate existence before committing the active state.
        let _ = self
            .scan()
            .themes
            .iter()
            .find(|t| t.name == name)
            .ok_or_else(|| ThemeError::NotFound(name.to_string()))?;
        let css = self.get_css(name)?;
        *self.active.lock() = Some(name.to_string());
        self.persist_active()?;
        Ok(css)
    }

    /// Clear the active theme (fall back to built-in dark/light).
    pub fn clear(&self) -> Result<(), ThemeError> {
        *self.active.lock() = None;
        self.persist_active()
    }

    fn persist_active(&self) -> Result<(), ThemeError> {
        let state = ActiveState {
            active: self.active.lock().clone(),
        };
        let bytes = serde_json::to_vec(&state)
            .map_err(|e| ThemeError::Serialization(e.to_string()))?;
        std::fs::write(self.dir.join(ACTIVE_FILE), bytes)?;
        Ok(())
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ActiveState {
    active: Option<String>,
}

/// Validate a theme's CSS against the P31 performance / safety constraints.
///
/// Returns `Err(reason)` on the first violation. Constraints:
/// 1. No `!important`.
/// 2. No `@import` (blocks external fetches).
/// 3. `backdrop-filter` usage bounded (`MAX_BACKDROP_FILTER`).
/// 4. Selector complexity bounded (`MAX_SELECTOR_DEPTH` descendant levels; no
///    child/sibling combinators `> + ~`).
pub fn validate_theme_css(css: &str) -> Result<(), String> {
    let lower = css.to_lowercase();

    if lower.contains("!important") {
        return Err("主题 CSS 禁止使用 !important".to_string());
    }
    if lower.contains("@import") {
        return Err("主题 CSS 禁止 @import（不允许外部加载）".to_string());
    }

    let backdrop_count = lower.matches("backdrop-filter").count()
        + lower.matches("-webkit-backdrop-filter").count();
    if backdrop_count > MAX_BACKDROP_FILTER {
        return Err(format!(
            "backdrop-filter 使用次数过多（{backdrop_count} > {MAX_BACKDROP_FILTER}）"
        ));
    }

    // Split into rules and inspect each selector for combinators / depth.
    for (i, rule) in css.split('}').enumerate() {
        let Some((selector, _body)) = rule.split_once('{') else {
            continue;
        };
        let selector = selector.trim();
        if selector.is_empty() || selector.starts_with('@') {
            continue; // e.g. @media — allow at-rule blocks structurally
        }
        if selector.contains('>') || selector.contains('+') || selector.contains('~') {
            return Err(format!(
                "选择器不允许子/兄弟组合符（> + ~）：第 {i} 条 «{selector}»"
            ));
        }
        let depth = selector.split_whitespace().count().saturating_sub(1);
        if depth > MAX_SELECTOR_DEPTH {
            return Err(format!(
                "选择器层级过深（{depth} > {MAX_SELECTOR_DEPTH}）：第 {i} 条 «{selector}»"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_theme(base: &Path, name: &str, manifest: &str, css: &str) {
        let dir = base.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        let mut m = std::fs::File::create(dir.join(MANIFEST_FILE)).unwrap();
        m.write_all(manifest.as_bytes()).unwrap();
        let mut c = std::fs::File::create(dir.join(CSS_FILE)).unwrap();
        c.write_all(css.as_bytes()).unwrap();
    }

    #[test]
    fn test_scan_valid_theme() {
        let tmp = tempfile::tempdir().unwrap();
        write_theme(
            tmp.path(),
            "midnight",
            "name: midnight\nversion: 1.0.0\nauthor: lantern\n",
            ":root { --color-accent: #ff6600; --background: #101010; }\n",
        );
        let mgr = ThemeManager::new(tmp.path().to_path_buf());
        let res = mgr.scan();
        assert!(!res.has_issues(), "unexpected issues: {:?}", res.issues);
        assert_eq!(res.themes.len(), 1);
        assert_eq!(res.themes[0].name, "midnight");
        assert_eq!(res.themes[0].version, "1.0.0");
    }

    #[test]
    fn test_scan_missing_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("broken")).unwrap();
        let mgr = ThemeManager::new(tmp.path().to_path_buf());
        let res = mgr.scan();
        assert!(res.has_issues());
        assert_eq!(res.issues[0].kind, ThemeIssueKind::MissingManifest);
        assert_eq!(res.themes.len(), 0);
    }

    #[test]
    fn test_scan_rejects_important() {
        let tmp = tempfile::tempdir().unwrap();
        write_theme(
            tmp.path(),
            "bad",
            "name: bad\nversion: 1.0.0\n",
            ":root { --color-accent: red !important; }\n",
        );
        let mgr = ThemeManager::new(tmp.path().to_path_buf());
        let res = mgr.scan();
        assert!(res.has_issues());
        assert_eq!(res.issues[0].kind, ThemeIssueKind::ValidationError);
    }

    #[test]
    fn test_validate_css_constraints() {
        assert!(validate_theme_css(":root { --x: 1; }").is_ok());
        assert!(validate_theme_css(":root { --x: 1 !important; }").is_err());
        assert!(validate_theme_css("@import url(x);").is_err());
        assert!(validate_theme_css(
            "a b c d { --x: 1; }" // depth 3 > 2
        )
        .is_err());
        assert!(validate_theme_css("a > b { --x: 1; }").is_err());
        assert!(validate_theme_css(
            ":root { --x: 1; } :root { backdrop-filter: blur(1px); backdrop-filter: blur(2px); backdrop-filter: blur(3px); }"
        )
        .is_err());
    }

    #[test]
    fn test_apply_and_persist() {
        let tmp = tempfile::tempdir().unwrap();
        write_theme(
            tmp.path(),
            "midnight",
            "name: midnight\nversion: 1.0.0\n",
            ":root { --color-accent: #ff6600; }\n",
        );
        let mgr = ThemeManager::new(tmp.path().to_path_buf());
        let css = mgr.apply("midnight").unwrap();
        assert!(css.contains("#ff6600"));
        assert_eq!(mgr.active(), Some("midnight".to_string()));

        // New manager reads persisted active state.
        let mgr2 = ThemeManager::new(tmp.path().to_path_buf());
        assert_eq!(mgr2.active(), Some("midnight".to_string()));
        let listed = mgr2.list();
        assert!(listed.themes.iter().any(|t| t.name == "midnight" && t.active));

        mgr2.clear().unwrap();
        assert_eq!(mgr2.active(), None);
    }

    #[test]
    fn test_apply_nonexistent_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = ThemeManager::new(tmp.path().to_path_buf());
        assert!(matches!(mgr.apply("ghost"), Err(ThemeError::NotFound(_))));
    }
}
