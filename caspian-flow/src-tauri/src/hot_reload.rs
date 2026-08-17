//! Generic directory hot-reload watcher (P30 WS2).
//!
//! Reuses the exact pattern from `config/watcher.rs` (`ConfigWatcher`): the
//! underlying `notify` debouncer is `!Sync`, so it is parked inside a
//! `Mutex<Option<Box<dyn Any + Send>>>` purely to restore the `Sync` auto-trait
//! — we never lock it; the `Mutex` is RAII only. A background thread drains the
//! debounce channel and fires a [`DirChangeCallback`] on any change.
//!
//! Unlike `ConfigWatcher`, this watcher is generic: it does not know *what* to
//! do on a change, it just debounces filesystem events and invokes the caller's
//! callback. Skill reloading and workflow-list refresh are wired in
//! `tauri_app.rs`.

use std::any::Any;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use notify::RecursiveMode;
use notify_debouncer_mini::new_debouncer;
use parking_lot::Mutex;

use crate::types::{AppError, AppResult};

/// Debounce window for coalescing bursts of filesystem events.
const DEBOUNCE_MS: u64 = 500;

/// Callback fired (debounced) when the watched directory changes.
pub type DirChangeCallback = Arc<dyn Fn() + Send + Sync>;

/// Watches a directory (recursively) and fires `cb` on any change.
///
/// If the target does not exist at construction time, the watcher is disabled
/// (returns `Ok` with no live debouncer — no panic). Callers that must observe
/// children created *later* (e.g. the workflows dir) should `create_dir_all`
/// the target before calling `watch`.
pub struct DirWatcher {
    /// The live debouncer, kept alive to keep watching. `Mutex` only restores
    /// `Sync` for the `!Sync` `notify` watcher; never locked at runtime.
    _debouncer: Mutex<Option<Box<dyn Any + Send>>>,
}

impl DirWatcher {
    /// Start watching `path` (recursively) and invoke `cb` on any change.
    pub fn watch(path: &Path, cb: DirChangeCallback) -> AppResult<Self> {
        if !path.exists() {
            tracing::warn!(
                path = %path.display(),
                "watch target missing, hot-reload disabled"
            );
            return Ok(Self {
                _debouncer: Mutex::new(None),
            });
        }

        let (tx, rx) = std::sync::mpsc::channel();
        let mut deb = new_debouncer(Duration::from_millis(DEBOUNCE_MS), tx)
            .map_err(AppError::Notify)?;

        deb.watcher()
            .watch(path, RecursiveMode::Recursive)
            .map_err(AppError::Notify)?;

        // Drain the debounce channel on a dedicated thread and fire `cb`.
        std::thread::spawn(move || {
            while let Ok(events_result) = rx.recv() {
                match events_result {
                    Ok(events) => {
                        if !events.is_empty() {
                            cb();
                        }
                    }
                    Err(errors) => {
                        tracing::warn!(errors = ?errors, "dir watcher errors");
                    }
                }
            }
        });

        Ok(Self {
            _debouncer: Mutex::new(Some(Box::new(deb))),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc as StdArc;

    #[test]
    fn test_watch_missing_dir_is_disabled_no_panic() {
        // A missing target must not panic or error — hot-reload is best-effort.
        let cb: DirChangeCallback = StdArc::new(|| {});
        let watcher = DirWatcher::watch(Path::new("/nonexistent/skills_dir"), cb).unwrap();
        let _ = watcher;
    }

    #[test]
    #[ignore = "requires a real inotify/fs-watch backend; run with --ignored locally"]
    fn test_watch_existing_dir_fires_callback() {
        let tmp = tempfile::tempdir().unwrap();
        let counter = StdArc::new(AtomicU32::new(0));
        let cb: DirChangeCallback = {
            let c = StdArc::clone(&counter);
            StdArc::new(move || {
                c.fetch_add(1, Ordering::SeqCst);
            })
        };

        let _watcher = DirWatcher::watch(tmp.path(), cb).unwrap();
        // Let the watcher thread + debouncer spin up.
        std::thread::sleep(Duration::from_millis(300));

        std::fs::write(tmp.path().join("changed.txt"), "x").unwrap();
        // Wait past the debounce window + processing.
        std::thread::sleep(Duration::from_millis(800));

        assert!(
            counter.load(Ordering::SeqCst) >= 1,
            "watcher should fire on file change"
        );
    }
}
