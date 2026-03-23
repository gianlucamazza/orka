# Skill Development Guide

Orka supports three kinds of skills:

| Kind         | Implementation         | Use case                                                |
| ------------ | ---------------------- | ------------------------------------------------------- |
| **Built-in** | Rust, compiled-in      | OS, web, HTTP, RAG, scheduler, memory                   |
| **WASM**     | Rust/C/other → `.wasm` | Isolated plugins with explicit capabilities             |
| **Soft**     | Markdown (`SKILL.md`)  | Instruction-based behaviour injected into system prompt |

---

## Built-in Skills

Built-in skills implement the `Skill` trait from `orka-core`:

```rust
use async_trait::async_trait;
use orka_core::{Result, SkillInput, SkillOutput};
use orka_core::traits::Skill;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, JsonSchema)]
struct MySkillArgs {
    /// The message to echo.
    message: String,
}

pub struct MySkill;

#[async_trait]
impl Skill for MySkill {
    fn name(&self) -> &'static str {
        "my_skill"
    }

    fn description(&self) -> &'static str {
        "Echoes the input message. Use when the user asks to echo text."
    }

    fn schema(&self) -> serde_json::Value {
        orka_core::json_schema::<MySkillArgs>()
    }

    async fn call(&self, input: SkillInput) -> Result<SkillOutput> {
        let args: MySkillArgs = serde_json::from_value(input.args)?;
        Ok(SkillOutput::text(args.message))
    }
}
```

Register it in the skill registry:

```rust
registry.register(Arc::new(MySkill));
```

### SkillInput

| Field     | Type                        | Description                         |
| --------- | --------------------------- | ----------------------------------- |
| `args`    | `serde_json::Value`         | Arguments from the LLM tool call    |
| `context` | `Option<Arc<SkillContext>>` | Session, memory, event sink, config |

Access the event sink to emit domain events:

```rust
if let Some(ctx) = &input.context {
    if let Some(sink) = &ctx.event_sink {
        sink.emit(DomainEvent::new(DomainEventKind::SkillInvoked { .. })).await;
    }
}
```

### SkillOutput

```rust
SkillOutput::text("plain string result")
SkillOutput::json(serde_json::json!({"key": "value"}))
SkillOutput::error("something went wrong")
```

---

## WASM Module Plugins

WASM module plugins use the stable C-ABI interface from `orka-plugin-sdk`.

### 1. Add the SDK

```toml
[dependencies]
orka-plugin-sdk = { path = "../../sdk/orka-plugin-sdk" }
```

### 2. Implement the plugin

```rust
use orka_plugin_sdk::{plugin_main, PluginInput, PluginOutput};

plugin_main!(run);

fn run(input: PluginInput) -> PluginOutput {
    let msg = input.args.get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("(no message)");
    PluginOutput::ok(msg)
}
```

### 3. Build for WASM

```bash
cargo build --release --target wasm32-unknown-unknown
```

### 4. Configure

```toml
[plugins]
dir = "./plugins"

[plugins.capabilities.my-plugin]
env = []          # env var names to expose
fs  = []          # host paths to pre-open
network = false   # allow outbound TCP/UDP
```

Place the `.wasm` file in `plugins/`. The file name (without `.wasm`) becomes
the skill name.

---

## WASM Component Model Plugins

The Component Model SDK (`orka-plugin-sdk-component`) uses WIT interface
definitions for a type-safe, language-agnostic ABI.

### WIT interface

```wit
// wit/orka-plugin.wit
package orka:plugin@0.1.0;

world plugin {
  import wasi:cli/environment@0.2.0;
  export run: func(input: plugin-input) -> plugin-output;
}
```

### Implement in Rust

```rust
// src/lib.rs
use orka_plugin_sdk_component::bindings::{PluginInput, PluginOutput, export_plugin};

struct MyPlugin;

impl export_plugin::Guest for MyPlugin {
    fn run(input: PluginInput) -> PluginOutput {
        PluginOutput {
            ok: true,
            data: input.args.get("message").cloned().unwrap_or_default(),
            error: None,
        }
    }
}

export_plugin!(MyPlugin);
```

### Build

```bash
cargo build --release --target wasm32-wasip2
```

---

## Soft Skills (SKILL.md)

Soft skills are **instruction-based** — they do not execute code. When activated,
their markdown body is injected into the LLM system prompt.

### Directory structure

```
skills/
└── code-review/
    └── SKILL.md
```

### SKILL.md format

```markdown
---
name: code-review
description: Reviews code for quality, security, and style. Use when the user asks for a code review.
tags: [development, quality]
---

## Code Review Instructions

When asked to review code:

1. Check for security vulnerabilities (OWASP top 10).
2. Identify performance bottlenecks.
3. Suggest idiomatic improvements for the language.
4. Always provide actionable feedback with examples.
```

**Frontmatter fields:**

| Field         | Required | Description                                                |
| ------------- | -------- | ---------------------------------------------------------- |
| `name`        | yes      | Unique kebab-case identifier (max 64 chars)                |
| `description` | yes      | What the skill does and **when to use it** (LLM sees this) |
| `tags`        | no       | Optional grouping tags                                     |

### Configuration

```toml
[soft_skills]
dir = "./skills"   # directory containing SKILL.md subdirectories
```

### How it works

1. The agent receives a user message.
2. Orka asks the LLM which soft skills are relevant (selection prompt).
3. The LLM replies with a JSON array of skill names: `["code-review"]`.
4. The body of each selected skill is injected into the system prompt as
   `## Active Skills` section.
5. The main LLM call runs with the enriched prompt.

---

## Evaluating Skills

`orka-eval` is a dedicated framework for testing skills offline using declarative TOML scenario files. It lets you write assertions against skill output, timing, and success/failure without running a live LLM.

For the full reference—including the scenario file format, all assertion fields, programmatic usage, and CI integration—see the dedicated **[Evaluation Framework Guide](eval-guide.md)**.

---

## Disabling Skills

```toml
[tools]
disabled = ["shell_exec", "package_install"]
```

Skills in this list are removed from the registry at startup.
