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
