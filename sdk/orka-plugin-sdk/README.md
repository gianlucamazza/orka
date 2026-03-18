# orka-plugin-sdk

SDK for building Orka plugins compiled to WebAssembly.

Plugins are `.wasm` files loaded at runtime by the server.
They expose a standardized FFI so the host can discover and invoke them
without any shared memory layout agreement beyond JSON.

## Writing a plugin

```rust
use orka_plugin_sdk::{Plugin, PluginInfo, PluginInput, PluginOutput};

#[derive(Default)]
struct GreetPlugin;

impl Plugin for GreetPlugin {
    fn info(&self) -> PluginInfo {
        PluginInfo {
            name: "greet".into(),
            description: "Returns a greeting".into(),
            parameters_schema: r#"
                {"type":"object","properties":{"name":{"type":"string"}}}
            "#.into(),
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

orka_plugin_sdk::export_plugin!(GreetPlugin);
```

## Building

Plugins must target `wasm32-unknown-unknown` and be compiled as a `cdylib`:

```toml
# Cargo.toml
[lib]
crate-type = ["cdylib"]
```

```bash
rustup target add wasm32-unknown-unknown
cargo build --target wasm32-unknown-unknown --release
# Output: target/wasm32-unknown-unknown/release/my_plugin.wasm
```

The plugin is **not** a workspace member because it uses a different compilation
target (`wasm32-unknown-unknown`) than the host crates (`x86_64-unknown-linux-gnu`).

## Loading

Place the compiled `.wasm` file in the `plugins/` directory configured in `orka.toml`.
The server discovers and loads plugins at startup.

## FFI contract

`export_plugin!` generates three exported functions:

| Symbol                                 | Purpose                                                                                           |
| -------------------------------------- | ------------------------------------------------------------------------------------------------- |
| `orka_plugin_info() -> u64`            | Returns a `(ptr << 32) \| len` encoding pointing to a JSON-serialized `PluginInfo`                |
| `orka_plugin_execute(ptr, len) -> u64` | Accepts a JSON-serialized `PluginInput`, returns a JSON-serialized `Result<PluginOutput, String>` |
| `orka_alloc(len) -> u32`               | Linear-memory allocator for the host to write input bytes into                                    |
