use orka_core::{Error, Result};
use wasmtime::{Engine, Module};

/// Shared wasmtime engine with fuel enabled.
///
/// Clone is cheap — the inner `Engine` is already `Arc`-wrapped by wasmtime.
#[derive(Clone)]
pub struct WasmEngine(pub(crate) Engine);

impl WasmEngine {
    /// Create a new engine with fuel consumption enabled.
    pub fn new() -> Result<Self> {
        let mut cfg = wasmtime::Config::new();
        cfg.consume_fuel(true);
        let engine = Engine::new(&cfg)
            .map_err(|e| Error::Sandbox(format!("failed to create wasmtime engine: {e}")))?;
        Ok(Self(engine))
    }

    /// Pre-compile a WASM module (bytes or WAT text).
    pub fn compile(&self, bytes: &[u8]) -> Result<WasmModule> {
        let module = Module::new(&self.0, bytes)
            .map_err(|e| Error::Sandbox(format!("failed to compile WASM module: {e}")))?;
        Ok(WasmModule { module })
    }
}

/// A compiled WASM module. Cheap to clone, thread-safe, reusable across calls.
#[derive(Clone)]
pub struct WasmModule {
    pub(crate) module: Module,
}
