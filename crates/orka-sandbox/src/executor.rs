use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Language for sandbox execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SandboxLang {
    /// WebAssembly (WASI) module.
    Wasm,
    /// Python 3 script.
    Python,
    /// Bash shell script.
    Bash,
}

/// Resource limits for sandbox execution.
#[derive(Debug, Clone)]
pub struct SandboxLimits {
    /// Maximum wall-clock time before execution is killed.
    pub timeout: Duration,
    /// Maximum memory the sandbox process may allocate (bytes).
    pub max_memory_bytes: usize,
    /// Maximum size of stdout/stderr captured (bytes).
    pub max_output_bytes: usize,
}

impl Default for SandboxLimits {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            max_memory_bytes: 64 * 1024 * 1024,
            max_output_bytes: 1024 * 1024,
        }
    }
}

/// Request to execute code in a sandbox.
#[derive(Debug, Clone)]
pub struct SandboxRequest {
    /// Raw code bytes (source or compiled WASM).
    pub code: Vec<u8>,
    /// Language/runtime to use for execution.
    pub language: SandboxLang,
    /// Optional data to pass on stdin.
    pub stdin: Option<Vec<u8>>,
    /// Environment variables for the sandbox process.
    pub env: HashMap<String, String>,
    /// Resource limits for this execution.
    pub limits: SandboxLimits,
}

/// Result of sandbox execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxResult {
    /// Process exit code (0 = success).
    pub exit_code: i32,
    /// Captured standard output (truncated to `max_output_bytes`).
    pub stdout: Vec<u8>,
    /// Captured standard error (truncated to `max_output_bytes`).
    pub stderr: Vec<u8>,
    /// Wall-clock time for the execution.
    pub duration: Duration,
}

/// Trait for sandbox execution backends.
#[async_trait]
pub trait SandboxExecutor: Send + Sync + 'static {
    /// Execute the given sandbox request and return the result.
    async fn execute(&self, req: SandboxRequest) -> orka_core::Result<SandboxResult>;
}
