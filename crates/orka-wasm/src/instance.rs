use orka_core::{Error, Result};
use wasmtime::{Linker, Store, StoreLimits, StoreLimitsBuilder, TypedFunc};
use wasmtime_wasi::WasiCtxBuilder;
use wasmtime_wasi::p1::WasiP1Ctx;

use crate::config::WasmLimits;
use crate::engine::WasmModule;

struct InstanceState {
    wasi: WasiP1Ctx,
    limits: StoreLimits,
}

/// A live WASM instance backed by a [`Store`].
pub struct WasmInstance {
    store: Store<InstanceState>,
    instance: wasmtime::Instance,
    stdout: wasmtime_wasi::p2::pipe::MemoryOutputPipe,
    stderr: wasmtime_wasi::p2::pipe::MemoryOutputPipe,
}

impl WasmInstance {
    /// Build an instance from a compiled module applying the given limits.
    pub fn build(
        module: &WasmModule,
        limits: &WasmLimits,
        stdin: Option<Vec<u8>>,
        env: &[(String, String)],
    ) -> Result<Self> {
        let engine = module.module.engine();

        let stdout = wasmtime_wasi::p2::pipe::MemoryOutputPipe::new(limits.max_output_bytes);
        let stderr = wasmtime_wasi::p2::pipe::MemoryOutputPipe::new(limits.max_output_bytes);

        let mut wasi_builder = WasiCtxBuilder::new();
        wasi_builder.stdout(stdout.clone());
        wasi_builder.stderr(stderr.clone());
        if let Some(data) = stdin {
            wasi_builder.stdin(wasmtime_wasi::p2::pipe::MemoryInputPipe::new(data));
        }
        for (k, v) in env {
            wasi_builder.env(k, v);
        }

        let store_limits = StoreLimitsBuilder::new()
            .memory_size(limits.max_memory_bytes)
            .build();
        let wasi = wasi_builder.build_p1();

        let mut store = Store::new(
            engine,
            InstanceState {
                wasi,
                limits: store_limits,
            },
        );
        store.limiter(|s| &mut s.limits);

        if let Some(fuel) = limits.fuel {
            store
                .set_fuel(fuel)
                .map_err(|e| Error::sandbox_msg(format!("failed to set fuel: {e}")))?;
        }

        let mut linker: Linker<InstanceState> = Linker::new(engine);
        wasmtime_wasi::p1::add_to_linker_sync(&mut linker, |s: &mut InstanceState| &mut s.wasi)
            .map_err(|e| Error::sandbox_msg(format!("failed to add WASI to linker: {e}")))?;

        let instance = linker
            .instantiate(&mut store, &module.module)
            .map_err(|e| Error::sandbox_msg(format!("failed to instantiate module: {e}")))?;

        Ok(Self {
            store,
            instance,
            stdout,
            stderr,
        })
    }

    /// Look up a typed export function.
    pub fn get_func<Params, Results>(&mut self, name: &str) -> Result<TypedFunc<Params, Results>>
    where
        Params: wasmtime::WasmParams,
        Results: wasmtime::WasmResults,
    {
        self.instance
            .get_typed_func::<Params, Results>(&mut self.store, name)
            .map_err(|e| Error::sandbox_msg(format!("missing export '{name}': {e}")))
    }

    /// Call a typed export function.
    pub fn call<Params, Results>(&mut self, name: &str, params: Params) -> Result<Results>
    where
        Params: wasmtime::WasmParams,
        Results: wasmtime::WasmResults,
    {
        let f: TypedFunc<Params, Results> = self.get_func(name)?;
        f.call(&mut self.store, params)
            .map_err(|e| Error::sandbox_msg(format!("call '{name}' failed: {e}")))
    }

    /// Call `_start` (WASI command entry point), returning the exit code.
    pub fn call_start(&mut self) -> Result<i32> {
        let f: TypedFunc<(), ()> = self.get_func("_start")?;
        match f.call(&mut self.store, ()) {
            Ok(()) => Ok(0),
            Err(e) => {
                if let Some(exit) = e.downcast_ref::<wasmtime_wasi::I32Exit>() {
                    Ok(exit.0)
                } else {
                    tracing::warn!(%e, "WASM execution error");
                    Ok(1)
                }
            }
        }
    }

    /// Read a slice from the guest's linear memory.
    pub fn read_memory(&mut self, ptr: u32, len: u32) -> Result<Vec<u8>> {
        let memory = self
            .instance
            .get_memory(&mut self.store, "memory")
            .ok_or_else(|| Error::sandbox_msg("wasm module has no 'memory' export"))?;
        let data = memory.data(&self.store);
        let start = ptr as usize;
        let end = start
            .checked_add(len as usize)
            .filter(|&e| e <= data.len())
            .ok_or_else(|| Error::sandbox_msg("memory read out of bounds"))?;
        Ok(data[start..end].to_vec())
    }

    /// Write bytes into the guest's linear memory at `ptr`.
    pub fn write_memory(&mut self, ptr: u32, data: &[u8]) -> Result<()> {
        let memory = self
            .instance
            .get_memory(&mut self.store, "memory")
            .ok_or_else(|| Error::sandbox_msg("wasm module has no 'memory' export"))?;
        memory
            .write(&mut self.store, ptr as usize, data)
            .map_err(|e| Error::sandbox_msg(format!("memory write failed: {e}")))
    }

    /// Consume the instance and return the captured stdout/stderr bytes.
    pub fn into_output(self) -> (Vec<u8>, Vec<u8>) {
        drop(self.store);
        (
            self.stdout.contents().to_vec(),
            self.stderr.contents().to_vec(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::WasmEngine;

    fn compile_wat(wat: &[u8]) -> WasmModule {
        let engine = WasmEngine::new().unwrap();
        engine.compile(wat).unwrap()
    }

    #[test]
    fn build_and_call_add_function() {
        let module = compile_wat(
            br#"
            (module
                (func (export "add") (param i32 i32) (result i32)
                    local.get 0
                    local.get 1
                    i32.add
                )
            )
        "#,
        );
        let limits = WasmLimits::default();
        let mut inst = WasmInstance::build(&module, &limits, None, &[]).unwrap();
        let result: i32 = inst.call("add", (3i32, 4i32)).unwrap();
        assert_eq!(result, 7);
    }

    #[test]
    fn call_missing_export_fails() {
        let module = compile_wat(b"(module)");
        let limits = WasmLimits::default();
        let mut inst = WasmInstance::build(&module, &limits, None, &[]).unwrap();
        let result = inst.call::<(), ()>("nonexistent", ());
        assert!(result.is_err());
    }

    #[test]
    fn read_write_memory() {
        let module = compile_wat(
            br#"
            (module
                (memory (export "memory") 1)
            )
        "#,
        );
        let limits = WasmLimits::default();
        let mut inst = WasmInstance::build(&module, &limits, None, &[]).unwrap();

        let data = b"hello wasm";
        inst.write_memory(0, data).unwrap();
        let read_back = inst.read_memory(0, data.len() as u32).unwrap();
        assert_eq!(read_back, data);
    }

    #[test]
    fn read_memory_out_of_bounds() {
        let module = compile_wat(
            br#"
            (module
                (memory (export "memory") 1)
            )
        "#,
        );
        let limits = WasmLimits::default();
        let mut inst = WasmInstance::build(&module, &limits, None, &[]).unwrap();
        // 1 page = 64KiB. Reading past that should fail.
        let result = inst.read_memory(0, 100_000);
        assert!(result.is_err());
    }

    #[test]
    fn into_output_returns_empty_for_no_io() {
        let module = compile_wat(b"(module)");
        let limits = WasmLimits::default();
        let inst = WasmInstance::build(&module, &limits, None, &[]).unwrap();
        let (stdout, stderr) = inst.into_output();
        assert!(stdout.is_empty());
        assert!(stderr.is_empty());
    }
}
