//! Auto-update IPC (P33).
//!
//! Compiled only under `--features tauri` (the plugin is gated there in
//! Cargo.toml), so the headless `cargo test --lib` build never pulls the
//! updater crate or its webview dependencies. Registered by `tauri_app.rs`
//! via `.plugin(tauri_plugin_updater::Builder::new().build())` and the two
//! commands below.
//!
//! Configuration lives in `tauri.conf.json` (`plugins.updater`): the
//! `pubkey` must match the private key used to sign release artifacts
//! (`TAURI_SIGNING_PRIVATE_KEY` in CI). `bundle.createUpdaterArtifacts`
//! must be `true` for the updater to have anything to fetch.

#![cfg(feature = "tauri")]

use tauri::AppHandle;
use tauri_plugin_updater::UpdaterExt;

/// Check for a newer release. Returns its version string, or `None` when the
/// running build is already the latest.
#[tauri::command]
pub async fn check_for_update(app: AppHandle) -> Result<Option<String>, String> {
    let update = app
        .updater()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| e.to_string())?;
    Ok(update.map(|u| u.version))
}

/// Download and install the latest update in place. The caller is expected to
/// restart the app afterwards (the updater replaces the executable on disk).
#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    if let Some(update) = app
        .updater()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| e.to_string())?
    {
        update
            .download_and_install(|_event| {
                // Progress is surfaced to the UI through a separate event in
                // a later iteration; for now the closure keeps the download
                // alive without per-chunk side effects.
            })
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}
