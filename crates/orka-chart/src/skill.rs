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
