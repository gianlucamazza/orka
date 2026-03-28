//! Sandboxed code execution via WASM or subprocess isolation.
//!
//! - [`SandboxExecutor`] — trait for running untrusted code safely
//! - [`WasmSandbox`] — WebAssembly-based executor
//! - [`ProcessSandbox`] — subprocess-based executor with resource limits

#![warn(missing_docs)]

/// Sandbox configuration owned by `orka-sandbox`.
pub mod config;
/// Core trait, types, and limits for sandbox execution.
pub mod executor;
/// Subprocess-based sandbox (Python / Bash).
pub mod process;
/// Skill wrapper that exposes sandbox execution as an agent tool.
pub mod skill;
/// WebAssembly (WASI) sandbox backed by Wasmtime.
#[cfg(feature = "wasm")]
pub mod wasm;

#[cfg(feature = "test-util")]
/// Test-only in-memory helpers and fakes for sandbox integration tests.
pub mod testing;

use std::sync::Arc;

pub use executor::{SandboxExecutor, SandboxLang, SandboxLimits, SandboxRequest, SandboxResult};
use orka_core::Result;
pub use process::ProcessSandbox;
pub use skill::SandboxSkill;
#[cfg(feature = "wasm")]
pub use wasm::WasmSandbox;

pub use crate::config::{SandboxConfig, SandboxLimitsConfig};

/// Create a [`SandboxExecutor`] from the given configuration.
pub fn create_sandbox(config: &SandboxConfig) -> Result<Arc<dyn SandboxExecutor>> {
    match config.backend.as_str() {
        #[cfg(feature = "wasm")]
        "wasm" => Ok(Arc::new(WasmSandbox::new(config)?)),
        _ => Ok(Arc::new(ProcessSandbox::new(config))),
    }
}
