//! Logging initialization using `tracing`.

use tracing_subscriber::{fmt, EnvFilter};

/// Initialize the global tracing subscriber.
///
/// Call this once at startup, before any other code runs.
/// Log level is controlled by the `RUST_LOG` environment variable,
/// defaulting to `info` if not set.
pub fn init() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    let _ = fmt()
        .with_env_filter(filter)
        .with_target(true)
        .with_thread_ids(false)
        .with_thread_names(false)
        .with_file(true)
        .with_line_number(true)
        .try_init();
}

/// Initialize logging with JSON output (for structured log collection).
pub fn init_json() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    let _ = fmt().json().with_env_filter(filter).try_init();
}
