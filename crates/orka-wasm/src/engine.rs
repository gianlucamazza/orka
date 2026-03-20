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
            .map_err(|e| Error::sandbox_msg(format!("failed to create wasmtime engine: {e}")))?;
        Ok(Self(engine))
    }

    /// Pre-compile a WASM module (bytes or WAT text).
    pub fn compile(&self, bytes: &[u8]) -> Result<WasmModule> {
        let module = Module::new(&self.0, bytes)
            .map_err(|e| Error::sandbox_msg(format!("failed to compile WASM module: {e}")))?;
        Ok(WasmModule { module })
    }
}

/// A compiled WASM module. Cheap to clone, thread-safe, reusable across calls.
#[derive(Clone)]
pub struct WasmModule {
    pub(crate) module: Module,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_creates_successfully() {
        let engine = WasmEngine::new();
        assert!(engine.is_ok());
    }

    #[test]
    fn compile_minimal_wat() {
        let engine = WasmEngine::new().unwrap();
        let wat = b"(module)";
        let module = engine.compile(wat);
        assert!(module.is_ok());
    }

    #[test]
    fn compile_invalid_bytes_fails() {
        let engine = WasmEngine::new().unwrap();
        let result = engine.compile(b"not wasm at all");
        assert!(result.is_err());
    }

    #[test]
    fn compile_wat_with_export() {
        let engine = WasmEngine::new().unwrap();
        let wat = br#"
            (module
                (func (export "add") (param i32 i32) (result i32)
                    local.get 0
                    local.get 1
                    i32.add
                )
            )
        "#;
        assert!(engine.compile(wat).is_ok());
    }

    #[test]
    fn engine_clone_is_cheap() {
        let engine = WasmEngine::new().unwrap();
        let cloned = engine.clone();
        // Both should compile successfully — they share the inner Arc
        assert!(engine.compile(b"(module)").is_ok());
        assert!(cloned.compile(b"(module)").is_ok());
    }
}
