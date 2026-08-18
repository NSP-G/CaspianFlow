// Tauri v2 build script. Only runs its code when the `tauri` feature is on;
// feature-off builds (default `cargo test --lib`, headless sandbox) are no-ops
// and never require the webview system libraries.
fn main() {
    #[cfg(feature = "tauri")]
    {
        tauri_build::build();
    }
}
