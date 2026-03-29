use std::{
    collections::{BTreeSet, HashMap},
    fs,
    path::{Path, PathBuf},
};

use async_trait::async_trait;
use regex::Regex;

use crate::cmd::doctor::{
    CheckContext, DoctorCheck,
    types::{Category, CheckId, CheckMeta, CheckOutcome, Severity},
};

pub struct ArchLayeringViolations;
pub struct ArchCrateTestMinimum;
pub struct ArchOversizedModules;

const MIN_CRATE_TESTS: usize = 3;
const MAX_MODULE_LINES: usize = 500;
const MIN_FUNCTIONS_FOR_MODULE_WARNING: usize = 5;

#[async_trait]
impl DoctorCheck for ArchLayeringViolations {
    fn meta(&self) -> CheckMeta {
        CheckMeta {
            id: CheckId::new("ARC-001"),
            category: Category::Architecture,
            severity: Severity::Critical,
            name: "Layering violations",
            description: "Workspace crates must depend only on the same layer or lower layers.",
        }
    }

    async fn run(&self, _ctx: &CheckContext) -> CheckOutcome {
        let Some(root) = find_workspace_root() else {
            return CheckOutcome::skip("workspace Cargo.toml not found from current directory");
        };

        let workspace = match load_workspace_model(&root) {
            Ok(workspace) => workspace,
            Err(err) => return CheckOutcome::skip(err),
        };

        let mut violations = Vec::new();
        for member in &workspace.members {
            let Some(source_layer) = member.layer.as_ref() else {
                continue;
            };

            for dependency in &member.internal_dependencies {
                let Some(target) = workspace.members_by_name.get(dependency) else {
                    continue;
                };
                let Some(target_layer) = target.layer.as_ref() else {
                    continue;
                };

                if target_layer.index > source_layer.index {
                    violations.push(format!(
                        "{} ({} L{}) -> {} ({} L{})",
                        member.name,
                        source_layer.name,
                        source_layer.index,
                        target.name,
                        target_layer.name,
                        target_layer.index,
                    ));
                }
            }
        }

        if violations.is_empty() {
            CheckOutcome::pass(format!(
                "{} workspace crate(s) checked",
                workspace.members.len()
            ))
        } else {
            CheckOutcome::fail(format!(
                "{} layering violation(s) detected",
                violations.len()
            ))
            .with_detail(violations.join("\n"))
            .with_hint(
                "Move the depending crate to a higher layer, or extract shared types/traits into a lower layer.",
            )
        }
    }

    fn explain(&self) -> &'static str {
        "The workspace root Cargo.toml defines the architectural layer order for internal crates. \
         This check parses that order, reads each member Cargo.toml, and fails when a crate \
         depends on another crate declared in a higher layer. \
         Same-layer dependencies are allowed; upward edges are not."
    }
}

#[async_trait]
impl DoctorCheck for ArchCrateTestMinimum {
    fn meta(&self) -> CheckMeta {
        CheckMeta {
            id: CheckId::new("ARC-002"),
            category: Category::Architecture,
            severity: Severity::Warning,
            name: "Minimum crate test coverage",
            description: "Each first-class crate should have at least a small baseline of tests.",
        }
    }

    async fn run(&self, _ctx: &CheckContext) -> CheckOutcome {
        let Some(root) = find_workspace_root() else {
            return CheckOutcome::skip("workspace Cargo.toml not found from current directory");
        };

        let workspace = match load_workspace_model(&root) {
            Ok(workspace) => workspace,
            Err(err) => return CheckOutcome::skip(err),
        };

        let minimal: Vec<_> = workspace
            .members
            .iter()
            .filter(|member| member.path.starts_with("crates/"))
            .filter(|member| member.test_count < MIN_CRATE_TESTS)
            .map(|member| format!("{} ({})", member.name, member.test_count))
            .collect();

        if minimal.is_empty() {
            CheckOutcome::pass("all first-class crates meet the minimum test baseline")
        } else {
            CheckOutcome::fail(format!(
                "{} crate(s) below the minimum of {MIN_CRATE_TESTS} tests",
                minimal.len()
            ))
            .with_detail(minimal.join(", "))
            .with_hint(
                "Add at least a few unit or integration tests for new crates before expanding their surface area.",
            )
        }
    }

    fn explain(&self) -> &'static str {
        "This is a structural baseline, not a coverage percentage. \
         It counts #[test] and #[tokio::test] occurrences under src/ and tests/ \
         for crates/ workspace members and warns when a crate falls below the minimum. \
         The goal is to prevent new crates from shipping with effectively no safety net."
    }
}

#[async_trait]
impl DoctorCheck for ArchOversizedModules {
    fn meta(&self) -> CheckMeta {
        CheckMeta {
            id: CheckId::new("ARC-003"),
            category: Category::Architecture,
            severity: Severity::Warning,
            name: "Oversized logic modules",
            description: "Large source files with substantial logic should be split before they become god modules.",
        }
    }

    async fn run(&self, _ctx: &CheckContext) -> CheckOutcome {
        let Some(root) = find_workspace_root() else {
            return CheckOutcome::skip("workspace Cargo.toml not found from current directory");
        };

        let workspace = match load_workspace_model(&root) {
            Ok(workspace) => workspace,
            Err(err) => return CheckOutcome::skip(err),
        };

        let mut flagged = Vec::new();
        for member in &workspace.members {
            if !member.path.starts_with("crates/") {
                continue;
            }
            flagged.extend(member.oversized_modules.iter().map(|signal| {
                format!(
                    "{}/{} ({} lines, {} fns)",
                    member.name,
                    signal.path.display(),
                    signal.line_count,
                    signal.function_count,
                )
            }));
        }

        if flagged.is_empty() {
            CheckOutcome::pass("no oversized logic-heavy modules detected")
        } else {
            CheckOutcome::fail(format!(
                "{} oversized logic module(s) detected",
                flagged.len()
            ))
            .with_detail(flagged.join("\n"))
            .with_hint(
                "Split orchestration, IO, domain policy, and protocol mapping into smaller modules before adding more behavior.",
            )
        }
    }

    fn explain(&self) -> &'static str {
        "This check scans src/**/*.rs under first-class crates and flags files with more than 500 lines \
         plus at least 5 function definitions, excluding clearly data-centric files such as types.rs \
         and config.rs. The result is a candidate list for review, not a proof of bad design."
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LayerInfo {
    index: usize,
    name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OversizedModule {
    path: PathBuf,
    line_count: usize,
    function_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkspaceMember {
    name: String,
    path: PathBuf,
    layer: Option<LayerInfo>,
    internal_dependencies: Vec<String>,
    test_count: usize,
    oversized_modules: Vec<OversizedModule>,
}

#[derive(Debug, Clone)]
struct WorkspaceModel {
    members: Vec<WorkspaceMember>,
    members_by_name: HashMap<String, WorkspaceMember>,
}

fn find_workspace_root() -> Option<PathBuf> {
    let mut dir = std::env::current_dir().ok()?;
    loop {
        let manifest = dir.join("Cargo.toml");
        if manifest.exists()
            && fs::read_to_string(&manifest)
                .ok()
                .is_some_and(|raw| raw.contains("[workspace]"))
        {
            return Some(dir);
        }
        if !dir.pop() {
            break;
        }
    }
    None
}

fn load_workspace_model(root: &Path) -> Result<WorkspaceModel, String> {
    let manifest_path = root.join("Cargo.toml");
    let raw = fs::read_to_string(&manifest_path)
        .map_err(|err| format!("cannot read {}: {err}", manifest_path.display()))?;
    let layer_map = parse_workspace_layers(&raw)?;
    let members = parse_workspace_members(&raw)?;

    let mut parsed_members = Vec::new();
    for member_path in members {
        let crate_dir = root.join(&member_path);
        let manifest = crate_dir.join("Cargo.toml");
        if !manifest.exists() {
            continue;
        }

        let member =
            parse_workspace_member(root, &member_path, &manifest, layer_map.get(&member_path))
                .map_err(|err| format!("{}: {err}", manifest.display()))?;
        parsed_members.push(member);
    }

    let members_by_name = parsed_members
        .iter()
        .cloned()
        .map(|member| (member.name.clone(), member))
        .collect();

    Ok(WorkspaceModel {
        members: parsed_members,
        members_by_name,
    })
}

fn parse_workspace_member(
    root: &Path,
    member_path: &Path,
    manifest_path: &Path,
    layer: Option<&LayerInfo>,
) -> Result<WorkspaceMember, String> {
    let raw =
        fs::read_to_string(manifest_path).map_err(|err| format!("cannot read manifest: {err}"))?;
    let value = raw
        .parse::<toml::Value>()
        .map_err(|err| format!("invalid Cargo.toml: {err}"))?;

    let name = value
        .get("package")
        .and_then(|package| package.get("name"))
        .and_then(toml::Value::as_str)
        .ok_or_else(|| "package.name not found".to_string())?
        .to_string();

    let crate_dir = root.join(member_path);
    Ok(WorkspaceMember {
        name,
        path: member_path.to_path_buf(),
        layer: layer.cloned(),
        internal_dependencies: collect_internal_dependencies(&value),
        test_count: count_tests_in_crate(&crate_dir)?,
        oversized_modules: find_oversized_modules(&crate_dir)?,
    })
}

fn parse_workspace_members(raw: &str) -> Result<Vec<PathBuf>, String> {
    let mut in_members = false;
    let mut members = Vec::new();

    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("members = [") {
            in_members = true;
            continue;
        }
        if in_members && trimmed == "]" {
            break;
        }
        if !in_members || trimmed.starts_with('#') || trimmed.is_empty() {
            continue;
        }

        if let Some(path) = extract_quoted_value(trimmed) {
            members.push(PathBuf::from(path));
        }
    }

    if members.is_empty() {
        Err("workspace members list is empty".to_string())
    } else {
        Ok(members)
    }
}

fn parse_workspace_layers(raw: &str) -> Result<HashMap<PathBuf, LayerInfo>, String> {
    let mut in_members = false;
    let mut current_layer = None;
    let mut layer_index = 0usize;
    let mut layers = HashMap::new();

    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("members = [") {
            in_members = true;
            continue;
        }
        if in_members && trimmed == "]" {
            break;
        }
        if !in_members {
            continue;
        }

        if let Some(layer_name) = parse_layer_heading(trimmed) {
            current_layer = Some(LayerInfo {
                index: layer_index,
                name: layer_name,
            });
            layer_index += 1;
            continue;
        }

        if let Some(path) = extract_quoted_value(trimmed)
            && let Some(layer) = &current_layer
        {
            layers.insert(PathBuf::from(path), layer.clone());
        }
    }

    if layers.is_empty() {
        Err("could not parse layer definitions from workspace members".to_string())
    } else {
        Ok(layers)
    }
}

fn parse_layer_heading(line: &str) -> Option<String> {
    if !line.starts_with("#") || !line.contains('─') {
        return None;
    }

    let without_hash = line.trim_start_matches('#').trim();
    let trimmed = without_hash.trim_matches('─').trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn extract_quoted_value(line: &str) -> Option<&str> {
    let start = line.find('"')?;
    let rest = &line[start + 1..];
    let end = rest.find('"')?;
    Some(&rest[..end])
}

fn collect_internal_dependencies(value: &toml::Value) -> Vec<String> {
    let mut dependencies = BTreeSet::new();
    collect_dependencies_from_table(value.get("dependencies"), &mut dependencies);
    collect_dependencies_from_table(value.get("build-dependencies"), &mut dependencies);

    if let Some(targets) = value.get("target").and_then(toml::Value::as_table) {
        for target in targets.values() {
            collect_dependencies_from_table(target.get("dependencies"), &mut dependencies);
            collect_dependencies_from_table(target.get("build-dependencies"), &mut dependencies);
        }
    }

    dependencies.into_iter().collect()
}

fn collect_dependencies_from_table(
    value: Option<&toml::Value>,
    dependencies: &mut BTreeSet<String>,
) {
    let Some(table) = value.and_then(toml::Value::as_table) else {
        return;
    };

    for key in table.keys() {
        if key.starts_with("orka-") {
            dependencies.insert(key.to_string());
        }
    }
}

fn count_tests_in_crate(crate_dir: &Path) -> Result<usize, String> {
    let mut total = 0usize;
    for dir_name in ["src", "tests"] {
        let dir = crate_dir.join(dir_name);
        total += visit_rust_files(&dir, &mut |path| {
            let content = fs::read_to_string(path).map_err(|err| err.to_string())?;
            Ok(count_test_attributes(&content))
        })?;
    }
    Ok(total)
}

fn count_test_attributes(content: &str) -> usize {
    content.matches("#[test]").count() + content.matches("#[tokio::test]").count()
}

fn find_oversized_modules(crate_dir: &Path) -> Result<Vec<OversizedModule>, String> {
    let src_dir = crate_dir.join("src");
    let fn_regex = Regex::new(r"(?m)^\s*(pub\s+)?(async\s+)?fn\s+[a-zA-Z0-9_]+")
        .map_err(|err| err.to_string())?;
    let mut modules = Vec::new();

    visit_rust_files(&src_dir, &mut |path| {
        let content = fs::read_to_string(path).map_err(|err| err.to_string())?;
        let line_count = content.lines().count();
        if line_count <= MAX_MODULE_LINES || is_data_like_file(path) {
            return Ok(0usize);
        }

        let function_count = fn_regex.find_iter(&content).count();
        if function_count >= MIN_FUNCTIONS_FOR_MODULE_WARNING {
            let relative = path.strip_prefix(crate_dir).unwrap_or(path).to_path_buf();
            modules.push(OversizedModule {
                path: relative,
                line_count,
                function_count,
            });
        }

        Ok(0usize)
    })?;

    Ok(modules)
}

fn is_data_like_file(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some("types.rs" | "config.rs" | "testing.rs" | "error.rs")
    )
}

fn visit_rust_files(
    dir: &Path,
    visitor: &mut dyn FnMut(&Path) -> Result<usize, String>,
) -> Result<usize, String> {
    if !dir.exists() {
        return Ok(0);
    }

    let mut total = 0usize;
    let entries = fs::read_dir(dir).map_err(|err| err.to_string())?;
    for entry in entries {
        let entry = entry.map_err(|err| err.to_string())?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|err| err.to_string())?;

        if file_type.is_dir() {
            total += visit_rust_files(&path, visitor)?;
            continue;
        }

        if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            total += visitor(&path)?;
        }
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_layer_heading_extracts_name() {
        assert_eq!(
            parse_layer_heading("# ── AI / Intelligence ─────────────────────"),
            Some("AI / Intelligence".to_string())
        );
    }

    #[test]
    fn count_test_attributes_counts_sync_and_tokio() {
        let content = "#[test]\nfn a() {}\n#[tokio::test]\nasync fn b() {}\n";
        assert_eq!(count_test_attributes(content), 2);
    }

    #[test]
    fn parse_workspace_layers_assigns_indices_from_comments() {
        let raw = r#"
[workspace]
members = [
  # ── Core ─────────────────
  "crates/orka-core",
  # ── Orchestration ────────
  "crates/orka-agent",
]
"#;
        let layers = parse_workspace_layers(raw).expect("layers");
        assert_eq!(layers[Path::new("crates/orka-core")].index, 0);
        assert_eq!(layers[Path::new("crates/orka-agent")].index, 1);
    }

    #[test]
    fn collect_internal_dependencies_reads_workspace_crates() {
        let value = r#"
[package]
name = "orka-demo"

[dependencies]
orka-core.workspace = true
serde.workspace = true
orka-llm = { workspace = true }
"#
        .parse::<toml::Value>()
        .expect("toml");
        let deps = collect_internal_dependencies(&value);
        assert_eq!(deps, vec!["orka-core".to_string(), "orka-llm".to_string()]);
    }
}
