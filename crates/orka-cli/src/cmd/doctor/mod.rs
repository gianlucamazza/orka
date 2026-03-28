mod checks;
mod output;
mod registry;
mod types;

use std::{path::PathBuf, sync::Arc, time::Duration};

use async_trait::async_trait;
use dialoguer::Confirm;
use orka_config::OrkaConfig;
pub use types::*;

/// Context passed to every check.
pub struct CheckContext {
    pub config: Option<OrkaConfig>,
    pub config_path: PathBuf,
    pub config_raw: Option<String>,
    pub verbose: bool,
    pub timeout: Duration,
}

/// A single diagnostic check.
#[async_trait]
pub trait DoctorCheck: Send + Sync {
    fn meta(&self) -> CheckMeta;
    async fn run(&self, ctx: &CheckContext) -> CheckOutcome;

    fn explain(&self) -> &'static str {
        self.meta().description
    }
}

/// Subcommand actions for `orka doctor`.
#[derive(clap::Subcommand)]
pub enum DoctorAction {
    /// List all available checks with their IDs, categories, and severities
    List,
    /// Show a detailed explanation for a specific check
    Explain {
        /// Check ID (e.g., CFG-001)
        id: String,
    },
}

/// Main entry point: run all checks and render the report.
#[allow(clippy::too_many_arguments)]
pub async fn run(
    config_path: Option<&str>,
    format: OutputFormat,
    category: Option<Category>,
    check_id: Option<&str>,
    min_severity: Severity,
    verbose: bool,
    fix: bool,
    timeout_secs: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    let timeout = Duration::from_secs(timeout_secs);

    // Load config context
    let ctx = build_context(config_path, verbose, timeout)?;
    let ctx = Arc::new(ctx);

    // Get all checks
    let all_checks = registry::build_registry();

    // Apply filters
    let checks: Vec<_> = all_checks
        .into_iter()
        .filter(|c| {
            let meta = c.meta();
            category.is_none_or(|cat| meta.category == cat)
                && check_id.is_none_or(|id| meta.id.as_str() == id)
                && meta.severity >= min_severity
        })
        .collect();

    // Run checks in three phases
    let results = run_checks(checks, ctx.clone(), timeout).await;

    // Build and render report
    let report = output::build_report(results);
    output::render(&report, format, verbose);

    // Exit code (computed before consuming report)
    let code = exit_code(&report);

    // Auto-fix phase (consumes report.results)
    if fix {
        run_fixes(report.results, ctx, timeout).await;
    }
    if code != 0 {
        std::process::exit(code);
    }

    Ok(())
}

/// List all available checks.
#[allow(clippy::unnecessary_wraps)]
pub fn list_checks() -> Result<(), Box<dyn std::error::Error>> {
    let checks = registry::build_registry();
    let pairs: Vec<_> = checks
        .iter()
        .map(|c| {
            let meta = c.meta();
            let explain = c.explain();
            (meta, explain)
        })
        .collect();
    output::list_checks(&pairs);
    Ok(())
}

/// Explain a specific check by ID.
pub fn explain_check(id: &str) -> Result<(), Box<dyn std::error::Error>> {
    let checks = registry::build_registry();
    match checks.iter().find(|c| c.meta().id.as_str() == id) {
        Some(check) => {
            output::explain_check(&check.meta(), check.explain());
            Ok(())
        }
        None => Err(format!("unknown check ID: {id}").into()),
    }
}

fn build_context(
    config_path: Option<&str>,
    verbose: bool,
    timeout: Duration,
) -> Result<CheckContext, Box<dyn std::error::Error>> {
    let path = config_path.map(std::path::Path::new);
    let config_path = OrkaConfig::resolve_path(path);

    let config_raw = if config_path.exists() {
        Some(std::fs::read_to_string(&config_path)?)
    } else {
        None
    };

    // Try to load the config (may fail — checks that need it will skip if None)
    let config = if config_path.exists() {
        OrkaConfig::load(Some(&config_path)).ok()
    } else {
        None
    };

    Ok(CheckContext {
        config,
        config_path,
        config_raw,
        verbose,
        timeout,
    })
}

async fn run_checks(
    checks: Vec<Box<dyn DoctorCheck>>,
    ctx: Arc<CheckContext>,
    timeout: Duration,
) -> Vec<(CheckMeta, CheckOutcome)> {
    let mut results = Vec::new();

    // Phase 1: sequential — Config (order matters, early-abort on Critical fail)
    let mut config_critical_failed = false;
    for check in checks
        .iter()
        .filter(|c| c.meta().category == Category::Config)
    {
        let meta = check.meta();
        if config_critical_failed {
            results.push((
                meta,
                CheckOutcome::skip("skipped: prior critical config check failed"),
            ));
            continue;
        }
        let outcome = run_with_timeout(check.as_ref(), &ctx, timeout).await;
        if outcome.status == CheckStatus::Fail && meta.severity == Severity::Critical {
            config_critical_failed = true;
        }
        results.push((meta, outcome));
    }

    // Phase 2: sequential — Security + Environment (filesystem, fast)
    for check in checks.iter().filter(|c| {
        matches!(
            c.meta().category,
            Category::Security | Category::Environment
        )
    }) {
        let outcome = run_with_timeout(check.as_ref(), &ctx, timeout).await;
        results.push((check.meta(), outcome));
    }

    // Phase 3: parallel — Connectivity + Providers (network I/O)
    let mut join_set = tokio::task::JoinSet::new();
    for check in checks.into_iter().filter(|c| {
        matches!(
            c.meta().category,
            Category::Connectivity | Category::Providers
        )
    }) {
        let ctx = ctx.clone();
        join_set.spawn(async move {
            let meta = check.meta();
            let outcome = run_with_timeout(check.as_ref(), &ctx, timeout).await;
            (meta, outcome)
        });
    }

    let mut network_results = Vec::new();
    while let Some(res) = join_set.join_next().await {
        match res {
            Ok(pair) => network_results.push(pair),
            Err(join_err) => {
                // A check task panicked or was cancelled — surface it as a synthetic
                // failure rather than silently dropping the result.
                let meta = CheckMeta {
                    id: CheckId::new("ERR-000"),
                    category: Category::Connectivity,
                    severity: Severity::Error,
                    name: "check task panicked",
                    description: "A diagnostic check panicked during parallel execution.",
                };
                network_results.push((
                    meta,
                    CheckOutcome::fail(format!("task panicked: {join_err}")),
                ));
            }
        }
    }

    // Sort network results by check ID for deterministic output
    network_results.sort_by(|a, b| a.0.id.as_str().cmp(b.0.id.as_str()));
    results.extend(network_results);

    results
}

async fn run_with_timeout(
    check: &dyn DoctorCheck,
    ctx: &CheckContext,
    timeout: Duration,
) -> CheckOutcome {
    match tokio::time::timeout(timeout, check.run(ctx)).await {
        Ok(outcome) => outcome,
        Err(_) => CheckOutcome::fail(format!("check timed out after {}s", timeout.as_secs()))
            .with_hint("Increase --timeout or check network connectivity."),
    }
}

#[allow(clippy::unused_async)]
async fn run_fixes(
    results: Vec<(CheckMeta, CheckOutcome)>,
    ctx: Arc<CheckContext>,
    _timeout: Duration,
) {
    let fixable: Vec<_> = results
        .into_iter()
        .filter(|(_, o)| o.status == CheckStatus::Fail && o.fix.is_some())
        .collect();

    if fixable.is_empty() {
        println!("\nNo auto-fixes available for failed checks.");
        return;
    }

    println!("\n{} auto-fix(es) available:", fixable.len());

    for (meta, mut outcome) in fixable {
        if let Some(fix) = outcome.fix.take() {
            let prompt = format!(
                "Apply fix for {} ({}): {}?",
                meta.id, meta.name, fix.description
            );
            let confirmed = Confirm::new()
                .with_prompt(&prompt)
                .default(false)
                .interact()
                .unwrap_or(false);

            if confirmed {
                match (fix.apply)() {
                    Ok(msg) => println!("  Applied: {msg}"),
                    Err(e) => println!("  Failed: {e}"),
                }
            } else {
                println!("  Skipped.");
            }
        }
    }

    // Suppress unused variable warning
    drop(ctx);
}

fn exit_code(report: &DoctorReport) -> i32 {
    let has_critical_or_error = report.results.iter().any(|(meta, outcome)| {
        outcome.status == CheckStatus::Fail
            && matches!(meta.severity, Severity::Critical | Severity::Error)
    });
    let has_warnings = report.results.iter().any(|(meta, outcome)| {
        outcome.status == CheckStatus::Fail
            && matches!(meta.severity, Severity::Warning | Severity::Info)
    });

    if has_critical_or_error {
        1
    } else if has_warnings {
        2
    } else {
        0
    }
}
