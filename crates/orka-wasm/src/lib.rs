//! WASM skill engine: loads and executes Orka plugin `.wasm` files.
#![warn(missing_docs)]

/// WASM engine configuration and resource limits.
pub mod config;
/// Core engine: compiles and caches WASM modules.
pub mod engine;
/// Per-invocation WASM instance with memory management.
pub mod instance;
/// Plugin ABI constants and pointer-packing helpers.
pub mod plugin_abi;

pub use config::WasmLimits;
pub use engine::{WasmEngine, WasmModule};
pub use instance::WasmInstance;
