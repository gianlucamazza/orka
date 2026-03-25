use colored::Colorize;
use serde::Serialize;

use crate::cmd::doctor::types::{
    Category, CheckMeta, CheckOutcome, CheckResultJson, CheckStatus, DoctorReport, OutputFormat,
    ReportSummary, Severity,
};

pub fn render(report: &DoctorReport, format: OutputFormat, verbose: bool) {
    match format {
        OutputFormat::Text => render_text(report, verbose),
        OutputFormat::Json => render_json(report),
        OutputFormat::Markdown => render_markdown(report),
    }
}

fn status_label(status: CheckStatus) -> colored::ColoredString {
    match status {
        CheckStatus::Pass => "PASS".green().bold(),
        CheckStatus::Fail => "FAIL".red().bold(),
        CheckStatus::Skip => "SKIP".blue(),
    }
}

fn severity_label(severity: Severity) -> colored::ColoredString {
    match severity {
        Severity::Info => "info".normal(),
        Severity::Warning => "warn".yellow(),
        Severity::Error => "error".red(),
        Severity::Critical => "crit".red().bold(),
    }
}

fn render_text(report: &DoctorReport, verbose: bool) {
    println!("{}", "Orka Doctor".bold());
    println!("{}", "===========".bold());

    let categories = [
        Category::Config,
        Category::Connectivity,
        Category::Providers,
        Category::Security,
        Category::Environment,
    ];

    for category in &categories {
        let checks: Vec<_> = report
            .results
            .iter()
            .filter(|(meta, _)| meta.category == *category)
            .collect();

        if checks.is_empty() {
            continue;
        }

        println!("\n{}", category.to_string().bold().underline());

        for (meta, outcome) in &checks {
            let label = status_label(outcome.status);
            let sev = severity_label(meta.severity);
            println!(
                "  [{label}] {id:<8} {sev:<5} {name}",
                id = meta.id.0,
                name = meta.name,
            );

            if outcome.status != CheckStatus::Pass || verbose {
                if !outcome.message.is_empty() {
                    println!("            {}", outcome.message.dimmed());
                }
                if verbose
                    && let Some(detail) = &outcome.detail
                {
                    println!("            {}", detail.dimmed());
                }
                if outcome.status == CheckStatus::Fail
                    && let Some(hint) = &outcome.hint
                {
                    println!("            {}: {}", "hint".yellow(), hint);
                }
            }
        }
    }

    println!();
    print_summary(&report.summary);
}

fn print_summary(summary: &ReportSummary) {
    let passed = format!("{} passed", summary.passed).green();
    let warnings = if summary.warnings > 0 {
        format!("{} warnings", summary.warnings).yellow()
    } else {
        format!("{} warnings", summary.warnings).normal()
    };
    let failed = if summary.failed > 0 {
        format!("{} errors", summary.failed).red()
    } else {
        format!("{} errors", summary.failed).normal()
    };
    let skipped = format!("{} skipped", summary.skipped).blue();

    println!("Summary: {passed}, {warnings}, {failed}, {skipped}");
}

#[derive(Serialize)]
struct JsonReport<'a> {
    schema_version: u32,
    results: Vec<CheckResultJson>,
    summary: &'a ReportSummary,
}

fn render_json(report: &DoctorReport) {
    let results: Vec<CheckResultJson> = report
        .results
        .iter()
        .map(|(meta, outcome)| CheckResultJson {
            id: meta.id.0.to_string(),
            category: format!("{:?}", meta.category).to_lowercase(),
            severity: format!("{:?}", meta.severity).to_lowercase(),
            name: meta.name.to_string(),
            status: outcome.status,
            message: outcome.message.clone(),
            detail: outcome.detail.clone(),
            hint: outcome.hint.clone(),
        })
        .collect();

    let json_report = JsonReport {
        schema_version: 1,
        results,
        summary: &report.summary,
    };

    match serde_json::to_string_pretty(&json_report) {
        Ok(s) => println!("{s}"),
        Err(e) => eprintln!("JSON serialization error: {e}"),
    }
}

fn render_markdown(report: &DoctorReport) {
    println!("# Orka Doctor Report\n");
    println!("| Status | ID | Severity | Name | Message |");
    println!("|--------|-----|----------|------|---------|");

    for (meta, outcome) in &report.results {
        let status_icon = match outcome.status {
            CheckStatus::Pass => "✅",
            CheckStatus::Fail => "❌",
            CheckStatus::Skip => "⏭️",
        };
        let escaped = outcome.message.replace('|', "\\|");
        println!(
            "| {status_icon} | `{}` | {:?} | {} | {escaped} |",
            meta.id.0, meta.severity, meta.name,
        );
    }

    println!();
    println!(
        "**Summary:** {} passed, {} warnings, {} errors, {} skipped",
        report.summary.passed,
        report.summary.warnings,
        report.summary.failed,
        report.summary.skipped,
    );
}

/// Print the list of all available checks.
pub fn list_checks(checks: &[(CheckMeta, &str)]) {
    use comfy_table::{modifiers::UTF8_ROUND_CORNERS, presets::UTF8_FULL, Table};

    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .apply_modifier(UTF8_ROUND_CORNERS)
        .set_header(["ID", "Category", "Severity", "Name"]);

    for (meta, _) in checks {
        table.add_row([
            meta.id.0,
            meta.category.to_string().as_str(),
            &format!("{:?}", meta.severity),
            meta.name,
        ]);
    }

    println!("{table}");
}

/// Print detailed explanation for a single check.
pub fn explain_check(meta: &CheckMeta, explanation: &str) {
    println!("{} — {}", meta.id.0.bold(), meta.name.bold());
    println!(
        "Category: {}  |  Severity: {:?}",
        meta.category, meta.severity
    );
    println!();
    println!("{explanation}");
}

/// Build the DoctorReport summary from results.
pub fn build_report(results: Vec<(CheckMeta, CheckOutcome)>) -> DoctorReport {
    let total = results.len();
    let passed = results
        .iter()
        .filter(|(_, o)| o.status == CheckStatus::Pass)
        .count();
    let skipped = results
        .iter()
        .filter(|(_, o)| o.status == CheckStatus::Skip)
        .count();
    let failed_results: Vec<_> = results
        .iter()
        .filter(|(_, o)| o.status == CheckStatus::Fail)
        .collect();
    let warnings = failed_results
        .iter()
        .filter(|(m, _)| m.severity == Severity::Warning || m.severity == Severity::Info)
        .count();
    let failed = failed_results
        .iter()
        .filter(|(m, _)| m.severity == Severity::Error || m.severity == Severity::Critical)
        .count();

    DoctorReport {
        results,
        summary: ReportSummary {
            total,
            passed,
            failed,
            skipped,
            warnings,
        },
    }
}
