//! Global path management for `~/.caspian/` directory tree.

use std::path::{Path, PathBuf};

/// Manages all paths under the CaspianFlow home directory.
#[derive(Debug, Clone)]
pub struct CaspianPaths {
    /// Root: `~/.caspian/`
    pub home: PathBuf,
    /// `~/.caspian/temp/` — runtime scratch space for per-run workflow state.
    /// Ephemeral: may be cleaned between runs. Persistent caches live under
    /// `cache`, never here (see the P20 storage-domain split).
    pub temp: PathBuf,
    /// `~/.caspian/cache/` — persistent caches that outlive a single run
    /// (e.g. P20 intermediate-result cache, keyed by workflow name).
    pub cache: PathBuf,
    /// `~/.caspian/agents/`
    pub agents: PathBuf,
    /// `~/.caspian/agents/shared/`
    pub shared: PathBuf,
    /// `~/.caspian/skills/`
    pub skills: PathBuf,
    /// `~/.caspian/sessions/`
    pub sessions: PathBuf,
    /// `~/.caspian/knowledge/`
    pub knowledge: PathBuf,
    /// `~/.caspian/config/`
    pub config: PathBuf,
    /// `~/.caspian/config/settings.yaml`
    pub settings_file: PathBuf,
    /// `~/.caspian/logs/`
    pub logs: PathBuf,
    /// `~/.caspian/backups/`
    pub backups: PathBuf,
    /// `~/.caspian/crash-reports/`
    pub crash_reports: PathBuf,
    /// `~/.caspian/workflows/` — user-authored workflow *definitions*
    /// (`<name>/workflow.yaml`). Distinct from the P17 run-state store under
    /// `temp/workflows/`. Drafts live in `<workflows>/.drafts/`.
    pub workflows: PathBuf,
    /// `~/.caspian/themes/` — user-installed theme packages (`<name>/`, P31).
    pub themes: PathBuf,
}

impl CaspianPaths {
    /// Resolve paths from the given home directory.
    /// If `home` is `None`, uses `~/.caspian/` (via the `dirs` crate).
    pub fn resolve(home: Option<&Path>) -> Self {
        let home = match home {
            Some(h) => h.to_path_buf(),
            None => {
                let base = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
                base.join(".caspian")
            }
        };

        let temp = home.join("temp");
        let cache = home.join("cache");
        let agents = home.join("agents");
        let shared = agents.join("shared");
        let skills = home.join("skills");
        let sessions = home.join("sessions");
        let knowledge = home.join("knowledge");
        let config = home.join("config");
        let settings_file = config.join("settings.yaml");
        let logs = home.join("logs");
        let backups = home.join("backups");
        let crash_reports = home.join("crash-reports");
        let workflows = home.join("workflows");
        let themes = home.join("themes");

        Self {
            home,
            temp,
            cache,
            agents,
            shared,
            skills,
            sessions,
            knowledge,
            config,
            settings_file,
            logs,
            backups,
            crash_reports,
            workflows,
            themes,
        }
    }

    /// Create all directories if they don't exist.
    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        for dir in &[
            &self.home,
            &self.temp,
            &self.cache,
            &self.agents,
            &self.shared,
            &self.skills,
            &self.sessions,
            &self.knowledge,
            &self.config,
            &self.logs,
            &self.backups,
            &self.crash_reports,
            &self.workflows,
            &self.themes,
        ] {
            std::fs::create_dir_all(dir)?;
        }
        Ok(())
    }

    /// Override the caspian home (used when `settings.yaml` specifies a custom path).
    pub fn with_caspian_home(&self, custom_home: &str) -> Self {
        let expanded = expand_tilde(custom_home);
        Self::resolve(Some(&expanded))
    }

    /// Get the settings file path as a string for display.
    pub fn settings_display(&self) -> String {
        self.settings_file.to_string_lossy().to_string()
    }
}

/// Expand `~` to the user's home directory.
pub fn expand_tilde(input: &str) -> PathBuf {
    if let Some(rest) = input.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    if input == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    }
    PathBuf::from(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_default_home() {
        let paths = CaspianPaths::resolve(None);
        assert!(paths.home.to_string_lossy().ends_with(".caspian"));
        assert!(paths.temp.to_string_lossy().ends_with("temp"));
        assert!(paths
            .settings_file
            .to_string_lossy()
            .ends_with("settings.yaml"));
        assert!(paths.agents.to_string_lossy().ends_with("agents"));
    }

    #[test]
    fn test_resolve_custom_home() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = CaspianPaths::resolve(Some(tmp.path()));
        assert_eq!(paths.home, tmp.path());
        assert_eq!(paths.temp, tmp.path().join("temp"));
        assert_eq!(paths.settings_file, tmp.path().join("config/settings.yaml"));
    }

    #[test]
    fn test_ensure_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = CaspianPaths::resolve(Some(tmp.path()));
        paths.ensure_dirs().unwrap();

        assert!(paths.home.exists());
        assert!(paths.temp.exists());
        assert!(paths.agents.exists());
        assert!(paths.shared.exists());
        assert!(paths.skills.exists());
        assert!(paths.sessions.exists());
        assert!(paths.knowledge.exists());
        assert!(paths.config.exists());
        assert!(paths.logs.exists());
        assert!(paths.backups.exists());
        assert!(paths.crash_reports.exists());
        assert!(paths.themes.exists());
    }

    #[test]
    fn test_expand_tilde() {
        let expanded = expand_tilde("~/foo/bar");
        assert!(expanded.to_string_lossy().contains("foo"));
        assert!(!expanded.to_string_lossy().starts_with("~"));
    }
}
