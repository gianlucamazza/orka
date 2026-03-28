//! Chart generation skill configuration.

use serde::{Deserialize, Serialize};

/// Configuration for the `create_chart` skill.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
#[non_exhaustive]
pub struct ChartConfig {
    /// Enable chart generation skills. Defaults to `false`.
    pub enabled: bool,
}
