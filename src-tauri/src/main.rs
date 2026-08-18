//! CaspianFlow binary entry point.
//!
//! Two shapes, selected by the `tauri` feature:
//! - default (no feature): a headless config loader used by tests / CI / the
//!   local-first runtime without a GUI. Keeps `cargo test --lib` free of the
//!   webview system libraries.
//! - `tauri` feature: the desktop GUI window (IPC commands wired in
//!   `caspian_flow::tauri_app`). Run locally with `cargo tauri build --features tauri`.

fn main() -> anyhow::Result<()> {
    #[cfg(feature = "tauri")]
    {
        caspian_flow::tauri_app::run_tauri();
        return Ok(());
    }

    #[cfg(not(feature = "tauri"))]
    {
        use caspian_flow::logging;
        logging::init();
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        rt.block_on(async {
            let manager = caspian_flow::config::ConfigManager::init()
                .await
                .expect("config init failed");
            let settings = manager.settings();
            tracing::info!(
                schema_version = %settings.schema_version,
                models = settings.models.len(),
                "CaspianFlow config loaded"
            );
        });
        Ok(())
    }
}
