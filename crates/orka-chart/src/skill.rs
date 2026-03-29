//! `create_chart` skill — renders a chart to PNG and attaches it inline.

use async_trait::async_trait;
use orka_core::{
    Error, Result, SkillInput, SkillOutput, SkillSchema, traits::Skill, types::MediaPayload,
};
use serde_json::json;
use tracing::instrument;

use crate::{render::render_chart, types::ChartSpec};

/// Skill that generates a chart image from a JSON spec and returns it as an
/// inline `image/png` attachment.
pub struct ChartCreateSkill;

#[async_trait]
impl Skill for ChartCreateSkill {
    fn name(&self) -> &'static str {
        "create_chart"
    }

    fn category(&self) -> &'static str {
        "visualization"
    }

    fn description(&self) -> &'static str {
        "Generate a chart (bar, line, pie, scatter, histogram, area, stacked_bar, combo) \
         from structured data and return it as an inline PNG image."
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(json!({
            "type": "object",
            "required": ["chart_type", "data"],
            "properties": {
                "chart_type": {
                    "type": "string",
                    "enum": ["bar", "line", "pie", "scatter", "histogram", "area", "stacked_bar", "combo"],
                    "description": "The type of chart to render."
                },
                "title": {
                    "type": "string",
                    "description": "Optional chart title."
                },
                "x_label": {
                    "type": "string",
                    "description": "Optional X-axis label."
                },
                "y_label": {
                    "type": "string",
                    "description": "Optional Y-axis label."
                },
                "width": {
                    "type": "integer",
                    "default": 800,
                    "minimum": 200,
                    "maximum": 2000,
                    "description": "Output width in pixels."
                },
                "height": {
                    "type": "integer",
                    "default": 600,
                    "minimum": 200,
                    "maximum": 2000,
                    "description": "Output height in pixels."
                },
                "data": {
                    "type": "object",
                    "required": ["series"],
                    "properties": {
                        "labels": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Category labels for the X axis."
                        },
                        "series": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "required": ["name", "values"],
                                "properties": {
                                    "name": { "type": "string" },
                                    "values": {
                                        "type": "array",
                                        "items": { "type": "number" }
                                    },
                                    "chart_type": {
                                        "type": "string",
                                        "description": "Per-series chart type override for combo charts."
                                    },
                                    "color": {
                                        "type": "string",
                                        "description": "Hex color string, e.g. '#4CAF50'."
                                    }
                                }
                            }
                        }
                    }
                },
                "caption": {
                    "type": "string",
                    "description": "Caption displayed alongside the chart image."
                }
            }
        }))
    }

    #[instrument(skip(self, input), fields(skill = "create_chart"))]
    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        let spec: ChartSpec = serde_json::from_value(serde_json::Value::Object(
            input
                .args
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        ))
        .map_err(|e| Error::Skill(format!("invalid chart spec: {e}")))?;

        if spec.width < 200 || spec.width > 2000 {
            return Err(Error::Skill(format!(
                "width must be between 200 and 2000, got {}",
                spec.width
            )));
        }
        if spec.height < 200 || spec.height > 2000 {
            return Err(Error::Skill(format!(
                "height must be between 200 and 2000, got {}",
                spec.height
            )));
        }

        let chart_type = format!("{:?}", spec.chart_type).to_lowercase();
        let title = spec.title.clone();
        let caption = spec.caption.clone();

        let png_bytes =
            render_chart(&spec).map_err(|e| Error::Skill(format!("chart render failed: {e}")))?;

        let attachment = MediaPayload::inline("image/png", png_bytes, caption);

        let data = json!({
            "success": true,
            "chart_type": chart_type,
            "title": title,
        });

        Ok(SkillOutput::new(data).with_attachments(vec![attachment]))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::collections::HashMap;

    use base64::Engine as _;
    use orka_core::SkillInput;

    use super::*;

    fn input(args: serde_json::Value) -> SkillInput {
        let map: HashMap<String, serde_json::Value> =
            serde_json::from_value(args).expect("valid test args");
        SkillInput::new(map)
    }

    #[tokio::test]
    async fn execute_bar_chart_returns_png_attachment() {
        let skill = ChartCreateSkill;
        let result = skill
            .execute(input(serde_json::json!({
                "chart_type": "bar",
                "data": {
                    "labels": ["A", "B", "C"],
                    "series": [{"name": "s1", "values": [1.0, 2.0, 3.0]}]
                }
            })))
            .await
            .unwrap();

        assert_eq!(result.attachments.len(), 1);
        let attachment = &result.attachments[0];
        assert_eq!(attachment.mime_type, "image/png");
        let b64 = attachment.data_base64.as_ref().expect("inline data");
        let bytes = base64::engine::general_purpose::STANDARD.decode(b64).unwrap();
        assert!(bytes.starts_with(b"\x89PNG"), "expected PNG header");
    }

    #[tokio::test]
    async fn execute_returns_chart_type_and_title_in_data() {
        let skill = ChartCreateSkill;
        let result = skill
            .execute(input(serde_json::json!({
                "chart_type": "line",
                "title": "My Chart",
                "data": {
                    "labels": ["x", "y"],
                    "series": [{"name": "s", "values": [5.0, 10.0]}]
                }
            })))
            .await
            .unwrap();

        assert_eq!(result.data["success"], serde_json::json!(true));
        assert_eq!(result.data["chart_type"], serde_json::json!("line"));
        assert_eq!(result.data["title"], serde_json::json!("My Chart"));
    }

    #[tokio::test]
    async fn execute_rejects_width_out_of_range() {
        let skill = ChartCreateSkill;
        let err = skill
            .execute(input(serde_json::json!({
                "chart_type": "bar",
                "width": 50,
                "data": {"series": [{"name": "s", "values": [1.0]}]}
            })))
            .await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn execute_rejects_invalid_chart_type() {
        let skill = ChartCreateSkill;
        let err = skill
            .execute(input(serde_json::json!({
                "chart_type": "not_a_real_type",
                "data": {"series": [{"name": "s", "values": [1.0]}]}
            })))
            .await;
        assert!(err.is_err());
    }

    #[test]
    fn skill_metadata() {
        let skill = ChartCreateSkill;
        assert_eq!(skill.name(), "create_chart");
        assert_eq!(skill.category(), "visualization");
        assert!(!skill.description().is_empty());
    }
}
