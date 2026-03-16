use std::time::Instant;

use async_trait::async_trait;
use orka_core::config::SandboxConfig;
use orka_core::{Error, Result};
use wasmtime::{Engine, Linker, Module, Store, StoreLimits, StoreLimitsBuilder};
use wasmtime_wasi::preview1::WasiP1Ctx;
use wasmtime_wasi::WasiCtxBuilder;

use crate::executor::{SandboxExecutor, SandboxLang, SandboxLimits, SandboxRequest, SandboxResult};

/// WASM-based sandbox executor using wasmtime.
pub struct WasmSandbox {
    engine: Engine,
    _default_limits: SandboxLimits,
}

struct WasmState {
    wasi: WasiP1Ctx,
    limits: StoreLimits,
}

impl WasmSandbox {
    pub fn new(config: &SandboxConfig) -> Self {
        let mut wasm_config = wasmtime::Config::new();
        wasm_config.consume_fuel(true);

        let engine = Engine::new(&wasm_config).expect("failed to create wasmtime engine");

        Self {
            engine,
            _default_limits: SandboxLimits {
                timeout: std::time::Duration::from_secs(config.limits.timeout_secs),
                max_memory_bytes: config.limits.max_memory_bytes,
                max_output_bytes: config.limits.max_output_bytes,
            },
        }
    }
}

#[async_trait]
impl SandboxExecutor for WasmSandbox {
    async fn execute(&self, req: SandboxRequest) -> Result<SandboxResult> {
        match req.language {
            SandboxLang::Wasm => {}
            _ => {
                return Err(Error::Sandbox(
                    "WASM sandbox only supports WASM modules".into(),
                ));
            }
        }

        let engine = self.engine.clone();
        let code = req.code.clone();
        let max_memory = req.limits.max_memory_bytes;
        let max_output = req.limits.max_output_bytes;
        let timeout = req.limits.timeout;
        let stdin_data = req.stdin.clone();
        let env_vars: Vec<(String, String)> = req.env.into_iter().collect();

        let result = tokio::time::timeout(
            timeout,
            tokio::task::spawn_blocking(move || {
                run_wasm_sync(
                    &engine, &code, max_memory, max_output, stdin_data, &env_vars,
                )
            }),
        )
        .await
        .map_err(|_| Error::Sandbox("WASM execution timed out".into()))?
        .map_err(|e| Error::Sandbox(format!("WASM task failed: {e}")))?;

        result
    }
}

fn run_wasm_sync(
    engine: &Engine,
    code: &[u8],
    max_memory: usize,
    max_output: usize,
    stdin_data: Option<Vec<u8>>,
    env_vars: &[(String, String)],
) -> Result<SandboxResult> {
    let start = Instant::now();

    let module = Module::new(engine, code)
        .map_err(|e| Error::Sandbox(format!("failed to compile WASM module: {e}")))?;

    // Build captured stdout/stderr pipes.
    let stdout_pipe = wasmtime_wasi::pipe::MemoryOutputPipe::new(max_output);
    let stderr_pipe = wasmtime_wasi::pipe::MemoryOutputPipe::new(max_output);

    let mut wasi_builder = WasiCtxBuilder::new();
    wasi_builder.stdout(stdout_pipe.clone());
    wasi_builder.stderr(stderr_pipe.clone());

    if let Some(data) = stdin_data {
        wasi_builder.stdin(wasmtime_wasi::pipe::MemoryInputPipe::new(data));
    }

    for (k, v) in env_vars {
        wasi_builder.env(k, v);
    }

    let store_limits = StoreLimitsBuilder::new().memory_size(max_memory).build();

    let wasi = wasi_builder.build_p1();

    let state = WasmState {
        wasi,
        limits: store_limits,
    };

    let mut store = Store::new(engine, state);
    store.limiter(|s| &mut s.limits);
    store
        .set_fuel(1_000_000_000)
        .map_err(|e| Error::Sandbox(format!("failed to set fuel: {e}")))?;

    let mut linker: Linker<WasmState> = Linker::new(engine);
    wasmtime_wasi::preview1::add_to_linker_sync(&mut linker, |s: &mut WasmState| &mut s.wasi)
        .map_err(|e| Error::Sandbox(format!("failed to add WASI to linker: {e}")))?;

    let instance = linker
        .instantiate(&mut store, &module)
        .map_err(|e| Error::Sandbox(format!("failed to instantiate module: {e}")))?;

    let start_fn = instance
        .get_typed_func::<(), ()>(&mut store, "_start")
        .map_err(|e| Error::Sandbox(format!("missing _start function: {e}")))?;

    let exit_code = match start_fn.call(&mut store, ()) {
        Ok(()) => 0,
        Err(e) => {
            // Check if it's a WASI proc_exit.
            if let Some(exit) = e.downcast_ref::<wasmtime_wasi::I32Exit>() {
                exit.0
            } else {
                tracing::warn!(%e, "WASM execution error");
                1
            }
        }
    };

    let duration = start.elapsed();

    // Drop the store to release WASI's references to the pipes.
    drop(store);

    let mut stdout = stdout_pipe.contents().to_vec();
    let mut stderr = stderr_pipe.contents().to_vec();
    stdout.truncate(max_output);
    stderr.truncate(max_output);

    Ok(SandboxResult {
        exit_code,
        stdout,
        stderr,
        duration,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[tokio::test]
    async fn test_wasm_hello() {
        let config = SandboxConfig::default();
        let sandbox = WasmSandbox::new(&config);

        let wat = r#"(module
            (import "wasi_snapshot_preview1" "fd_write" (func $fd_write (param i32 i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "hello\n")
            (func (export "_start")
                ;; iov at offset 100: ptr=0, len=6
                (i32.store (i32.const 100) (i32.const 0))
                (i32.store (i32.const 104) (i32.const 6))
                ;; fd_write(stdout=1, iovs=100, iovs_len=1, nwritten=200)
                (drop (call $fd_write (i32.const 1) (i32.const 100) (i32.const 1) (i32.const 200)))
            )
        )"#;

        let req = SandboxRequest {
            code: wat.as_bytes().to_vec(),
            language: SandboxLang::Wasm,
            stdin: None,
            env: HashMap::new(),
            limits: SandboxLimits::default(),
        };

        let result = sandbox.execute(req).await.unwrap();
        assert_eq!(result.exit_code, 0);
        assert_eq!(String::from_utf8_lossy(&result.stdout), "hello\n");
    }
}
