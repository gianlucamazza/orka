//! Custom Skill Example
//!
//! Demonstrates how to create a custom skill for Orka.
//! This skill calculates the factorial of a number.
//!
//! ## Running the example
//!
//! ```bash
//! cargo run --bin custom_skill
//! ```

use async_trait::async_trait;
use orka_core::traits::Skill;
use orka_core::types::{Result, SkillInput, SkillOutput, SkillSchema};
use serde::Deserialize;

/// Custom skill that calculates factorial
pub struct FactorialSkill;

#[derive(Debug, Deserialize)]
struct FactorialInput {
    number: u64,
}

impl FactorialSkill {
    pub fn new() -> Self {
        Self
    }

    fn calculate_factorial(&self, n: u64) -> u64 {
        (1..=n).product()
    }
}

impl Default for FactorialSkill {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Skill for FactorialSkill {
    fn name(&self) -> &str {
        "factorial"
    }

    fn description(&self) -> &str {
        "Calculates the factorial of a non-negative integer"
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema {
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "number": {
                        "type": "integer",
                        "minimum": 0,
                        "maximum": 20,
                        "description": "The number to calculate factorial for"
                    }
                },
                "required": ["number"]
            }),
        }
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        // Parse input
        let args: FactorialInput = serde_json::from_value(serde_json::to_value(&input.args)?)
            .map_err(|e| orka_core::Error::Skill(format!("Invalid input: {}", e)))?;

        // Validate range to prevent overflow
        if args.number > 20 {
            return Err(orka_core::Error::Skill(
                "Number too large. Maximum is 20 to prevent overflow.".into()
            ));
        }

        // Calculate
        let result = self.calculate_factorial(args.number);

        // Return output
        Ok(SkillOutput {
            data: serde_json::json!({
                "input": args.number,
                "result": result,
                "formula": format!("{}! = {}", args.number, result)
            }),
        })
    }

    fn category(&self) -> &str {
        "math"
    }
}

/// Another custom skill: weather info (mock)
pub struct WeatherSkill;

#[derive(Debug, Deserialize)]
struct WeatherInput {
    city: String,
}

impl WeatherSkill {
    pub fn new() -> Self {
        Self
    }
}

impl Default for WeatherSkill {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Skill for WeatherSkill {
    fn name(&self) -> &str {
        "weather"
    }

    fn description(&self) -> &str {
        "Gets current weather information for a city (mock implementation)"
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema {
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "city": {
                        "type": "string",
                        "description": "City name"
                    }
                },
                "required": ["city"]
            }),
        }
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        let city = input.args.get("city")
            .and_then(|v| v.as_str())
            .ok_or_else(|| orka_core::Error::Skill("City parameter required".into()))?;

        // Mock weather data
        let weather = serde_json::json!({
            "city": city,
            "temperature": 22,
            "unit": "celsius",
            "condition": "sunny",
            "humidity": 45,
        });

        Ok(SkillOutput { data: weather })
    }

    fn category(&self) -> &str {
        "info"
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    println!("=== Custom Skill Example ===\n");

    // Create and register skills
    let factorial = FactorialSkill::new();
    let weather = WeatherSkill::new();

    println!("Registered Skills:");
    println!("  - {}: {}", factorial.name(), factorial.description());
    println!("  - {}: {}\n", weather.name(), weather.description());

    // Test factorial skill
    println!("Testing factorial skill:");
    let input = SkillInput {
        args: [("number".into(), serde_json::json!(5))].into(),
        context: None,
    };
    
    match factorial.execute(input).await {
        Ok(output) => {
            println!("  Input: 5");
            println!("  Output: {}", serde_json::to_string_pretty(&output.data)?);
        }
        Err(e) => println!("  Error: {}", e),
    }

    // Test weather skill
    println!("\nTesting weather skill:");
    let input = SkillInput {
        args: [("city".into(), serde_json::json!("Rome"))].into(),
        context: None,
    };

    match weather.execute(input).await {
        Ok(output) => {
            println!("  City: Rome");
            println!("  Output: {}", serde_json::to_string_pretty(&output.data)?);
        }
        Err(e) => println!("  Error: {}", e),
    }

    // Show schemas
    println!("\nSkill Schemas (for LLM tool use):");
    println!("  factorial: {}", serde_json::to_string_pretty(&factorial.schema().parameters)?);
    println!("  weather: {}", serde_json::to_string_pretty(&weather.schema().parameters)?);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_factorial() {
        let skill = FactorialSkill::new();
        let input = SkillInput {
            args: [("number".into(), serde_json::json!(5))].into(),
            context: None,
        };

        let output = skill.execute(input).await.unwrap();
        assert_eq!(output.data["result"], 120);
    }

    #[tokio::test]
    async fn test_factorial_zero() {
        let skill = FactorialSkill::new();
        let input = SkillInput {
            args: [("number".into(), serde_json::json!(0))].into(),
            context: None,
        };

        let output = skill.execute(input).await.unwrap();
        assert_eq!(output.data["result"], 1);
    }

    #[tokio::test]
    async fn test_weather() {
        let skill = WeatherSkill::new();
        let input = SkillInput {
            args: [("city".into(), serde_json::json!("Paris"))].into(),
            context: None,
        };

        let output = skill.execute(input).await.unwrap();
        assert_eq!(output.data["city"], "Paris");
    }
}
