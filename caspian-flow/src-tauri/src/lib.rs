//! CaspianFlow — Local-first AI agent runtime.
//!
//! Library entry point exposing all core modules.

#![allow(clippy::field_reassign_with_default)]

pub mod commands;
pub mod config;
pub mod guardian;
pub mod hot_reload;
pub mod knowledge;
pub mod theme;
pub mod logging;
pub mod router;
pub mod session;
pub mod skill;
pub mod startup;
pub mod types;
pub mod workflow;
pub mod memory;
pub mod package;
pub mod self_healing;

// Optional jemalloc global allocator (P35) — gated behind the `jemalloc`
// feature so the default build (and CI `cargo test --lib`) is unaffected.
#[cfg(feature = "jemalloc")]
pub mod allocator;

// Tauri GUI runtime — gated behind the `tauri` feature so the default lib/test
// build (and CI `cargo test --lib`) stays free of the webview system deps.
// Enable with `cargo build --features tauri` / `cargo tauri build`.
#[cfg(feature = "tauri")]
pub mod tauri_app;

// Auto-updater IPC (P33) — same `tauri` feature gate as above.
#[cfg(feature = "tauri")]
pub mod updater;
