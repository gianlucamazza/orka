use serde::Serialize;

/// Stable identifier for a check (e.g., "CFG-001").
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct CheckId(&'static str);

impl CheckId {
    /// Create a new check identifier from a static string.
    pub const fn new(id: &'static str) -> Self {
        Self(id)
    }

    /// Return the identifier as a string slice.
    pub fn as_str(&self) -> &str {
        self.0
    }
}

impl std::fmt::Display for CheckId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

impl AsRef<str> for CheckId {
    fn as_ref(&self) -> &str {
        self.0
    }
}

/// Check category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    Config,
    Connectivity,
    Providers,
    Security,
    Environment,
}

impl std::fmt::Display for Category {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Category::Config => write!(f, "Config"),
            Category::Connectivity => write!(f, "Connectivity"),
            Category::Providers => write!(f, "Providers"),
            Category::Security => write!(f, "Security"),
            Category::Environment => write!(f, "Environment"),
        }
    }
}

/// Severity of a check outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warning,
    Error,
    Critical,
}

/// Outcome status of a single check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckStatus {
    Pass,
    Fail,
    Skip,
}

/// Auto-fix action for a failed check.
pub struct FixAction {
    pub description: String,
    pub apply: Box<dyn FnOnce() -> Result<String, Box<dyn std::error::Error + Send + Sync>> + Send>,
}

/// Result of running a single check.
pub struct CheckOutcome {
    pub status: CheckStatus,
    pub message: String,
    /// Additional details shown with --verbose.
    pub detail: Option<String>,
    /// Hint for manual remediation.
    pub hint: Option<String>,
    /// Optional auto-fix (used when --fix is passed).
    pub fix: Option<FixAction>,
}

impl CheckOutcome {
    pub fn pass(message: impl Into<String>) -> Self {
        Self {
            status: CheckStatus::Pass,
            message: message.into(),
            detail: None,
            hint: None,
            fix: None,
        }
    }

    pub fn fail(message: impl Into<String>) -> Self {
        Self {
            status: CheckStatus::Fail,
            message: message.into(),
            detail: None,
            hint: None,
            fix: None,
        }
    }

    pub fn skip(message: impl Into<String>) -> Self {
        Self {
            status: CheckStatus::Skip,
            message: message.into(),
            detail: None,
            hint: None,
            fix: None,
        }
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn with_fix(mut self, fix: FixAction) -> Self {
        self.fix = Some(fix);
        self
    }
}

/// Metadata for a registered check.
#[derive(Debug, Clone)]
pub struct CheckMeta {
    pub id: CheckId,
    pub category: Category,
    pub severity: Severity,
    pub name: &'static str,
    pub description: &'static str,
}

/// Serializable result entry (check meta + outcome, without the fix closure).
#[derive(Debug, Serialize)]
pub struct CheckResultJson {
    pub id: String,
    pub category: String,
    pub severity: String,
    pub name: String,
    pub status: CheckStatus,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

/// Summary of the doctor run.
#[derive(Debug, Default, Serialize)]
pub struct ReportSummary {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub warnings: usize,
}

/// Full doctor run report.
pub struct DoctorReport {
    pub results: Vec<(CheckMeta, CheckOutcome)>,
    pub summary: ReportSummary,
}

/// Output format selection.
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum OutputFormat {
    Text,
    Json,
    Markdown,
}
