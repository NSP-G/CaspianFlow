//! Startup performance instrumentation (P34).
//!
//! A tiny phase timer that gives us a repeatable cold-start baseline and lets
//! the real app emit a startup report. Pure std + `tracing` — no Tauri
//! dependency — so it compiles under the default (headless) feature and is
//! exercised by the `cargo test --lib` baseline below.

use std::time::{Duration, Instant};

/// Accumulates named phase durations since construction.
///
/// `mark("config")` records wall time since the *previous* `mark` (or since
/// construction for the first call) and logs it. `report()` returns a flat
/// summary line suitable for the startup banner.
#[derive(Debug, Clone)]
pub struct StartupTimer {
    start: Instant,
    phases: Vec<(String, Duration)>,
}

impl Default for StartupTimer {
    fn default() -> Self {
        Self::new()
    }
}

impl StartupTimer {
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
            phases: Vec::new(),
        }
    }

    /// Mark the end of a named phase, logging its delta (ms) and returning it.
    pub fn mark(&mut self, phase: &str) -> Duration {
        let elapsed = self.start.elapsed();
        let delta = match self.phases.last() {
            Some((_, prev)) => elapsed - *prev,
            None => elapsed,
        };
        self.phases.push((phase.to_string(), delta));
        tracing::info!(
            phase = phase,
            ms = delta.as_millis() as u64,
            "startup phase"
        );
        delta
    }

    /// Total wall time since construction.
    pub fn total(&self) -> Duration {
        self.start.elapsed()
    }

    /// Flat, log-friendly summary: `startup: config@12ms skills@8ms total@20ms`.
    pub fn report(&self) -> String {
        let mut s = String::from("startup:");
        for (name, d) in &self.phases {
            s.push_str(&format!(" {name}@{:.1}ms", d.as_secs_f64() * 1000.0));
        }
        s.push_str(&format!(" total@{:.1}ms", self.total().as_secs_f64() * 1000.0));
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ConfigManager;
    use crate::skill::SkillManager;
    use crate::workflow::store::RunStore;

    /// Headless cold-start baseline for the non-GUI core (P34).
    ///
    /// Measures the three init phases that run before the window appears:
    /// config load, skill scan (idempotent builtin install + parallel scan),
    /// and the run store (SQLite open / schema init). The GUI's own cost
    /// (Tauri window creation + frontend hydration) is NOT covered here — it
    /// requires the webview runtime and is measured by Seeker via the
    /// `StartupTimer` wired into `run_tauri`.
    #[tokio::test]
    async fn startup_baseline_headless() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let mut t = StartupTimer::new();

        let _cfg = ConfigManager::init_with_paths(Some(home))
            .await
            .expect("config init");
        let cfg_ms = t.mark("config").as_millis() as u64;

        let skills_dir = home.join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();
        let _skills = SkillManager::init(&skills_dir).await.expect("skill init");
        let skills_ms = t.mark("skills").as_millis() as u64;

        let paths = crate::config::CaspianPaths::resolve(Some(home));
        let _store = RunStore::from_paths(&paths);
        let store_ms = t.mark("runstore").as_millis() as u64;

        let total_ms = t.total().as_millis() as u64;
        tracing::info!(
            config_ms = cfg_ms,
            skills_ms = skills_ms,
            store_ms = store_ms,
            total_ms = total_ms,
            "headless startup baseline"
        );
        eprintln!(
            "[startup baseline] config={cfg_ms}ms skills={skills_ms}ms runstore={store_ms}ms total={total_ms}ms"
        );

        // Regression guard: the headless core cold start must stay well under a
        // few seconds even on slow CI filesystems.
        assert!(
            total_ms < 10_000,
            "headless startup exceeded 10s budget: {total_ms}ms"
        );
    }
}
