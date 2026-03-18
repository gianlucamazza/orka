# hello-plugin

A minimal example of an Orka WASM plugin.

The plugin exposes a single skill called `hello` that returns a greeting string.

## Building

```bash
# Install the WASM target (once)
rustup target add wasm32-unknown-unknown

# Build the plugin
cargo build --target wasm32-unknown-unknown --release \
    --manifest-path examples/hello-plugin/Cargo.toml

# Output
ls target/wasm32-unknown-unknown/release/hello_plugin.wasm
```

> **Why is this not a workspace member?**
> The plugin must be compiled for `wasm32-unknown-unknown` while all other
> workspace crates target the host architecture. Mixing targets in a single
> workspace requires per-crate target overrides that complicate the build.
> The plugin is kept as a standalone crate that simply references the SDK by path.

## Loading into Orka

Copy the `.wasm` file to the `plugins/` directory configured in `orka.toml`:

```toml
[plugins]
dir = "plugins"
```

Then restart (or send `SIGHUP` if hot-reload is enabled). The server will
discover `hello_plugin.wasm` and register the `hello` skill automatically.

## Calling the skill

Once loaded, the skill appears as a tool available to the LLM. You can also
invoke it directly via the API:

```bash
curl -X POST http://localhost:8080/skills/hello/invoke \
     -H 'Content-Type: application/json' \
     -d '{"name": "Alice"}'
# → {"data": "Hello, Alice!"}
```

## Code walkthrough

```rust
// examples/hello-plugin/src/lib.rs

#[derive(Default)]
struct HelloPlugin;

impl Plugin for HelloPlugin {
    fn info(&self) -> PluginInfo {
        PluginInfo {
            name: "hello".into(),
            description: "A hello world plugin".into(),
            // JSON Schema for the skill's parameters
            parameters_schema: r#"{"type":"object","properties":{"name":{"type":"string"}}}"#.into(),
        }
    }

    fn execute(&self, input: PluginInput) -> Result<PluginOutput, String> {
        let name = input.args.iter()
            .find(|(k, _)| k == "name")
            .map(|(_, v)| v.as_str())
            .unwrap_or("World");
        Ok(PluginOutput { data: format!("Hello, {name}!") })
    }
}

// Generates the WASM FFI exports
orka_plugin_sdk::export_plugin!(HelloPlugin);
```
