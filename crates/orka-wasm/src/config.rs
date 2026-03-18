use std::time::Duration;

use orka_core::config::SandboxLimitsConfig;

/// Resource limits applied to a single WASM instance.
#[derive(Debug, Clone)]
pub struct WasmLimits {
    /// Max wasmtime fuel units (CPU proxy). `None` = unlimited.
    pub fuel: Option<u64>,
    /// Max linear-memory bytes the module may allocate.
    pub max_memory_bytes: usize,
    /// Max bytes read from stdout/stderr pipes.
    pub max_output_bytes: usize,
    /// Wall-clock timeout for the entire execution.
    pub timeout: Duration,
}

impl Default for WasmLimits {
    fn default() -> Self {
        Self {
            fuel: Some(1_000_000_000),
            max_memory_bytes: 64 * 1024 * 1024,
            max_output_bytes: 1024 * 1024,
            timeout: Duration::from_secs(30),
        }
    }
}

impl From<&SandboxLimitsConfig> for WasmLimits {
    fn from(c: &SandboxLimitsConfig) -> Self {
        Self {
            fuel: Some(1_000_000_000),
            max_memory_bytes: c.max_memory_bytes,
            max_output_bytes: c.max_output_bytes,
            timeout: Duration::from_secs(c.timeout_secs),
        }
    }
}
