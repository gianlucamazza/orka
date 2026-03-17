use std::sync::Arc;

use orka_core::config::OrkaConfig;
use orka_core::testing::InMemorySecretManager;
use orka_core::traits::SecretManager;

use crate::client::Result;

pub async fn run(config_path: Option<&str>) -> Result<()> {
    let config_path = config_path.map(std::path::Path::new);
    let config = OrkaConfig::load(config_path).ok();

    // Build skill registry
    let mut skills = orka_skills::create_skill_registry();
    skills.register(Arc::new(orka_skills::EchoSkill));

    // Load WASM plugins if configured
    if let Some(ref config) = config
        && let Some(ref plugin_dir) = config.plugins.dir
        && let Ok(plugins) = orka_skills::load_plugins(std::path::Path::new(plugin_dir))
    {
        for plugin in plugins {
            skills.register(plugin);
        }
    }

    let skills = Arc::new(skills);
    let secrets: Arc<dyn SecretManager> = Arc::new(InMemorySecretManager::new());

    let server = Arc::new(orka_mcp::McpServer::new(skills, secrets));
    server.run_stdio().await.map_err(|e| e.into())
}
