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

## Running Examples

Each example is a standalone Cargo project. To run an example:

```bash
cd <example_name>
cargo run
```

Some examples may require environment variables or configuration. Check the example's `src/main.rs` for specific requirements.
