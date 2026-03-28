//! Integration tests for orka-prompts: template engine, registry, pipeline,
//! context providers and coordinator.

use std::collections::HashMap;

use orka_prompts::pipeline::StaticSection;
use orka_prompts::{
    BuildContext, PipelineConfig, PromptSection, SystemPromptPipeline, TemplateEngine,
    TemplateRegistry,
    context::{
        ContextCoordinator, SectionsContextProvider, SessionContext, ShellContextProvider,
        WorkspaceProvider,
    },
};

// ---------------------------------------------------------------------------
// TemplateEngine
// ---------------------------------------------------------------------------

#[test]
fn engine_renders_variable() {
    let mut engine = TemplateEngine::new();
    engine
        .register_template("hello", "Hello, {{name}}!")
        .unwrap();
    let ctx = serde_json::json!({ "name": "Orka" });
    let out = engine.render("hello", &ctx).unwrap();
    assert_eq!(out, "Hello, Orka!");
}

#[test]
fn engine_missing_template_returns_error() {
    let engine = TemplateEngine::new();
    let ctx = serde_json::json!({});
    let err = engine.render("nonexistent", &ctx);
    assert!(err.is_err());
    assert!(err.unwrap_err().to_string().contains("nonexistent"));
}

#[test]
fn engine_has_template_after_register() {
    let mut engine = TemplateEngine::new();
    assert!(!engine.has_template("t1"));
    engine.register_template("t1", "content").unwrap();
    assert!(engine.has_template("t1"));
}

#[test]
fn engine_unregister_removes_template() {
    let mut engine = TemplateEngine::new();
    engine.register_template("t2", "content").unwrap();
    engine.unregister_template("t2");
    assert!(!engine.has_template("t2"));
}

#[test]
fn engine_join_helper() {
    let mut engine = TemplateEngine::new();
    engine
        .register_template("list", r#"{{join items ", "}}"#)
        .unwrap();
    let ctx = serde_json::json!({ "items": ["a", "b", "c"] });
    let out = engine.render("list", &ctx).unwrap();
    assert_eq!(out, "a, b, c");
}

#[test]
fn engine_rejects_empty_template_name() {
    let mut engine = TemplateEngine::new();
    assert!(engine.register_template("", "content").is_err());
}

#[test]
fn engine_allows_path_like_names() {
    let mut engine = TemplateEngine::new();
    engine
        .register_template("system/reflection", "{{thought}}")
        .unwrap();
    let ctx = serde_json::json!({ "thought": "ok" });
    let out = engine.render("system/reflection", &ctx).unwrap();
    assert_eq!(out, "ok");
}

// ---------------------------------------------------------------------------
// TemplateRegistry (async)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn registry_register_and_render() {
    let reg = TemplateRegistry::new();
    reg.register_inline("greet", "Hi {{name}}.").await.unwrap();
    let ctx = serde_json::json!({ "name": "World" });
    let out = reg.render("greet", &ctx).await.unwrap();
    assert_eq!(out, "Hi World.");
}

#[tokio::test]
async fn registry_missing_template_errors() {
    let reg = TemplateRegistry::new();
    let ctx = serde_json::json!({});
    assert!(reg.render("ghost", &ctx).await.is_err());
}

#[tokio::test]
async fn registry_override_template() {
    let reg = TemplateRegistry::new();
    reg.register_inline("t", "v1").await.unwrap();
    reg.register_inline("t", "v2").await.unwrap();
    let ctx = serde_json::json!({});
    let out = reg.render("t", &ctx).await.unwrap();
    assert_eq!(out, "v2");
}

// ---------------------------------------------------------------------------
// PipelineConfig
// ---------------------------------------------------------------------------

#[test]
fn pipeline_config_defaults_are_sensible() {
    let cfg = PipelineConfig::default();
    assert!(!cfg.sections.is_empty());
    assert!(!cfg.section_separator.is_empty());
    assert!(cfg.max_principles > 0);
    assert!(cfg.include_summary);
    assert!(cfg.include_datetime);
    assert!(!cfg.timezone.is_empty());
}

#[test]
fn pipeline_config_deserialize_custom() {
    let toml_str = r#"
        sections = ["persona", "tools"]
        section_separator = "\n---\n"
        max_principles = 3
        include_summary = false
        include_datetime = false
    "#;
    let cfg: PipelineConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(cfg.sections, vec!["persona", "tools"]);
    assert_eq!(cfg.section_separator, "\n---\n");
    assert_eq!(cfg.max_principles, 3);
    assert!(!cfg.include_summary);
    assert!(!cfg.include_datetime);
}

// ---------------------------------------------------------------------------
// StaticSection
// ---------------------------------------------------------------------------

#[tokio::test]
async fn static_section_renders_content() {
    let section = StaticSection::new("intro", "You are Orka.");
    let ctx = BuildContext::default();
    let out = section.render(&ctx).await.unwrap();
    assert_eq!(out, Some("You are Orka.".to_string()));
}

#[tokio::test]
async fn static_section_empty_returns_none() {
    let section = StaticSection::new("empty", "");
    let ctx = BuildContext::default();
    let out = section.render(&ctx).await.unwrap();
    assert_eq!(out, None);
}

#[tokio::test]
async fn static_section_required_empty_returns_some() {
    let section = StaticSection::new("req", "").required();
    let ctx = BuildContext::default();
    assert!(section.is_required());
    let out = section.render(&ctx).await.unwrap();
    assert_eq!(out, Some("".to_string()));
}

// ---------------------------------------------------------------------------
// SystemPromptPipeline
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pipeline_builds_prompt_with_persona() {
    let cfg = PipelineConfig {
        sections: vec!["persona".to_string()],
        ..Default::default()
    };
    let pipeline = SystemPromptPipeline::from_config(&cfg);
    let ctx = BuildContext::new("TestAgent").with_persona("I am a helpful assistant.");
    let prompt = pipeline.build(&ctx).await.unwrap();
    assert!(prompt.contains("helpful assistant"));
}

#[tokio::test]
async fn pipeline_skips_empty_optional_sections_without_panic() {
    let cfg = PipelineConfig {
        sections: vec!["persona".to_string(), "tools".to_string()],
        ..Default::default()
    };
    let pipeline = SystemPromptPipeline::from_config(&cfg);
    let ctx = BuildContext::new("Agent").with_persona("Hello.");
    let prompt = pipeline.build(&ctx).await.unwrap();
    assert!(prompt.contains("Hello."));
}

#[tokio::test]
async fn pipeline_with_explicit_sections_respects_order() {
    let cfg = PipelineConfig {
        section_separator: "||".to_string(),
        sections: vec!["a".to_string(), "b".to_string()],
        ..Default::default()
    };
    let sections: Vec<Box<dyn orka_prompts::PromptSection>> = vec![
        Box::new(StaticSection::new("a", "FIRST")),
        Box::new(StaticSection::new("b", "SECOND")),
    ];
    let pipeline = SystemPromptPipeline::with_sections(sections, cfg);
    let ctx = BuildContext::default();
    let prompt = pipeline.build(&ctx).await.unwrap();
    assert!(prompt.find("FIRST").unwrap() < prompt.find("SECOND").unwrap());
    assert!(prompt.contains("||"));
}

// ---------------------------------------------------------------------------
// Context providers
// ---------------------------------------------------------------------------

#[tokio::test]
async fn workspace_provider_returns_correct_fields() {
    use orka_prompts::ContextProvider;
    let provider = WorkspaceProvider::new(vec!["default".to_string(), "other".to_string()]);
    let session = SessionContext {
        workspace: "default".to_string(),
        cwd: Some("/home/gianluca".to_string()),
        ..Default::default()
    };
    let value = provider.provide(&session).await.unwrap();
    let ws = value["workspace"].as_object().unwrap();
    assert_eq!(ws["name"].as_str(), Some("default"));
    assert_eq!(ws["cwd"].as_str(), Some("/home/gianluca"));
    let available: Vec<&str> = ws["available"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(available, vec!["default", "other"]);
}

#[tokio::test]
async fn sections_provider_returns_all_sections() {
    use orka_prompts::ContextProvider;
    let mut map = HashMap::new();
    map.insert("custom_a".to_string(), "value_a".to_string());
    map.insert("custom_b".to_string(), "value_b".to_string());
    let provider = SectionsContextProvider::new(map);
    let session = SessionContext::default();
    let value = provider.provide(&session).await.unwrap();
    let obj = value.as_object().unwrap();
    assert_eq!(obj["custom_a"].as_str(), Some("value_a"));
    assert_eq!(obj["custom_b"].as_str(), Some("value_b"));
}

#[tokio::test]
async fn shell_provider_empty_when_no_commands() {
    use orka_prompts::ContextProvider;
    let provider = ShellContextProvider::new();
    let session = SessionContext::default();
    let value = provider.provide(&session).await.unwrap();
    assert!(value.as_object().map(|o| o.is_empty()).unwrap_or(true));
}

#[tokio::test]
async fn shell_provider_returns_commands_from_metadata() {
    use orka_prompts::ContextProvider;
    let provider = ShellContextProvider::new();
    let mut session = SessionContext::default();
    session.metadata.insert(
        "shell:recent_commands".to_string(),
        serde_json::Value::String("ls -la\ncd /tmp".to_string()),
    );
    let value = provider.provide(&session).await.unwrap();
    let content = value["shell_commands"]["content"].as_str().unwrap();
    assert!(content.contains("ls -la"));
}

// ---------------------------------------------------------------------------
// ContextCoordinator
// ---------------------------------------------------------------------------

#[tokio::test]
async fn coordinator_merges_workspace_provider() {
    let mut sections = HashMap::new();
    sections.insert("extra".to_string(), "extra_value".to_string());

    let base = BuildContext::new("Bot");
    let coordinator = ContextCoordinator::new(base)
        .with_provider(Box::new(WorkspaceProvider::new(vec!["ws1".to_string()])))
        .with_provider(Box::new(SectionsContextProvider::new(sections)));

    let session = SessionContext {
        workspace: "ws1".to_string(),
        ..Default::default()
    };

    let ctx = coordinator.build(&session).await.unwrap();
    assert_eq!(ctx.workspace_name, "ws1");
    assert_eq!(
        ctx.dynamic_sections.get("extra").map(String::as_str),
        Some("extra_value")
    );
}

#[tokio::test]
async fn coordinator_provider_failure_does_not_abort() {
    use async_trait::async_trait;
    use orka_prompts::ContextProvider;

    struct FailingProvider;

    #[async_trait]
    impl ContextProvider for FailingProvider {
        fn provider_id(&self) -> &str {
            "failing"
        }
        async fn provide(&self, _ctx: &SessionContext) -> orka_core::Result<serde_json::Value> {
            Err(orka_core::Error::Other("intentional failure".into()))
        }
    }

    let base = BuildContext::new("Bot");
    let coordinator = ContextCoordinator::new(base).with_provider(Box::new(FailingProvider));

    let session = SessionContext::default();
    let ctx = coordinator.build(&session).await.unwrap();
    assert_eq!(ctx.agent_name, "Bot");
}
