# Tutorial: Building a WASM Plugin

Orka supports deep extensibility through WebAssembly (WASM). While built-in Rust skills provide maximum performance and access to the system, WASM plugins allow you to write skills in any language that compiles to WASM components, distribute them securely, and run them inside Orka's isolated WASM sandbox.

This tutorial covers the creation of a simple WASM plugin using the WebAssembly Component Model and Orka's plugin SDK.

## Prerequisites

You'll need the following installed:
*   [Rust](https://rustup.rs/) (latest stable)
*   The `wasm32-wasip2` target: `rustup target add wasm32-wasip2`
*   [`cargo-component`](https://github.com/bytecodealliance/cargo-component): `cargo binstall cargo-component` (or `cargo install cargo-component`)

## Step 1: Initialize the Project

We will use `cargo component` to create a new WASM component library. Let's create a skill called `weather-plugin`.

```bash
cargo component new --lib weather-plugin
cd weather-plugin
```

## Step 2: Add the SDK

Orka provides a WIT SDK for plugins. In your new project, you'll need to define your component against Orka's `world`. Assuming Orka publishes its WIT definitions to an accessible registry or path:

```bash
# Add the dependency in your cargo.toml or via CLI
cargo component add orka:plugin-sdk
```

*(Note: Depending on how the SDK is distributed, you might need to copy the `plugin.wit` file from the Orka repository into a `wit/` folder within your project).*

## Step 3: Implement the Required Traits

Open `src/lib.rs`. A basic Orka component must implement an `execute` function according to the WIT contract. Here is an example of what that integration might look like using the `orka-plugin-sdk`:

```rust
cargo_component_bindings::generate!();

use bindings::exports::orka::plugin::api::{Guest, ExecutionResult};

struct Component;

impl Guest for Component {
    fn execute(input: String) -> ExecutionResult {
        // Here, input is a JSON string passed from the LLM based on your JSON schema.
        
        // Return your result to the LLM
        ExecutionResult::success("The weather is currently 72°F and sunny.".to_string())
    }
}
```

## Step 4: Define the Schema

Orka needs to tell the LLM how to invoke your skill. You generally provide a companion `.json` schema file alongside your `.wasm` binary, or embed it as a custom section depending on Orka's WASM loader mechanism. Orka expects standard JSON schema:

```json
{
  "name": "get_weather",
  "description": "Fetch the current weather for a specific location.",
  "parameters": {
    "type": "object",
    "properties": {
      "location": {
        "type": "string",
        "description": "The city name, e.g. London or New York"
      }
    },
    "required": ["location"]
  }
}
```

## Step 5: Build and Deploy

Build the WebAssembly component:

```bash
cargo component build --release
```

This will produce a `.wasm` file in `target/wasm32-wasip2/release/weather_plugin.wasm`.

Next, move the `.wasm` file and its `.json` schema to Orka's plugins directory. By default, this is defined in `orka.toml`:

```toml
[plugins]
dir = "/path/to/your/plugins"
```

## Step 6: Test Your Plugin

Restart your Orka server (or wait for the hot reloader if enabled for the plugin directory). You can verify the skill is loaded via the CLI:

```bash
orka skill describe get_weather
```

Finally, ask the agent to use your new skill via the chat!

```bash
orka send "What's the weather like in Paris using the new plugin?"
```
