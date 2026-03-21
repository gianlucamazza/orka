use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use orka_core::traits::Skill;
use orka_core::{Error, Result, SkillInput, SkillOutput, SkillSchema};
use orka_wasm::{PluginCapabilities, WasmComponent, WasmEngine, WasmLimits};

/// A skill backed by a WASM Component Model plugin.
pub struct WasmPluginSkill {
    name: String,
    description: String,
    schema: SkillSchema,
    component: Arc<WasmComponent>,
    capabilities: PluginCapabilities,
    limits: WasmLimits,
}

impl WasmPluginSkill {
    /// Load a WASM Component plugin from a file.
    ///
    /// Compiles once and probes metadata via the WIT `info()` export.
    pub fn load(
        path: &Path,
        engine: &WasmEngine,
        capabilities: PluginCapabilities,
    ) -> Result<Self> {
        let bytes = std::fs::read(path).map_err(|e| Error::Skill(format!("read wasm: {e}")))?;

        let component = engine.compile_component(&bytes)?;
        let limits = WasmLimits::default();

        let info = component.probe_info(&capabilities, &limits)?;

        let schema: serde_json::Value =
            serde_json::from_str(&info.parameters_schema).unwrap_or_else(|_| serde_json::json!({}));

        Ok(Self {
            name: info.name,
            description: info.description,
            schema: SkillSchema::new(schema),
            component: Arc::new(component),
            capabilities,
            limits,
        })
    }
}

#[async_trait]
impl Skill for WasmPluginSkill {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn schema(&self) -> SkillSchema {
        self.schema.clone()
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        // Convert HashMap<String, Value> → Vec<(String, String)> (JSON-encode each value).
        let args: Vec<(String, String)> = input
            .args
            .into_iter()
            .map(|(k, v)| {
                let encoded = match &v {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                (k, encoded)
            })
            .collect();

        let component = self.component.clone();
        let capabilities = self.capabilities.clone();
        let limits = self.limits.clone();

        let (_, data) =
            tokio::task::spawn_blocking(move || component.run(args, &capabilities, &limits))
                .await
                .map_err(|e| Error::Skill(format!("spawn blocking: {e}")))??;

        // Try to parse the output as JSON; fall back to a plain string.
        let value = serde_json::from_str::<serde_json::Value>(&data)
            .unwrap_or(serde_json::Value::String(data));

        Ok(SkillOutput::new(value))
    }
}

impl WasmPluginSkill {
    /// Run the plugin synchronously (for use inside `spawn_blocking`).
    ///
    /// This is exposed for testing purposes.
    #[cfg(test)]
    pub fn run_sync(&self, args: Vec<(String, String)>) -> Result<String> {
        let (_, data) = self.component.run(args, &self.capabilities, &self.limits)?;
        Ok(data)
    }
}
