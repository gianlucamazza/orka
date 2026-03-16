use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Language for sandbox execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SandboxLang {
    Wasm,
    Python,
    Bash,
}

/// Resource limits for sandbox execution.
#[derive(Debug, Clone)]
pub struct SandboxLimits {
    pub timeout: Duration,
    pub max_memory_bytes: usize,
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
    pub code: Vec<u8>,
    pub language: SandboxLang,
    pub stdin: Option<Vec<u8>>,
    pub env: HashMap<String, String>,
    pub limits: SandboxLimits,
}

/// Result of sandbox execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxResult {
    pub exit_code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub duration: Duration,
}

/// Trait for sandbox execution backends.
#[async_trait]
pub trait SandboxExecutor: Send + Sync + 'static {
    async fn execute(&self, req: SandboxRequest) -> orka_core::Result<SandboxResult>;
}
