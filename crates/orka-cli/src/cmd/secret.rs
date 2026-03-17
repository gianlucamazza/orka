use orka_core::SecretValue;
use orka_core::config::OrkaConfig;
use orka_core::traits::SecretManager;
use orka_secrets::RedisSecretManager;

use crate::client::Result;

fn create_manager() -> Result<RedisSecretManager> {
    let config = OrkaConfig::load(None)?;
    let mgr = RedisSecretManager::new(&config.redis.url)?;
    Ok(mgr)
}

pub async fn set(path: &str, value: &str) -> Result<()> {
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
        if raw.len() <= 4 {
            println!("****");
        } else {
            println!("{}****", &raw[..4]);
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
