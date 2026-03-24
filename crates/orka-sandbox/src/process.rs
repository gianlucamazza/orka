use std::time::Instant;

use async_trait::async_trait;
use nix::sys::resource::{Resource, setrlimit};
use orka_core::{Error, Result, config::SandboxConfig};
use tempfile::NamedTempFile;
use tokio::io::AsyncWriteExt;
use tracing::debug;

use crate::executor::{SandboxExecutor, SandboxLang, SandboxLimits, SandboxRequest, SandboxResult};

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
        let (cmd_name, ext, extra_args) = match req.language {
            SandboxLang::Python => ("python3", ".py", vec![]),
            SandboxLang::Bash => ("bash", ".sh", vec![]),
            SandboxLang::JavaScript => {
                // Prefer Deno for built-in permission sandboxing; fall back to Node.
                if which_js_runtime() == JsRuntime::Deno {
                    (
                        "deno",
                        ".js",
                        vec![
                            "run",
                            "--deny-net",
                            "--deny-env",
                            "--deny-write",
                            "--deny-run",
                        ],
                    )
                } else {
                    ("node", ".js", vec![])
                }
            }
            SandboxLang::Wasm => {
                return Err(Error::sandbox_msg("process sandbox does not support WASM"));
            }
        };

        // Write code to a temp file.
        let tmp = NamedTempFile::with_suffix(ext)
            .map_err(|e| Error::sandbox(e, "failed to create temp file"))?;
        tokio::fs::write(tmp.path(), &req.code)
            .await
            .map_err(|e| Error::sandbox(e, "failed to write temp file"))?;

        let mut command = tokio::process::Command::new(cmd_name);
        command.args(&extra_args);
        command.arg(tmp.path());
        command.stdout(std::process::Stdio::piped());
        command.stderr(std::process::Stdio::piped());

        // Apply resource limits in the child process via pre_exec (Linux only).
        // pre_exec runs after fork() and before exec(), so limits apply only to
        // the child. RLIMIT_AS caps virtual address space (closest to RSS memory).
        let mem_limit = req.limits.max_memory_bytes as u64;
        // SAFETY: pre_exec closure runs in the forked child between fork and exec.
        // Only async-signal-safe operations should be used; setrlimit is safe here.
        unsafe {
            command.pre_exec(move || {
                setrlimit(Resource::RLIMIT_AS, mem_limit, mem_limit)
                    .map_err(std::io::Error::other)?;
                Ok(())
            });
        }
        debug!(max_memory_bytes = mem_limit, "process sandbox resource limits applied");

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

#[derive(PartialEq)]
enum JsRuntime {
    Deno,
    Node,
}

/// Detect which JavaScript runtime is available. Prefers Deno for its built-in
/// sandboxing.
fn which_js_runtime() -> JsRuntime {
    if std::process::Command::new("deno")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
    {
        JsRuntime::Deno
    } else {
        JsRuntime::Node
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

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
