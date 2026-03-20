use std::io::BufRead as _;
use std::io::IsTerminal as _;
use std::sync::Arc;

use orka_core::SecretValue;
use orka_core::config::OrkaConfig;
use orka_core::traits::SecretManager;

use crate::client::Result;

fn create_manager() -> Result<Arc<dyn SecretManager>> {
    let config = OrkaConfig::load(None)?;
    let mgr = orka_secrets::create_secret_manager(&config)?;
    Ok(mgr)
}

/// Read the secret value from stdin.
/// - Interactive TTY: prompts the user and reads one line (value is not echoed
///   in terminals that support it via the OS, though no explicit no-echo is
///   enforced here — use a shell `read -s` wrapper for that).
/// - Pipe: reads the first line from stdin (e.g. `echo -n "$SECRET" | orka secret set path`).
fn read_value_from_stdin() -> Result<String> {
    if std::io::stdin().is_terminal() {
        eprint!("Value: ");
        std::io::Write::flush(&mut std::io::stderr()).ok();
    }
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    // Trim trailing newline(s) only
    while line.ends_with('\n') || line.ends_with('\r') {
        line.pop();
    }
    if line.is_empty() {
        return Err("secret value must not be empty".into());
    }
    Ok(line)
}

pub async fn set(path: &str) -> Result<()> {
    let value = read_value_from_stdin()?;
    let mgr = create_manager()?;
    let secret = SecretValue::new(value.as_bytes().to_vec());
    mgr.set_secret(path, &secret).await?;
    println!("secret '{}' set", path);
    Ok(())
}

pub async fn get(path: &str, reveal: bool) -> Result<()> {
    let mgr = create_manager()?;
    let secret = mgr.get_secret(path).await?;
    if reveal {
        println!("{}", secret.expose_str().unwrap_or("<binary>"));
    } else {
        let raw = secret.expose_str().unwrap_or("");
        if raw.chars().count() <= 4 {
            println!("****");
        } else {
            let prefix: String = raw.chars().take(4).collect();
            println!("{prefix}****");
        }
    }
    Ok(())
}

pub async fn list() -> Result<()> {
    let mgr = create_manager()?;
    let keys = mgr.list_secrets().await?;
    if keys.is_empty() {
        println!("no secrets found");
    } else {
        for key in keys {
            println!("{key}");
        }
    }
    Ok(())
}

pub async fn delete(path: &str) -> Result<()> {
    let mgr = create_manager()?;
    mgr.delete_secret(path).await?;
    println!("secret '{}' deleted", path);
    Ok(())
}
