use std::{
    io::{BufRead as _, IsTerminal as _},
    sync::Arc,
};

use orka_config::OrkaConfig;
use orka_core::{SecretValue, traits::SecretManager};

use crate::client::Result;

fn create_manager() -> Result<Arc<dyn SecretManager>> {
    let config = OrkaConfig::load(None)?;
    let mgr = orka_secrets::create_secret_manager(&config.secrets, &config.redis.url)?;
    Ok(mgr)
}

/// Read the secret value.
/// - Interactive TTY: uses a masked password prompt via dialoguer.
/// - Pipe: reads the first line from stdin (e.g. `echo -n "$SECRET" | orka
///   secret set path`).
fn read_value() -> Result<String> {
    if std::io::stdin().is_terminal() {
        let value = dialoguer::Password::new()
            .with_prompt("Secret value")
            .interact()?;
        if value.is_empty() {
            return Err("secret value must not be empty".into());
        }
        return Ok(value);
    }
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    while line.ends_with('\n') || line.ends_with('\r') {
        line.pop();
    }
    if line.is_empty() {
        return Err("secret value must not be empty".into());
    }
    Ok(line)
}

pub async fn set(path: &str) -> Result<()> {
    let value = read_value()?;
    let mgr = create_manager()?;
    let secret = SecretValue::new(value.as_bytes().to_vec());
    mgr.set_secret(path, &secret).await?;
    println!("secret '{path}' set");
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
    println!("secret '{path}' deleted");
    Ok(())
}
