use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use orka_core::traits::Skill;
use orka_core::{Error, ErrorCategory, Result, SkillInput, SkillOutput, SkillSchema};

use crate::executor::{SandboxExecutor, SandboxLang, SandboxLimits, SandboxRequest};

/// Skill that executes code in a sandboxed environment.
pub struct SandboxSkill {
    executor: Arc<dyn SandboxExecutor>,
}

impl SandboxSkill {
    /// Create a new sandbox skill backed by the given executor.
    pub fn new(executor: Arc<dyn SandboxExecutor>) -> Self {
        Self { executor }
    }
}

#[async_trait]
impl Skill for SandboxSkill {
    fn name(&self) -> &str {
        "sandbox"
    }

    fn category(&self) -> &str {
        "code"
    }

    fn description(&self) -> &str {
        "Execute code in a sandboxed environment."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "properties": {
                "code": {
                    "type": "string",
                    "description": "The code to execute"
                },
                "language": {
                    "type": "string",
                    "enum": ["python", "bash", "javascript", "wasm"],
                    "description": "The programming language"
                },
                "timeout_secs": {
                    "type": "number",
                    "description": "Optional execution timeout in seconds"
                },
                "stdin": {
                    "type": "string",
                    "description": "Data to pipe to the process via stdin"
                },
                "env": {
                    "type": "object",
                    "additionalProperties": { "type": "string" },
                    "description": "Environment variables to set for the process"
                }
            },
            "required": ["code", "language"]
        }))
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        let code = input
            .args
            .get("code")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::SkillCategorized {
                message: "missing 'code' argument".into(),
                category: ErrorCategory::Input,
            })?;

        let language_str = input
            .args
            .get("language")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::SkillCategorized {
                message: "missing 'language' argument".into(),
                category: ErrorCategory::Input,
            })?;

        let language = match language_str {
            "python" => SandboxLang::Python,
            "bash" => SandboxLang::Bash,
            "javascript" | "js" => SandboxLang::JavaScript,
            "wasm" => SandboxLang::Wasm,
            other => {
                return Err(Error::SkillCategorized {
                    message: format!("unsupported language: {other}"),
                    category: ErrorCategory::Input,
                });
            }
        };

        let mut limits = SandboxLimits::default();
        if let Some(timeout) = input.args.get("timeout_secs").and_then(|v| v.as_u64()) {
            limits.timeout = std::time::Duration::from_secs(timeout);
        }

        let code_bytes = match language_str {
            "wasm" => base64_decode(code).map_err(|e| Error::SkillCategorized {
                message: format!("invalid base64 wasm code: {e}"),
                category: ErrorCategory::Input,
            })?,
            _ => code.as_bytes().to_vec(),
        };

        let stdin = input
            .args
            .get("stdin")
            .and_then(|v| v.as_str())
            .map(|s| s.as_bytes().to_vec());

        let env: HashMap<String, String> = input
            .args
            .get("env")
            .and_then(|v| v.as_object())
            .map(|obj| {
                obj.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default();

        let req = SandboxRequest {
            code: code_bytes,
            language,
            stdin,
            env,
            limits,
        };

        let result = self.executor.execute(req).await?;

        Ok(SkillOutput::new(serde_json::json!({
            "exit_code": result.exit_code,
            "stdout": String::from_utf8_lossy(&result.stdout),
            "stderr": String::from_utf8_lossy(&result.stderr),
            "duration_ms": result.duration.as_millis() as u64,
        })))
    }
}

fn base64_decode(input: &str) -> std::result::Result<Vec<u8>, String> {
    let cleaned: String = input.chars().filter(|c| !c.is_whitespace()).collect();
    let mut out = Vec::with_capacity(cleaned.len() * 3 / 4);

    fn val(c: u8) -> std::result::Result<u8, String> {
        match c {
            b'A'..=b'Z' => Ok(c - b'A'),
            b'a'..=b'z' => Ok(c - b'a' + 26),
            b'0'..=b'9' => Ok(c - b'0' + 52),
            b'+' => Ok(62),
            b'/' => Ok(63),
            _ => Err(format!("invalid base64 character: {}", c as char)),
        }
    }

    let bytes = cleaned.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes.len() - i < 4 {
            return Err("invalid base64 length".into());
        }

        let chunk = &bytes[i..i + 4];
        let (a, b) = (val(chunk[0])?, val(chunk[1])?);

        if chunk[2] == b'=' {
            out.push((a << 2) | (b >> 4));
        } else if chunk[3] == b'=' {
            let c = val(chunk[2])?;
            out.push((a << 2) | (b >> 4));
            out.push(((b & 0xf) << 4) | (c >> 2));
        } else {
            let c = val(chunk[2])?;
            let d = val(chunk[3])?;
            out.push((a << 2) | (b >> 4));
            out.push(((b & 0xf) << 4) | (c >> 2));
            out.push(((c & 0x3) << 6) | d);
        }

        i += 4;
    }

    Ok(out)
}
