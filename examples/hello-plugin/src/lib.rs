use orka_plugin_sdk::{Plugin, PluginInfo, PluginInput, PluginOutput};

#[derive(Default)]
struct HelloPlugin;

impl Plugin for HelloPlugin {
    fn info(&self) -> PluginInfo {
        PluginInfo {
            name: "hello".into(),
            description: "A hello world plugin".into(),
            parameters_schema: r#"{"type":"object","properties":{"name":{"type":"string"}}}"#
                .into(),
        }
    }

    fn execute(&self, input: PluginInput) -> Result<PluginOutput, String> {
        let name = input
            .args
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("World");
        Ok(PluginOutput {
            data: format!("Hello, {name}!"),
        })
    }
}

orka_plugin_sdk::export_plugin!(HelloPlugin);
