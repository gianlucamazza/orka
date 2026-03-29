//! Chart specification types used by the `create_chart` skill.

use serde::Deserialize;

/// Supported chart types.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChartType {
    /// Vertical bar chart.
    Bar,
    /// Line chart.
    Line,
    /// Pie chart.
    Pie,
    /// Scatter plot.
    Scatter,
    /// Histogram.
    Histogram,
    /// Area (filled line) chart.
    Area,
    /// Stacked bar chart.
    StackedBar,
    /// Combination chart (per-series type override).
    Combo,
}

/// A single data series.
#[derive(Debug, Clone, Deserialize)]
pub struct Series {
    /// Human-readable series name (used in legend).
    pub name: String,
    /// Numeric data points.
    pub values: Vec<f64>,
    /// Per-series chart type override (for combo charts).
    #[serde(default)]
    pub chart_type: Option<String>,
    /// Optional hex colour string, e.g. `"#4CAF50"`.
    #[serde(default)]
    pub color: Option<String>,
}

/// Data section of a chart spec.
#[derive(Debug, Clone, Deserialize)]
pub struct ChartData {
    /// Category labels along the X axis.
    #[serde(default)]
    pub labels: Vec<String>,
    /// One or more series.
    pub series: Vec<Series>,
}

/// Full chart specification received from the LLM.
#[derive(Debug, Clone, Deserialize)]
pub struct ChartSpec {
    /// Type of chart to render.
    pub chart_type: ChartType,
    /// Optional chart title.
    #[serde(default)]
    pub title: Option<String>,
    /// Optional X-axis label.
    #[serde(default)]
    pub x_label: Option<String>,
    /// Optional Y-axis label.
    #[serde(default)]
    pub y_label: Option<String>,
    /// Output width in pixels (default 800).
    #[serde(default = "default_width")]
    pub width: u32,
    /// Output height in pixels (default 600).
    #[serde(default = "default_height")]
    pub height: u32,
    /// Chart data.
    pub data: ChartData,
    /// Caption sent alongside the image.
    #[serde(default)]
    pub caption: Option<String>,
}

fn default_width() -> u32 {
    800
}
fn default_height() -> u32 {
    600
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn bar_spec(labels: Vec<&str>, values: Vec<f64>) -> ChartSpec {
        ChartSpec {
            chart_type: ChartType::Bar,
            title: Some("Test".into()),
            x_label: None,
            y_label: None,
            width: 800,
            height: 600,
            caption: None,
            data: ChartData {
                labels: labels.into_iter().map(String::from).collect(),
                series: vec![Series {
                    name: "s1".into(),
                    values,
                    chart_type: None,
                    color: None,
                }],
            },
        }
    }

    #[test]
    fn default_dimensions() {
        let spec = bar_spec(vec!["a"], vec![1.0]);
        assert_eq!(spec.width, 800);
        assert_eq!(spec.height, 600);
    }

    #[test]
    fn deserializes_from_json() {
        let json = r#"{
            "chart_type": "bar",
            "data": {
                "labels": ["A", "B"],
                "series": [{"name": "s1", "values": [1.0, 2.0]}]
            }
        }"#;
        let spec: ChartSpec = serde_json::from_str(json).unwrap();
        assert_eq!(spec.data.labels, vec!["A", "B"]);
        assert_eq!(spec.data.series[0].values, vec![1.0, 2.0]);
        assert_eq!(spec.width, 800);
        assert_eq!(spec.height, 600);
    }

    #[test]
    fn series_with_color_and_type_override() {
        let json = r##"{
            "chart_type": "combo",
            "data": {
                "labels": ["A"],
                "series": [
                    {"name": "s1", "values": [1.0], "color": "#FF0000", "chart_type": "bar"},
                    {"name": "s2", "values": [2.0]}
                ]
            }
        }"##;
        let spec: ChartSpec = serde_json::from_str(json).unwrap();
        assert_eq!(spec.data.series[0].color.as_deref(), Some("#FF0000"));
        assert_eq!(spec.data.series[0].chart_type.as_deref(), Some("bar"));
        assert!(spec.data.series[1].color.is_none());
    }

    #[test]
    fn all_chart_types_deserialize() {
        for variant in &[
            "bar",
            "line",
            "pie",
            "scatter",
            "histogram",
            "area",
            "stacked_bar",
            "combo",
        ] {
            let json = format!(
                r#"{{"chart_type":"{variant}","data":{{"series":[{{"name":"s","values":[1.0]}}]}}}}"#
            );
            let result: Result<ChartSpec, _> = serde_json::from_str(&json);
            assert!(result.is_ok(), "failed to deserialize chart_type={variant}");
        }
    }
}
