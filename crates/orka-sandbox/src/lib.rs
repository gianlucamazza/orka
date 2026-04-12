//! Sandboxed code execution for Orka agents.
//!
//! Provides [`SandboxExecutor`] backends for running untrusted code in
//! isolation: a subprocess-based backend ([`ProcessSandbox`]) for Python/Bash
//! and an optional WebAssembly backend ([`WasmSandbox`]).

#![warn(missing_docs)]

/// Sandbox configuration.
pub mod config;
/// Core trait, types, and limits for sandbox execution.
pub mod executor;
/// Subprocess-based sandbox (Python / Bash).
pub mod process;
/// Skill wrapper that exposes sandbox execution as an agent tool.
pub mod skill;
/// WebAssembly (WASI) sandbox backed by Wasmtime.
#[cfg(feature = "wasm-backend")]
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
#[cfg(feature = "wasm-backend")]
pub use wasm::WasmSandbox;

/// Create a [`SandboxExecutor`] from the given configuration.
pub fn create_sandbox(config: &SandboxConfig) -> Result<Arc<dyn SandboxExecutor>> {
    #[cfg(feature = "wasm-backend")]
    if config.backend == "wasm" {
        return Ok(Arc::new(WasmSandbox::new(config)?));
    }
    Ok(Arc::new(ProcessSandbox::new(config)))
}
