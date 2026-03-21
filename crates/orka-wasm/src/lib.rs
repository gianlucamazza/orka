//! WASM skill engine: loads and executes Orka plugin `.wasm` files.
#![warn(missing_docs)]

/// WASM Component Model execution (WIT-based, WASI P2).
pub mod component;
/// WASM engine configuration and resource limits.
pub mod config;
/// Core engine: compiles and caches WASM modules and components.
pub mod engine;
/// Per-invocation WASM instance with memory management (for core modules / sandbox).
pub mod instance;

pub use component::{PluginCapabilities, WasmComponent};
pub use config::WasmLimits;
pub use engine::{WasmEngine, WasmModule};
pub use instance::WasmInstance;
