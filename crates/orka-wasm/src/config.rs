use std::time::Duration;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_limits_are_reasonable() {
        let limits = WasmLimits::default();
        assert_eq!(limits.fuel, Some(1_000_000_000));
        assert_eq!(limits.max_memory_bytes, 64 * 1024 * 1024);
        assert_eq!(limits.max_output_bytes, 1024 * 1024);
        assert_eq!(limits.timeout, Duration::from_secs(30));
    }
}
