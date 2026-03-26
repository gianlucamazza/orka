//! Tool definitions for the onboarding wizard LLM.
//!
//! The wizard LLM uses these tools to build the config incrementally,
//! store secrets, ask the user questions, and signal completion.

use orka_llm::ToolDefinition;
use serde_json::json;

/// All tool definitions exposed to the wizard LLM.
pub fn all_tools() -> Vec<ToolDefinition> {
    vec![
        set_config_tool(),
        append_config_tool(),
        store_secret_tool(),
        ask_user_tool(),
        validate_config_tool(),
        finalize_tool(),
    ]
}

fn set_config_tool() -> ToolDefinition {
    ToolDefinition::new(
        "set_config",
        "Set key-value pairs under a config section. \
         Use for scalar sections like [server], [redis], [llm], [adapters.telegram], etc.",
        json!({
            "type": "object",
            "required": ["section", "values"],
            "properties": {
                "section": {
                    "type": "string",
                    "description": "Dotted section path, e.g. \"server\", \"adapters.telegram\", \"llm\""
                },
                "values": {
                    "type": "object",
                    "description": "Key-value pairs to set in the section. Null values are omitted."
                }
            }
        }),
    )
}

fn append_config_tool() -> ToolDefinition {
    ToolDefinition::new(
        "append_config",
        "Append an entry to an array-of-tables section. \
         Use for [[agents]], [[llm.providers]], [[graph.edges]], [[mcp.servers]], etc.",
        json!({
            "type": "object",
            "required": ["section", "entry"],
            "properties": {
                "section": {
                    "type": "string",
                    "description": "Array-of-tables path, e.g. \"agents\", \"llm.providers\""
                },
                "entry": {
                    "type": "object",
                    "description": "The new table entry to append."
                }
            }
        }),
    )
}

fn store_secret_tool() -> ToolDefinition {
    ToolDefinition::new(
        "store_secret",
        "Prompt the user to enter a secret value (e.g. API key, bot token) and store it \
         securely. Returns the secret path to reference in config via `api_key_secret` or \
         `bot_token_secret`. NEVER store secret values directly in config.",
        json!({
            "type": "object",
            "required": ["path", "prompt"],
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Secret store path, e.g. \"llm/anthropic\", \"adapters/telegram\""
                },
                "prompt": {
                    "type": "string",
                    "description": "Human-readable prompt shown to the user, e.g. \"Enter your Anthropic API key\""
                }
            }
        }),
    )
}

fn ask_user_tool() -> ToolDefinition {
    ToolDefinition::new(
        "ask_user",
        "Ask the user a question. Use this when you need information to proceed. \
         Provide options for selection questions; omit for free-text input.",
        json!({
            "type": "object",
            "required": ["question"],
            "properties": {
                "question": {
                    "type": "string",
                    "description": "The question to ask the user"
                },
                "options": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional list of choices. If provided, user selects from these."
                },
                "multi_select": {
                    "type": "boolean",
                    "description": "If true, user can select multiple options. Default: false.",
                    "default": false
                }
            }
        }),
    )
}

fn validate_config_tool() -> ToolDefinition {
    ToolDefinition::new(
        "validate_config",
        "Validate the current config state. Returns any validation errors or warnings. \
         Call this after setting up the main sections to catch issues early.",
        json!({
            "type": "object",
            "properties": {}
        }),
    )
}

fn finalize_tool() -> ToolDefinition {
    ToolDefinition::new(
        "finalize",
        "Signal that the onboarding wizard is complete. \
         Call this when you have configured at least a minimal working setup \
         and the user is satisfied.",
        json!({
            "type": "object",
            "required": ["summary"],
            "properties": {
                "summary": {
                    "type": "string",
                    "description": "A brief summary of what was configured (2-4 sentences)."
                }
            }
        }),
    )
}
