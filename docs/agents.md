# Orka Prompt Architecture

This document describes the template-based prompt architecture introduced in Orka.

## Overview

The prompt system follows modern architectural patterns:

- **Template Engine**: Handlebars-based rendering with hot-reload
- **Pipeline Pattern**: Composable sections for prompt construction
- **Context Providers**: Dependency injection for dynamic data
- **Configuration over Code**: TOML-based configuration

## Architectural Principles

### 1. Separation of Concerns
- **Templates**: Pure presentation logic (Handlebars)
- **Context Providers**: Data fetching and transformation
- **Pipeline**: Orchestration and composition
- **Configuration**: Static behavior tuning

### 2. Dependency Injection
Components receive their dependencies rather than creating them:
```rust
// Good: Dependencies injected
let provider = ExperienceContextProvider::new(experience_service);
let coordinator = ContextCoordinator::new(base_context)
    .with_provider(Box::new(provider));

// Bad: Hardcoded dependencies (workaround)
let principles = experience_service.retrieve_principles(...).await;
```

### 3. Trait-Based Abstraction
Components depend on traits, not concrete types:
```rust
pub trait ContextProvider: Send + Sync {
    async fn provide(&self, ctx: &SessionContext) -> Result<Value>;
}
```

## Architecture Components

### 1. Template Engine (`orka-prompts`)

The `orka-prompts` crate provides the core template functionality:

```
crates/orka-prompts/
├── src/
│   ├── template/        # Handlebars-based template engine
│   │   ├── engine.rs    # Template rendering
│   │   ├── registry.rs  # Template storage with hot-reload
│   │   └── loader.rs    # File system loading
│   ├── pipeline/        # Prompt construction pipeline
│   │   ├── builder.rs   # SystemPromptPipeline
│   │   ├── section.rs   # Section trait implementations
│   │   └── config.rs    # Pipeline configuration
│   └── defaults.rs      # Centralized constants
└── templates/           # Built-in templates
    ├── system/          # System-level prompts (reflection, distillation)
    └── sections/        # Prompt sections (persona, tools, workspace, etc.)
```

### 2. Template Registry

Templates are stored in a thread-safe registry with hot-reload support:

```rust
use orka_prompts::template::TemplateRegistry;

let registry = TemplateRegistry::new();
registry.register_inline("greeting", "Hello, {{name}}!").await?;
registry.register_file("custom", path).await?;

let result = registry.render("greeting", &json!({"name": "World"})).await?;
```

### 3. System Prompt Pipeline

The pipeline assembles prompts from composable sections:

```rust
use orka_prompts::pipeline::{SystemPromptPipeline, PipelineConfig, BuildContext};

let config = PipelineConfig::default();
let pipeline = SystemPromptPipeline::from_config(&config);

let ctx = BuildContext::new("MyAgent")
    .with_persona("I am helpful.")
    .with_workspace("default", vec!["default".to_string()]);

let prompt = pipeline.build(&ctx).await?;
```

#### Default Section Order

1. `persona` - Agent identity and personality
2. `datetime` - Current date/time context
3. `workspace` - Workspace awareness (current, available, CWD)
4. `tools` - Tool usage instructions
5. `principles` - Learned principles from experience system
6. `summary` - Prior conversation summary

### 4. Context Providers

Context Providers implement the Strategy Pattern for fetching dynamic data via dependency injection:

```rust
use orka_prompts::context::{
    ContextProvider, ContextProviderRegistry, SessionContext
};

// Implement the trait
pub struct PrinciplesProvider {
    experience: Arc<ExperienceService>,
}

#[async_trait]
impl ContextProvider for PrinciplesProvider {
    fn provider_id(&self) -> &str { "principles" }
    
    async fn provide(&self, ctx: &SessionContext) -> Result<Value> {
        let principles = self.experience
            .retrieve_principles(&ctx.workspace.name, 5)
            .await;
        Ok(json!({ "principles": principles }))
    }
}

// Registry coordinates all providers
let mut registry = ContextProviderRegistry::new(base_context);
registry.register(Box::new(PrinciplesProvider::new(exp)));
registry.register(Box::new(SoftSkillsProvider::new(soft_skills)));

// Gather all data
let build_context = registry.gather_all(&session_ctx).await?;
```

#### Built-in Providers

| Provider | Data Source | Output |
|----------|-------------|--------|
| `PrinciplesProvider` | `ExperienceService` | `{principles: [...]}` |
| `SoftSkillsProvider` | `SoftSkillRegistry` | `{soft_skills: "..."}` |
| `ShellCommandsProvider` | Session metadata | `{shell: {recent_commands: [...]}}` |
| `WorkspaceMetadataProvider` | Session metadata | `{workspace: {...}}` |

### 5. Configuration

Prompt configuration is part of `OrkaConfig`:

```toml
[prompts]
templates_dir = "PROMPTS"        # Relative to workspace
hot_reload = true               # Auto-reload on file changes
section_order = ["persona", "datetime", "workspace", "tools", "principles"]
section_separator = "\n\n"
max_principles = 5
```

## Custom Templates

### Workspace-Level Templates

Create a `PROMPTS/` directory in your workspace:

```
my-workspace/
├── SOUL.md
├── TOOLS.md
└── PROMPTS/
    ├── system/
    │   └── reflection.hbs    # Override reflection prompt
    └── sections/
        └── workspace.hbs     # Override workspace context
```

### Template Syntax

Templates use Handlebars syntax:

```handlebars
{{#if principles}}
## Learned Principles

{{#each principles}}
{{index}}. [{{kind}}] {{text}}
{{/each}}
{{/if}}
```

Built-in helpers:
- `join array separator` - Join array elements
- `inc number` - Increment number (for 1-based indexing)

### Available Context Variables

Each section receives specific context:

**persona**: `agent_name`, `persona`
**datetime**: `datetime`, `date`, `time`, `timezone`
**workspace**: `workspace_name`, `available_workspaces`, `cwd`
**tools**: `instructions`
**principles**: `principles` (array of `{text, kind, index}`)
**summary**: `summary`

## Integration Points

### Agent Crate (with Context Providers)

The modern approach uses dependency injection with context providers:

```rust
use orka_prompts::context::{
    ContextProviderRegistry, SessionContext, PrinciplesProvider,
    SoftSkillsProvider, WorkspaceMetadataProvider, ShellCommandsProvider,
};

// 1. Create session context from trigger data
let session_ctx = SessionContext {
    workspace: WorkspaceContext {
        name: workspace_name,
        available_workspaces,
        cwd,
    },
    trigger: &ctx.trigger,
    experience: deps.experience.as_deref(),
};

// 2. Register providers based on available dependencies
let mut registry = ContextProviderRegistry::new(base_context);
if let Some(exp) = deps.experience.clone() {
    registry.register(Box::new(PrinciplesProvider::new(exp)));
}
if let Some(soft) = deps.soft_skills.clone() {
    registry.register(Box::new(SoftSkillsProvider::new(soft)));
}
registry.register(Box::new(WorkspaceMetadataProvider::new()));
registry.register(Box::new(ShellCommandsProvider::new()));

// 3. Gather context and build
let build_ctx = registry.gather_all(&session_ctx).await?;
let prompt = pipeline.build(&build_ctx).await?;
```

The old manual approach is deprecated:
```rust
// DEPRECATED: Manual data extraction
let principles = experience.retrieve_principles(...).await;
let soft_skills = soft_skill_registry.build_prompt_section(...);
```

### Experience Crate

Reflection and distillation prompts are now template-based:

```rust
// Built-in templates loaded from orka-prompts/templates/system/
const DEFAULT_REFLECTION_PROMPT: &str = 
    include_str!("../../orka-prompts/templates/system/reflection.hbs");
```

### Skills Crate

Soft skill selection prompt can be customized:

```rust
registry.register_inline(
    "selection/soft_skill",
    include_str!("../../orka-prompts/templates/selection/soft_skill.hbs"),
).await?;
```

## Migration Guide

### From Hardcoded to Template

Before:
```rust
let mut prompt = format!("You are {name}.\n\n{persona}");
if !tools.is_empty() {
    prompt.push_str(&format!("\n\n{tools}"));
}
```

After:
```rust
let config = PipelineConfig::default();
let pipeline = SystemPromptPipeline::from_config(&config);
let ctx = BuildContext::new(name)
    .with_persona(persona)
    .with_tool_instructions(tools);
let prompt = pipeline.build(&ctx).await?;
```

## Best Practices

1. **Keep templates focused**: Each section should have a single responsibility
2. **Use conditionals**: Sections can be empty - use `{{#if}}` to handle this
3. **Document context**: Comment what variables are available in each template
4. **Test templates**: Use `TemplateEngine` to test templates in isolation
5. **Version control**: Track template changes in git for rollback capability

## Troubleshooting

### Template Not Found

Ensure templates are loaded:
```rust
// Built-ins are loaded automatically
orka_workspace::load_builtins(&registry).await?;

// Custom templates from PROMPTS/
registry.load_from_dir("PROMPTS").await?;
```

### Hot-Reload Not Working

Check that `hot_reload = true` in config and the file watcher is started:
```rust
let mut loader = TemplateLoader::new(registry, "PROMPTS".into());
loader.load_all().await?;
let mut events = loader.watch().await?;
```

### Performance

Templates are cached after first render. Use `registry.reload(name).await?` to force reload.

## Future Enhancements

- [ ] Async context providers for dynamic data fetching
- [ ] Template inheritance/partials
- [ ] Localization support (i18n)
- [ ] Template validation (JSON schema for context)
- [ ] Web UI for template editing with live preview
