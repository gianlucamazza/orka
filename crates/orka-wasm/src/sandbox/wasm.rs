use std::time::Instant;

use async_trait::async_trait;
use orka_core::{Error, Result};

use super::{
    config::SandboxConfig,
    executor::{SandboxExecutor, SandboxLang, SandboxLimits, SandboxRequest, SandboxResult},
};
use crate::{WasmEngine, WasmInstance, WasmLimits};

/// WASM-based sandbox executor using a shared [`WasmEngine`].
pub struct WasmSandbox {
    engine: WasmEngine,
    _default_limits: SandboxLimits,
}

impl WasmSandbox {
    /// Create a new WASM sandbox using limits from `config`.
    pub fn new(config: &SandboxConfig) -> Result<Self> {
        let engine = WasmEngine::new()?;
        Ok(Self {
            engine,
            _default_limits: SandboxLimits {
                timeout: std::time::Duration::from_secs(config.limits.timeout_secs),
                max_memory_bytes: config.limits.max_memory_bytes,
                max_output_bytes: config.limits.max_output_bytes,
            },
        })
    }
}

#[async_trait]
impl SandboxExecutor for WasmSandbox {
    async fn execute(&self, req: SandboxRequest) -> Result<SandboxResult> {
        match req.language {
            SandboxLang::Wasm => {}
            _ => {
                return Err(Error::sandbox_msg(
                    "WASM sandbox only supports WASM modules",
                ));
            }
        }

        let engine = self.engine.clone();
        let code = req.code.clone();
        let limits = WasmLimits {
            fuel: Some(1_000_000_000),
            max_memory_bytes: req.limits.max_memory_bytes,
            max_output_bytes: req.limits.max_output_bytes,
            timeout: req.limits.timeout,
        };
        let stdin_data = req.stdin.clone();
        let env_vars: Vec<(String, String)> = req.env.into_iter().collect();
        let timeout = req.limits.timeout;

        tokio::time::timeout(
            timeout,
            tokio::task::spawn_blocking(move || {
                run_wasm_sync(&engine, &code, &limits, stdin_data, &env_vars)
            }),
        )
        .await
        .map_err(|_| Error::sandbox_msg("WASM execution timed out"))?
        .map_err(|e| Error::sandbox(e, "WASM task failed"))?
    }
}

fn run_wasm_sync(
    engine: &WasmEngine,
    code: &[u8],
    limits: &WasmLimits,
    stdin: Option<Vec<u8>>,
    env: &[(String, String)],
) -> Result<SandboxResult> {
    let start = Instant::now();

    let module = engine.compile(code)?;
    let mut instance = WasmInstance::build(&module, limits, stdin, env)?;
    let exit_code = instance.call_start()?;
    let duration = start.elapsed();

    let max_output = limits.max_output_bytes;
    let (mut stdout, mut stderr) = instance.into_output();
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
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::field_reassign_with_default,
    clippy::default_trait_access,
    clippy::needless_pass_by_value,
    clippy::stable_sort_primitive
)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    #[tokio::test]
    async fn test_wasm_hello() {
        let config = SandboxConfig::default();
        let sandbox = WasmSandbox::new(&config).unwrap();

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
