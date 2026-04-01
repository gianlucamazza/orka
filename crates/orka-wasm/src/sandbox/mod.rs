//! Sandboxed code execution via WASM or subprocess isolation.
//!
//! - [`SandboxExecutor`] — trait for running untrusted code safely
//! - [`WasmSandbox`] — WebAssembly-based executor
//! - [`ProcessSandbox`] — subprocess-based executor with resource limits

/// Sandbox configuration.
pub mod config;
/// Core trait, types, and limits for sandbox execution.
pub mod executor;
/// Subprocess-based sandbox (Python / Bash).
pub mod process;
/// Skill wrapper that exposes sandbox execution as an agent tool.
pub mod skill;
/// WebAssembly (WASI) sandbox backed by Wasmtime.
pub mod wasm;

#[cfg(feature = "test-util")]
/// Test-only in-memory helpers and fakes for sandbox integration tests.
pub mod testing;

use std::sync::Arc;

pub use config::{SandboxConfig, SandboxLimitsConfig};
pub use executor::{SandboxExecutor, SandboxLang, SandboxLimits, SandboxRequest, SandboxResult};
use orka_core::Result;
pub use process::ProcessSandbox;
pub use skill::SandboxSkill;
pub use wasm::WasmSandbox;

/// Create a [`SandboxExecutor`] from the given configuration.
pub fn create_sandbox(config: &SandboxConfig) -> Result<Arc<dyn SandboxExecutor>> {
    match config.backend.as_str() {
        "wasm" => Ok(Arc::new(WasmSandbox::new(config)?)),
        _ => Ok(Arc::new(ProcessSandbox::new(config))),
    }
}
