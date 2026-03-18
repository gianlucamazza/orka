use std::sync::Arc;

use async_trait::async_trait;
use orka_core::traits::Skill;
use orka_core::{Result, SkillInput, SkillOutput, SkillSchema};

use crate::client::McpClient;

/// Wraps a single MCP tool as an Orka Skill.
pub struct McpToolBridge {
    client: Arc<McpClient>,
    tool_name: String,
    tool_description: String,
    tool_schema: serde_json::Value,
    /// Qualified name: "server_name/tool_name"
    qualified_name: String,
}

impl McpToolBridge {
    /// Create a bridge for a single MCP tool exposed by the given client.
    pub fn new(
        client: Arc<McpClient>,
        tool_name: String,
        tool_description: String,
        tool_schema: serde_json::Value,
    ) -> Self {
        let qualified_name = format!("{}/{}", client.server_name(), tool_name);
        Self {
            client,
            tool_name,
            tool_description,
            tool_schema,
            qualified_name,
        }
    }
}

#[async_trait]
impl Skill for McpToolBridge {
    fn name(&self) -> &str {
        &self.qualified_name
    }

    fn description(&self) -> &str {
        &self.tool_description
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(self.tool_schema.clone())
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        let arguments = serde_json::to_value(&input.args)
            .map_err(|e| orka_core::Error::Skill(format!("failed to serialize args: {e}")))?;

        let result = self.client.call_tool(&self.tool_name, arguments).await?;

        // Concatenate text content blocks
        let text: String = result
            .content
            .iter()
            .map(|c| match c {
                crate::client::McpContent::Text { text } => text.as_str(),
            })
            .collect::<Vec<_>>()
            .join("\n");

        if result.is_error {
            return Err(orka_core::Error::Skill(format!("MCP tool error: {text}")));
        }

        Ok(SkillOutput::new(serde_json::Value::String(text)))
    }
}
