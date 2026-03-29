use orka_core::{Error, Result};
use tracing::debug;
use wasmtime::{Engine, Module};

/// Shared wasmtime engine with fuel and Component Model enabled.
///
/// Clone is cheap — the inner `Engine` is already `Arc`-wrapped by wasmtime.
#[derive(Clone)]
pub struct WasmEngine(pub(crate) Engine);

impl WasmEngine {
    /// Create a new engine with fuel consumption and Component Model enabled.
    pub fn new() -> Result<Self> {
        let mut cfg = wasmtime::Config::new();
        cfg.consume_fuel(true);
        cfg.wasm_component_model(true);
        let engine = Engine::new(&cfg)
            .map_err(|e| Error::sandbox_msg(format!("failed to create wasmtime engine: {e}")))?;
        Ok(Self(engine))
    }

    /// Pre-compile a core WASM module (bytes or WAT text).
    pub fn compile(&self, bytes: &[u8]) -> Result<WasmModule> {
        debug!(bytes_len = bytes.len(), "compiling WASM core module");
        let module = Module::new(&self.0, bytes)
            .map_err(|e| Error::sandbox_msg(format!("failed to compile WASM module: {e}")))?;
        debug!("WASM core module compiled successfully");
        Ok(WasmModule { module })
    }

    /// Pre-compile a WASM Component (Component Model binary).
    pub fn compile_component(&self, bytes: &[u8]) -> Result<crate::component::WasmComponent> {
        crate::component::WasmComponent::compile(self, bytes)
    }
}

/// A compiled WASM module. Cheap to clone, thread-safe, reusable across calls.
#[derive(Clone)]
pub struct WasmModule {
    pub(crate) module: Module,
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::field_reassign_with_default,
    clippy::default_trait_access,
    clippy::needless_pass_by_value,
    clippy::stable_sort_primitive
)]
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
