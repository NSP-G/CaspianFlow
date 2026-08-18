//! Optional jemalloc global allocator (P35).
//!
//! Enable with `--features jemalloc`. Off by default: the Rust core's heap
//! footprint is sub-100 KB (see `crate::memory`), and the process memory budget
//! is dominated by the Tauri webview, so the platform system allocator is
//! adequate today. jemalloc earns its place once the embedded-model tensors and
//! large session logs create sustained allocation pressure and fragmentation in
//! a long-running desktop process.

#[cfg(feature = "jemalloc")]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;
