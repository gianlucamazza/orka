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
                .map_err(|e| Error::Sandbox(format!("failed to set fuel: {e}")))?;
        }

        let mut linker: Linker<InstanceState> = Linker::new(engine);
        wasmtime_wasi::p1::add_to_linker_sync(&mut linker, |s: &mut InstanceState| &mut s.wasi)
            .map_err(|e| Error::Sandbox(format!("failed to add WASI to linker: {e}")))?;

        let instance = linker
            .instantiate(&mut store, &module.module)
            .map_err(|e| Error::Sandbox(format!("failed to instantiate module: {e}")))?;

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
            .map_err(|e| Error::Sandbox(format!("missing export '{name}': {e}")))
    }

    /// Call a typed export function.
    pub fn call<Params, Results>(&mut self, name: &str, params: Params) -> Result<Results>
    where
        Params: wasmtime::WasmParams,
        Results: wasmtime::WasmResults,
    {
        let f: TypedFunc<Params, Results> = self.get_func(name)?;
        f.call(&mut self.store, params)
            .map_err(|e| Error::Sandbox(format!("call '{name}' failed: {e}")))
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
            .ok_or_else(|| Error::Sandbox("wasm module has no 'memory' export".into()))?;
        let data = memory.data(&self.store);
        let start = ptr as usize;
        let end = start
            .checked_add(len as usize)
            .filter(|&e| e <= data.len())
            .ok_or_else(|| Error::Sandbox("memory read out of bounds".into()))?;
        Ok(data[start..end].to_vec())
    }

    /// Write bytes into the guest's linear memory at `ptr`.
    pub fn write_memory(&mut self, ptr: u32, data: &[u8]) -> Result<()> {
        let memory = self
            .instance
            .get_memory(&mut self.store, "memory")
            .ok_or_else(|| Error::Sandbox("wasm module has no 'memory' export".into()))?;
        memory
            .write(&mut self.store, ptr as usize, data)
            .map_err(|e| Error::Sandbox(format!("memory write failed: {e}")))
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
