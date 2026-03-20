use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;

/// Parsed user input action.
#[derive(Debug, PartialEq)]
pub enum InputAction {
    /// `!command` — run via bash
    ShellExec(String),
    /// `!!` — repeat last shell command
    RepeatLast,
    /// `!cd path` — builtin: change CWD
    Builtin(Builtin),
    /// `/quit`, `/help`, `/clear` — local slash commands
    SlashLocal(String),
    /// `/skill`, `/skills`, `/reset`, `/status` — forwarded to server
    SlashServer(String),
    /// Free text → AI agent
    AgentMessage(String),
    /// Empty input
    Empty,
    /// User input error (e.g. malformed builtin)
    Error(String),
}

#[derive(Debug, PartialEq)]
pub enum Builtin {
    Cd(PathBuf),
    Export(String, String),
    Unset(String),
    History,
}

/// Classify a line of user input into the appropriate action.
pub fn classify_input(line: &str) -> InputAction {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return InputAction::Empty;
    }

    // Shell escape: `!`
    if let Some(rest) = trimmed.strip_prefix('!') {
        let rest = rest.trim();
        if rest.is_empty() {
            // Bare `!` with nothing after — treat as agent message
            return InputAction::AgentMessage(trimmed.to_string());
        }
        if rest == "!" {
            return InputAction::RepeatLast;
        }
        // Builtins — require whitespace or end of string after the keyword
        if let Some(path) = rest
            .strip_prefix("cd")
            .filter(|s| s.is_empty() || s.starts_with(char::is_whitespace))
        {
            let path = path.trim();
            let path = if path.is_empty() {
                dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
            } else {
                PathBuf::from(shellexpand(path))
            };
            return InputAction::Builtin(Builtin::Cd(path));
        }
        if let Some(kv) = rest
            .strip_prefix("export")
            .filter(|s| s.is_empty() || s.starts_with(char::is_whitespace))
        {
            let kv = kv.trim();
            if let Some((k, v)) = kv.split_once('=') {
                return InputAction::Builtin(Builtin::Export(
                    k.trim().to_string(),
                    v.trim().to_string(),
                ));
            }
            if kv.is_empty() {
                return InputAction::Error("export: usage: !export KEY=VALUE".to_string());
            }
            return InputAction::Error("export: usage: !export KEY=VALUE".to_string());
        }
        if let Some(key) = rest
            .strip_prefix("unset")
            .filter(|s| s.is_empty() || s.starts_with(char::is_whitespace))
        {
            let key = key.trim();
            if key.is_empty() {
                return InputAction::Error("unset: usage: !unset KEY".to_string());
            }
            return InputAction::Builtin(Builtin::Unset(key.to_string()));
        }
        if rest == "history" {
            return InputAction::Builtin(Builtin::History);
        }
        return InputAction::ShellExec(rest.to_string());
    }

    // Slash commands
    if let Some(rest) = trimmed.strip_prefix('/') {
        let cmd_name = rest.split_whitespace().next().unwrap_or("").to_lowercase();
        return match cmd_name.as_str() {
            "quit" | "exit" | "help" | "clear" | "think" | "feedback" | "history" | "save" => {
                InputAction::SlashLocal(trimmed.to_string())
            }
            _ => InputAction::SlashServer(trimmed.to_string()),
        };
    }

    InputAction::AgentMessage(trimmed.to_string())
}

/// Expand `~` at the start of a path string.
fn shellexpand(s: &str) -> String {
    if let Some(rest) = s.strip_prefix('~')
        && let Some(home) = dirs::home_dir()
    {
        return format!("{}{rest}", home.display());
    }
    s.to_string()
}

/// Execute a shell command and return the exit code.
pub async fn execute_shell(
    cmd: &str,
    cwd: &Path,
    env_overrides: &HashMap<String, String>,
    env_removes: &HashSet<String>,
) -> Option<i32> {
    let mut command = tokio::process::Command::new("bash");
    command
        .arg("-c")
        .arg(cmd)
        .current_dir(cwd)
        .envs(env_overrides)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    for key in env_removes {
        command.env_remove(key);
    }
    let mut child = match command.spawn() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to spawn shell: {e}");
            return Some(127);
        }
    };

    match child.wait().await {
        Ok(status) => status.code(),
        Err(e) => {
            eprintln!("Failed to wait for shell: {e}");
            Some(1)
        }
    }
}

/// Handle a builtin command, mutating CWD/env as needed. Returns a status message.
pub fn handle_builtin(
    builtin: &Builtin,
    cwd: &mut PathBuf,
    env_overrides: &mut HashMap<String, String>,
    env_removes: &mut HashSet<String>,
) -> String {
    match builtin {
        Builtin::Cd(path) => {
            let target = if path.is_absolute() {
                path.clone()
            } else {
                cwd.join(path)
            };
            match target.canonicalize() {
                Ok(real) if real.is_dir() => {
                    // Set PWD explicitly so child processes inherit the correct value
                    env_overrides.insert("PWD".to_string(), real.to_string_lossy().into_owned());
                    *cwd = real;
                    String::new()
                }
                Ok(_) => format!("cd: not a directory: {}", target.display()),
                Err(e) => format!("cd: {}: {e}", target.display()),
            }
        }
        Builtin::Export(k, v) => {
            env_overrides.insert(k.clone(), v.clone());
            String::new()
        }
        Builtin::Unset(k) => {
            env_overrides.remove(k);
            // Track the unset so execute_shell can pass env_remove to child processes
            env_removes.insert(k.clone());
            String::new()
        }
        Builtin::History => {
            println!("Use arrow keys to browse history.");
            String::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_shell_exec() {
        assert_eq!(
            classify_input("!ls -la"),
            InputAction::ShellExec("ls -la".to_string())
        );
    }

    #[test]
    fn classify_repeat_last() {
        assert_eq!(classify_input("!!"), InputAction::RepeatLast);
    }

    #[test]
    fn classify_cd_home() {
        match classify_input("!cd") {
            InputAction::Builtin(Builtin::Cd(_)) => {}
            other => panic!("expected Builtin::Cd, got {other:?}"),
        }
    }

    #[test]
    fn classify_cd_path() {
        assert_eq!(
            classify_input("!cd /tmp"),
            InputAction::Builtin(Builtin::Cd(PathBuf::from("/tmp")))
        );
    }

    #[test]
    fn classify_export() {
        assert_eq!(
            classify_input("!export FOO=bar"),
            InputAction::Builtin(Builtin::Export("FOO".into(), "bar".into()))
        );
    }

    #[test]
    fn classify_unset() {
        assert_eq!(
            classify_input("!unset FOO"),
            InputAction::Builtin(Builtin::Unset("FOO".into()))
        );
    }

    #[test]
    fn classify_history() {
        assert_eq!(
            classify_input("!history"),
            InputAction::Builtin(Builtin::History)
        );
    }

    #[test]
    fn classify_slash_local() {
        assert_eq!(
            classify_input("/quit"),
            InputAction::SlashLocal("/quit".to_string())
        );
        assert_eq!(
            classify_input("/help"),
            InputAction::SlashLocal("/help".to_string())
        );
        assert_eq!(
            classify_input("/clear"),
            InputAction::SlashLocal("/clear".to_string())
        );
        assert_eq!(
            classify_input("/think"),
            InputAction::SlashLocal("/think".to_string())
        );
        assert_eq!(
            classify_input("/feedback good"),
            InputAction::SlashLocal("/feedback good".to_string())
        );
    }

    #[test]
    fn classify_slash_server() {
        assert_eq!(
            classify_input("/skill search query"),
            InputAction::SlashServer("/skill search query".to_string())
        );
        assert_eq!(
            classify_input("/skills"),
            InputAction::SlashServer("/skills".to_string())
        );
    }

    #[test]
    fn classify_agent_message() {
        assert_eq!(
            classify_input("trova i file rust"),
            InputAction::AgentMessage("trova i file rust".to_string())
        );
    }

    #[test]
    fn classify_empty() {
        assert_eq!(classify_input(""), InputAction::Empty);
        assert_eq!(classify_input("   "), InputAction::Empty);
    }

    #[test]
    fn classify_bare_exclamation() {
        // Single `!` with no command — treat as agent message
        assert_eq!(
            classify_input("!"),
            InputAction::AgentMessage("!".to_string())
        );
    }

    #[test]
    fn classify_cdr_is_shell_exec_not_cd() {
        // `!cdr` must not match the `cd` builtin prefix — it's a shell command
        assert_eq!(
            classify_input("!cdr"),
            InputAction::ShellExec("cdr".to_string())
        );
        assert_eq!(
            classify_input("!cdrom"),
            InputAction::ShellExec("cdrom".to_string())
        );
    }

    #[test]
    fn classify_exportfoo_is_shell_exec_not_export() {
        assert_eq!(
            classify_input("!exportfoo=bar"),
            InputAction::ShellExec("exportfoo=bar".to_string())
        );
    }

    #[test]
    fn classify_unsetfoo_is_shell_exec_not_unset() {
        assert_eq!(
            classify_input("!unsetFOO"),
            InputAction::ShellExec("unsetFOO".to_string())
        );
    }

    #[test]
    fn classify_export_without_equals_is_error() {
        assert!(matches!(
            classify_input("!export FOO"),
            InputAction::Error(_)
        ));
    }

    #[test]
    fn classify_bare_export_is_error() {
        assert!(matches!(classify_input("!export"), InputAction::Error(_)));
    }

    #[test]
    fn classify_bare_unset_is_error() {
        assert!(matches!(classify_input("!unset"), InputAction::Error(_)));
    }

    #[test]
    fn handle_cd_to_tmp() {
        let mut cwd = std::env::current_dir().unwrap();
        let mut env = HashMap::new();
        let mut removes = HashSet::new();
        let msg = handle_builtin(
            &Builtin::Cd(PathBuf::from("/tmp")),
            &mut cwd,
            &mut env,
            &mut removes,
        );
        assert!(msg.is_empty());
        assert_eq!(cwd, PathBuf::from("/tmp").canonicalize().unwrap());
        // PWD should be set in env_overrides
        assert!(env.contains_key("PWD"));
    }

    #[tokio::test]
    async fn execute_shell_echo_returns_zero() {
        let cwd = std::env::current_dir().unwrap();
        let code = execute_shell("echo hello", &cwd, &HashMap::new(), &HashSet::new()).await;
        assert_eq!(code, Some(0));
    }

    #[tokio::test]
    async fn execute_shell_false_returns_nonzero() {
        let cwd = std::env::current_dir().unwrap();
        let code = execute_shell("false", &cwd, &HashMap::new(), &HashSet::new()).await;
        assert_ne!(code, Some(0));
    }

    #[test]
    fn shellexpand_tilde_expands_to_home() {
        if let Some(home) = dirs::home_dir() {
            let result = shellexpand("~/foo/bar");
            assert_eq!(result, format!("{}/foo/bar", home.display()));
        }
        // No tilde → unchanged
        assert_eq!(shellexpand("/tmp/test"), "/tmp/test");
    }

    #[test]
    fn handle_export_unset() {
        let mut cwd = std::env::current_dir().unwrap();
        let mut env = HashMap::new();
        let mut removes = HashSet::new();
        handle_builtin(
            &Builtin::Export("TEST_ORKA_VAR".into(), "42".into()),
            &mut cwd,
            &mut env,
            &mut removes,
        );
        assert_eq!(env.get("TEST_ORKA_VAR").unwrap(), "42");
        handle_builtin(
            &Builtin::Unset("TEST_ORKA_VAR".into()),
            &mut cwd,
            &mut env,
            &mut removes,
        );
        assert!(!env.contains_key("TEST_ORKA_VAR"));
        assert!(removes.contains("TEST_ORKA_VAR"));
    }
}
