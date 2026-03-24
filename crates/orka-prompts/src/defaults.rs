//! Centralized default values and constants for prompt management.

/// Default maximum number of agent iterations.
pub const DEFAULT_MAX_ITERATIONS: usize = 15;

/// Default timeout for skill execution in seconds.
pub const DEFAULT_SKILL_TIMEOUT_SECS: u64 = 120;

/// Default context window size in tokens.
pub const DEFAULT_CONTEXT_WINDOW: u32 = 128_000;

/// Default maximum output tokens.
pub const DEFAULT_MAX_TOKENS: u32 = 4096;

/// Default sampling temperature.
pub const DEFAULT_TEMPERATURE: f32 = 0.7;

/// Default timezone for datetime formatting.
pub const DEFAULT_TIMEZONE: &str = "Europe/Rome";

// Section formatting constants

/// Separator between prompt sections.
pub const SECTION_SEPARATOR: &str = "\n\n";

/// Header for learned principles section.
pub const PRINCIPLES_SECTION_HEADER: &str = "## Learned Principles";

/// Prefix for "do" principles.
pub const PRINCIPLE_PREFIX_DO: &str = "DO";

/// Prefix for "avoid" principles.
pub const PRINCIPLE_PREFIX_AVOID: &str = "AVOID";

/// Maximum number of principles to include in a prompt.
pub const DEFAULT_MAX_PRINCIPLES: usize = 5;

/// Template file extension.
pub const TEMPLATE_EXTENSION: &str = ".hbs";

/// Default directory for custom templates.
pub const DEFAULT_TEMPLATES_DIR: &str = "prompts";

// System section names

/// Persona section identifier.
pub const SECTION_PERSONA: &str = "persona";

/// Tools section identifier.
pub const SECTION_TOOLS: &str = "tools";

/// Dynamic runtime/context sections identifier.
pub const SECTION_DYNAMIC: &str = "dynamic";

/// Workspace context section identifier.
pub const SECTION_WORKSPACE: &str = "workspace";

/// Learned principles section identifier.
pub const SECTION_PRINCIPLES: &str = "principles";

/// Conversation summary section identifier.
pub const SECTION_SUMMARY: &str = "summary";

/// Current datetime section identifier.
pub const SECTION_DATETIME: &str = "datetime";

/// Default section order for system prompts.
pub const DEFAULT_SECTION_ORDER: &[&str] = &[
    SECTION_PERSONA,
    SECTION_DATETIME,
    SECTION_WORKSPACE,
    SECTION_TOOLS,
    SECTION_DYNAMIC,
    SECTION_PRINCIPLES,
    SECTION_SUMMARY,
];
