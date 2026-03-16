use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;

use crate::executor::{SandboxExecutor, SandboxRequest, SandboxResult};

/// In-memory sandbox executor that returns canned results, useful for testing.
pub struct InMemorySandbox {
    result: Mutex<SandboxResult>,
}

impl InMemorySandbox {
    /// Create a sandbox that always returns exit_code=0, stdout=b"ok", empty stderr.
    pub fn new() -> Self {
        Self {
            result: Mutex::new(SandboxResult {
                exit_code: 0,
                stdout: b"ok".to_vec(),
                stderr: Vec::new(),
                duration: Duration::ZERO,
            }),
        }
    }

    /// Customize the canned result.
    pub fn with_result(result: SandboxResult) -> Self {
        Self {
            result: Mutex::new(result),
        }
    }
}

impl Default for InMemorySandbox {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SandboxExecutor for InMemorySandbox {
    async fn execute(&self, _req: SandboxRequest) -> orka_core::Result<SandboxResult> {
        let result = self.result.lock().unwrap().clone();
        Ok(result)
    }
}
