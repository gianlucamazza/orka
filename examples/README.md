# Orka Examples

This directory contains example applications demonstrating how to use the Orka framework.

## Available Examples

### `basic_bot`

A simple Telegram bot that echoes back messages.

**Features demonstrated:**
- Setting up a Telegram adapter
- Using the in-memory message bus
- Creating a custom message handler
- Publishing responses

**Run:**
```bash
export TELEGRAM_BOT_TOKEN="your_bot_token"
cd basic_bot && cargo run
```

### `custom_skill`

Shows how to create custom skills for Orka agents.

**Features demonstrated:**
- Implementing the `Skill` trait
- Defining skill schemas for LLM tool use
- Input validation and error handling
- Async skill execution

**Run:**
```bash
cd custom_skill && cargo run
```

### `wasm_plugin`

Demonstrates loading and executing WASM plugins as skills.

**Features demonstrated:**
- Loading WASM components
- Configuring runtime limits
- Executing sandboxed plugins
- Handling plugin capabilities

**Run:**
```bash
# Build the example plugin first
cd ../sdk/hello-plugin
cargo build --target wasm32-wasip2 --release

# Run the example
cd ../../examples/wasm_plugin
export WASM_PLUGIN_PATH="../../sdk/hello-plugin/target/wasm32-wasip2/release/hello_plugin.wasm"
cargo run
```

### `multi_agent`

Demonstrates building a multi-agent workflow with specialized agents.

**Features demonstrated:**
- Creating an AgentGraph
- Routing between specialized agents
- Collaborative task execution
- Termination policies

**Run:**
```bash
cd multi_agent && cargo run
```

## Running Examples

Each example is a standalone Cargo project. To run an example:

```bash
cd <example_name>
cargo run
```

Some examples may require environment variables or configuration. Check the example's `src/main.rs` for specific requirements.

## Adding New Examples

When adding a new example:

1. Create a new directory with `Cargo.toml` and `src/main.rs`
2. Add a brief description to this README
3. Include usage instructions
4. Add inline documentation in the code
5. Add tests where appropriate
