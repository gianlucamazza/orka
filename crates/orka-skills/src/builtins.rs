use async_trait::async_trait;
use orka_core::traits::Skill;
use orka_core::{Error, Result, SkillInput, SkillOutput, SkillSchema};

/// Echo skill — returns input arguments as output data.
pub struct EchoSkill;

#[async_trait]
impl Skill for EchoSkill {
    fn name(&self) -> &str {
        "echo"
    }

    fn category(&self) -> &str {
        "general"
    }

    fn description(&self) -> &str {
        "Echoes back the input arguments"
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "properties": {},
            "additionalProperties": true
        }))
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        Ok(SkillOutput::new(
            serde_json::to_value(input.args).map_err(|e| Error::Skill(e.to_string()))?,
        ))
    }
}
