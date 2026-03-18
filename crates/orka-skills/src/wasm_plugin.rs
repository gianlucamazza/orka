use std::path::Path;

use async_trait::async_trait;
use orka_core::traits::Skill;
use orka_core::{Error, Result, SkillInput, SkillOutput, SkillSchema};
use wasmtime::*;
use wasmtime_wasi::WasiCtxBuilder;
use wasmtime_wasi::p1::WasiP1Ctx;

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

struct PluginState {
    wasi: WasiP1Ctx,
}

/// A skill backed by a WASM plugin module.
pub struct WasmPluginSkill {
    name: String,
    description: String,
    schema: SkillSchema,
    engine: Engine,
    module_bytes: Vec<u8>,
}

impl WasmPluginSkill {
    /// Load a WASM plugin from a file, calling its `orka_plugin_info` export to
    /// extract metadata.
    pub fn load(path: &Path) -> Result<Self> {
        let module_bytes =
            std::fs::read(path).map_err(|e| Error::Skill(format!("read wasm: {e}")))?;

        let engine = Engine::default();
        let module = Module::new(&engine, &module_bytes)
            .map_err(|e| Error::Skill(format!("compile wasm: {e}")))?;

        let mut linker: Linker<PluginState> = Linker::new(&engine);
        wasmtime_wasi::p1::add_to_linker_sync(&mut linker, |s: &mut PluginState| &mut s.wasi)
            .map_err(|e| Error::Skill(format!("link wasi: {e}")))?;

        let wasi = WasiCtxBuilder::new().build_p1();
        let mut store = Store::new(&engine, PluginState { wasi });

        let instance = linker
            .instantiate(&mut store, &module)
            .map_err(|e| Error::Skill(format!("instantiate wasm: {e}")))?;

        // Call orka_plugin_info() -> u64 (packed ptr << 32 | len)
        let info_fn = instance
            .get_typed_func::<(), i64>(&mut store, "orka_plugin_info")
            .map_err(|e| Error::Skill(format!("get orka_plugin_info: {e}")))?;

        let packed = info_fn
            .call(&mut store, ())
            .map_err(|e| Error::Skill(format!("call orka_plugin_info: {e}")))?;

        let ptr = (packed >> 32) as u32;
        let len = (packed & 0xFFFF_FFFF) as u32;

        let memory = instance
            .get_memory(&mut store, "memory")
            .ok_or_else(|| Error::Skill("wasm module has no memory export".into()))?;

        let data = memory.data(&store);
        let start = ptr as usize;
        let end = start + len as usize;
        if end > data.len() {
            return Err(Error::Skill("plugin info pointer out of bounds".into()));
        }

        let info: PluginInfo = serde_json::from_slice(&data[start..end])
            .map_err(|e| Error::Skill(format!("parse plugin info: {e}")))?;

        let schema: serde_json::Value =
            serde_json::from_str(&info.parameters_schema).unwrap_or_else(|_| serde_json::json!({}));

        Ok(Self {
            name: info.name,
            description: info.description,
            schema: SkillSchema::new(schema),
            engine,
            module_bytes,
        })
    }

    /// Create a fresh WASM instance and execute the plugin with the given input JSON.
    fn run_execute(&self, input_json: &[u8]) -> Result<serde_json::Value> {
        let module = Module::new(&self.engine, &self.module_bytes)
            .map_err(|e| Error::Skill(format!("compile wasm: {e}")))?;

        let mut linker: Linker<PluginState> = Linker::new(&self.engine);
        wasmtime_wasi::p1::add_to_linker_sync(&mut linker, |s: &mut PluginState| &mut s.wasi)
            .map_err(|e| Error::Skill(format!("link wasi: {e}")))?;

        let wasi = WasiCtxBuilder::new().build_p1();
        let mut store = Store::new(&self.engine, PluginState { wasi });

        let instance = linker
            .instantiate(&mut store, &module)
            .map_err(|e| Error::Skill(format!("instantiate wasm: {e}")))?;

        // Allocate memory in the guest for the input
        let alloc_fn = instance
            .get_typed_func::<i32, i32>(&mut store, "orka_alloc")
            .map_err(|e| Error::Skill(format!("get orka_alloc: {e}")))?;

        let input_len = input_json.len() as i32;
        let input_ptr = alloc_fn
            .call(&mut store, input_len)
            .map_err(|e| Error::Skill(format!("call orka_alloc: {e}")))?;

        // Write input into guest memory
        let memory = instance
            .get_memory(&mut store, "memory")
            .ok_or_else(|| Error::Skill("wasm module has no memory export".into()))?;

        memory
            .write(&mut store, input_ptr as usize, input_json)
            .map_err(|e| Error::Skill(format!("write input to wasm memory: {e}")))?;

        // Call orka_plugin_execute(ptr, len) -> u64
        let exec_fn = instance
            .get_typed_func::<(i32, i32), i64>(&mut store, "orka_plugin_execute")
            .map_err(|e| Error::Skill(format!("get orka_plugin_execute: {e}")))?;

        let packed = exec_fn
            .call(&mut store, (input_ptr, input_len))
            .map_err(|e| Error::Skill(format!("call orka_plugin_execute: {e}")))?;

        let result_ptr = (packed >> 32) as u32;
        let result_len = (packed & 0xFFFF_FFFF) as u32;

        let data = memory.data(&store);
        let start = result_ptr as usize;
        let end = start + result_len as usize;
        if end > data.len() {
            return Err(Error::Skill("plugin result pointer out of bounds".into()));
        }

        // The guest returns Result<PluginOutput, String> as JSON
        let result: std::result::Result<serde_json::Value, String> =
            serde_json::from_slice(&data[start..end])
                .map_err(|e| Error::Skill(format!("parse plugin result: {e}")))?;

        match result {
            Ok(value) => {
                // value is {"data": "..."}, extract the data field
                if let Some(data_str) = value.get("data").and_then(|v| v.as_str()) {
                    // Try to parse the data string as JSON, otherwise wrap as string
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
        // Convert SkillInput (HashMap<String, Value>) to PluginInput (HashMap<String, Value>)
        let plugin_input = PluginInput { args: input.args };

        let input_json = serde_json::to_vec(&plugin_input)
            .map_err(|e| Error::Skill(format!("serialize input: {e}")))?;

        // wasmtime operations are synchronous, run on a blocking thread
        let module_bytes = self.module_bytes.clone();
        let engine = self.engine.clone();
        let name = self.name.clone();

        let result = tokio::task::spawn_blocking(move || {
            let skill = WasmPluginSkill {
                name,
                description: String::new(),
                schema: SkillSchema::new(serde_json::json!({})),
                engine,
                module_bytes,
            };
            skill.run_execute(&input_json)
        })
        .await
        .map_err(|e| Error::Skill(format!("spawn blocking: {e}")))?;

        Ok(SkillOutput::new(result?))
    }
}
