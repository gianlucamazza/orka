use orka_core::{Error, ErrorCategory, Result};
use regex::Regex;

/// Parse the first numeric capture group from output text.
///
/// Returns `None` if no regex is provided, the pattern has no match, or there
/// is no capture group. Fails if the regex is invalid or the captured text
/// cannot be parsed as `f64`.
pub(crate) fn extract_metric(output: &str, regex: Option<&str>) -> Result<Option<f64>> {
    let Some(regex) = regex else {
        return Ok(None);
    };
    let compiled = Regex::new(regex).map_err(|e| Error::SkillCategorized {
        message: format!("invalid metric regex: {e}"),
        category: ErrorCategory::Input,
    })?;
    let Some(captures) = compiled.captures(output) else {
        return Ok(None);
    };
    let Some(value) = captures.get(1) else {
        return Ok(None);
    };
    value
        .as_str()
        .parse::<f64>()
        .map(Some)
        .map_err(|e| Error::SkillCategorized {
            message: format!("failed to parse metric value: {e}"),
            category: ErrorCategory::Input,
        })
}
