use std::time::Instant;

use async_trait::async_trait;
use orka_core::{Error, Result};
use tempfile::NamedTempFile;
use tokio::io::AsyncWriteExt;
use tracing::warn;

use crate::executor::{SandboxExecutor, SandboxLang, SandboxLimits, SandboxRequest, SandboxResult};
use orka_core::config::SandboxConfig;

/// Process-based sandbox executor for Python and Bash.
pub struct ProcessSandbox {
    _default_limits: SandboxLimits,
}

impl ProcessSandbox {
    /// Create a new process sandbox using limits from `config`.
    pub fn new(config: &SandboxConfig) -> Self {
        Self {
            _default_limits: SandboxLimits {
                timeout: std::time::Duration::from_secs(config.limits.timeout_secs),
                max_memory_bytes: config.limits.max_memory_bytes,
                max_output_bytes: config.limits.max_output_bytes,
            },
        }
    }
}

#[async_trait]
impl SandboxExecutor for ProcessSandbox {
    async fn execute(&self, req: SandboxRequest) -> Result<SandboxResult> {
        let (cmd_name, ext) = match req.language {
            SandboxLang::Python => ("python3", ".py"),
            SandboxLang::Bash => ("bash", ".sh"),
            SandboxLang::Wasm => {
                return Err(Error::sandbox_msg("process sandbox does not support WASM"));
            }
        };

        warn!("process sandbox has no memory limiting — dev use only");

        // Write code to a temp file.
        let tmp = NamedTempFile::with_suffix(ext)
            .map_err(|e| Error::sandbox(e, "failed to create temp file"))?;
        tokio::fs::write(tmp.path(), &req.code)
            .await
            .map_err(|e| Error::sandbox(e, "failed to write temp file"))?;

        let mut command = tokio::process::Command::new(cmd_name);
        command.arg(tmp.path());
        command.stdout(std::process::Stdio::piped());
        command.stderr(std::process::Stdio::piped());

        // Set environment variables.
        for (k, v) in &req.env {
            command.env(k, v);
        }

        // Provide stdin if requested.
        if req.stdin.is_some() {
            command.stdin(std::process::Stdio::piped());
        }

        let start = Instant::now();

        let mut child = command
            .spawn()
            .map_err(|e| Error::sandbox(e, "failed to spawn process"))?;

        // Write stdin if provided.
        if let Some(stdin_data) = &req.stdin
            && let Some(mut stdin) = child.stdin.take()
        {
            stdin
                .write_all(stdin_data)
                .await
                .map_err(|e| Error::sandbox(e, "failed to write stdin"))?;
            // Drop to close stdin.
        }

        let timeout = req.limits.timeout;
        let max_output = req.limits.max_output_bytes;

        let output = tokio::time::timeout(timeout, child.wait_with_output())
            .await
            .map_err(|_| Error::sandbox_msg("execution timed out"))?
            .map_err(|e| Error::sandbox(e, "process error"))?;

        let duration = start.elapsed();

        let mut stdout = output.stdout;
        let mut stderr = output.stderr;
        stdout.truncate(max_output);
        stderr.truncate(max_output);

        Ok(SandboxResult {
            exit_code: output.status.code().unwrap_or(-1),
            stdout,
            stderr,
            duration,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[tokio::test]
    async fn test_bash_echo() {
        let config = SandboxConfig::default();
        let sandbox = ProcessSandbox::new(&config);

        let req = SandboxRequest {
            code: b"echo hello".to_vec(),
            language: SandboxLang::Bash,
            stdin: None,
            env: HashMap::new(),
            limits: SandboxLimits::default(),
        };

        let result = sandbox.execute(req).await.unwrap();
        assert_eq!(result.exit_code, 0);
        assert_eq!(String::from_utf8_lossy(&result.stdout).trim(), "hello");
    }
}
