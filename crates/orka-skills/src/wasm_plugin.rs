use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use orka_core::traits::Skill;
use orka_core::{Error, Result, SkillInput, SkillOutput, SkillSchema};
use orka_wasm::plugin_abi::{ABI_VERSION, exports, unpack_ptr_len};
use orka_wasm::{WasmEngine, WasmInstance, WasmLimits, WasmModule};
use tracing::warn;

/// Plugin metadata mirroring the guest SDK's `PluginInfo`.
#[derive(Debug, Clone, serde::Deserialize)]
struct PluginInfo {
    name: String,
    description: String,
    parameters_schema: String,
}

/// Plugin input mirroring the guest SDK's `PluginInput`.
#[derive(Debug, serde::Serialize)]
struct PluginInput {
    args: std::collections::HashMap<String, serde_json::Value>,
}

/// A skill backed by a WASM plugin module (ABI v2).
pub struct WasmPluginSkill {
    name: String,
    description: String,
    schema: SkillSchema,
    engine: WasmEngine,
    /// Pre-compiled module — reused across calls.
    module: Arc<WasmModule>,
    limits: WasmLimits,
}

impl WasmPluginSkill {
    /// Load a WASM plugin from a file. Compiles once, calls `orka_plugin_init`.
    pub fn load(path: &Path, engine: &WasmEngine) -> Result<Self> {
        let module_bytes =
            std::fs::read(path).map_err(|e| Error::Skill(format!("read wasm: {e}")))?;

        let module = engine.compile(&module_bytes)?;

        // Build a throw-away instance to read plugin info and run init.
        let limits = WasmLimits::default();
        let mut inst = WasmInstance::build(&module, &limits, None, &[])?;

        // Verify ABI version.
        let abi: i32 = inst.call(exports::ABI_VERSION, ()).unwrap_or(0);
        if abi != ABI_VERSION {
            return Err(Error::Skill(format!(
                "unsupported plugin ABI version {abi} (expected {ABI_VERSION})"
            )));
        }

        // Read plugin metadata.
        let packed: i64 = inst.call(exports::PLUGIN_INFO, ())?;
        let (ptr, len) = unpack_ptr_len(packed);
        let info_bytes = inst.read_memory(ptr, len)?;
        let info: PluginInfo = serde_json::from_slice(&info_bytes)
            .map_err(|e| Error::Skill(format!("parse plugin info: {e}")))?;

        // Run init — log warning on failure but continue.
        let init_rc: i32 = inst.call(exports::PLUGIN_INIT, ()).unwrap_or(1);
        if init_rc != 0 {
            warn!(name = %info.name, "plugin init returned non-zero");
        }

        let schema: serde_json::Value =
            serde_json::from_str(&info.parameters_schema).unwrap_or_else(|_| serde_json::json!({}));

        Ok(Self {
            name: info.name,
            description: info.description,
            schema: SkillSchema::new(schema),
            engine: engine.clone(),
            module: Arc::new(module),
            limits,
        })
    }

    fn run_execute(&self, input_json: &[u8]) -> Result<serde_json::Value> {
        let mut inst = WasmInstance::build(&self.module, &self.limits, None, &[])?;

        // Allocate guest memory for the input.
        let input_len = input_json.len() as i32;
        let input_ptr: i32 = inst.call(exports::ALLOC, input_len)?;

        // Write input.
        inst.write_memory(input_ptr as u32, input_json)?;

        // Call execute.
        let packed: i64 = inst.call(exports::PLUGIN_EXECUTE, (input_ptr, input_len))?;
        let (res_ptr, res_len) = unpack_ptr_len(packed);
        let result_bytes = inst.read_memory(res_ptr, res_len)?;

        // Free the result buffer via orka_dealloc.
        let _ = inst.call::<(i32, i32), ()>(exports::DEALLOC, (res_ptr as i32, res_len as i32));

        let result: std::result::Result<serde_json::Value, String> =
            serde_json::from_slice(&result_bytes)
                .map_err(|e| Error::Skill(format!("parse plugin result: {e}")))?;

        match result {
            Ok(value) => {
                if let Some(data_str) = value.get("data").and_then(|v| v.as_str()) {
                    match serde_json::from_str::<serde_json::Value>(data_str) {
                        Ok(parsed) => Ok(parsed),
                        Err(_) => Ok(serde_json::Value::String(data_str.to_string())),
                    }
                } else {
                    Ok(value)
                }
            }
            Err(e) => Err(Error::Skill(e)),
        }
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
        let plugin_input = PluginInput { args: input.args };
        let input_json = serde_json::to_vec(&plugin_input)
            .map_err(|e| Error::Skill(format!("serialize input: {e}")))?;

        let module = self.module.clone();
        let engine = self.engine.clone();
        let limits = self.limits.clone();
        let name = self.name.clone();
        let description = self.description.clone();
        let schema = self.schema.clone();

        let result = tokio::task::spawn_blocking(move || {
            let skill = WasmPluginSkill {
                name,
                description,
                schema,
                engine,
                module,
                limits,
            };
            skill.run_execute(&input_json)
        })
        .await
        .map_err(|e| Error::Skill(format!("spawn blocking: {e}")))?;

        Ok(SkillOutput::new(result?))
    }
}
