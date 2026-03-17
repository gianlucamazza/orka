pub mod executor;
pub mod process;
pub mod skill;
pub mod wasm;

#[cfg(feature = "test-util")]
pub mod testing;

pub use executor::{SandboxExecutor, SandboxLang, SandboxLimits, SandboxRequest, SandboxResult};
pub use process::ProcessSandbox;
pub use skill::SandboxSkill;
pub use wasm::WasmSandbox;

use orka_core::Result;
use orka_core::config::SandboxConfig;
use std::sync::Arc;

pub fn create_sandbox(config: &SandboxConfig) -> Result<Arc<dyn SandboxExecutor>> {
    match config.backend.as_str() {
        "wasm" => Ok(Arc::new(WasmSandbox::new(config)?)),
        _ => Ok(Arc::new(ProcessSandbox::new(config))),
    }
}
