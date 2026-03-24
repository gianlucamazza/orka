use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use orka_core::Result;
use orka_experience::{
    collector::TrajectoryCollector,
    service::ExperienceService,
    store::PrincipleStore,
    trajectory_store::TrajectoryStore,
    types::{OutcomeSignal, PrincipleKind},
};
use orka_knowledge::{
    embeddings::EmbeddingProvider, types::SearchResult, vector_store::VectorStore,
};
use orka_llm::client::{ChatMessage, CompletionOptions, LlmClient};

// ---------------------------------------------------------------------------
// Mock infrastructure
// ---------------------------------------------------------------------------

/// Embedding provider that returns a fixed-size zero vector.
struct MockEmbeddings;

#[async_trait]
impl EmbeddingProvider for MockEmbeddings {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|_| vec![0.0f32; 4]).collect())
    }
    fn dimensions(&self) -> usize {
        4
    }
}

/// In-memory vector store.
struct MockVectorStore {
    data: tokio::sync::Mutex<Vec<HashMap<String, String>>>,
}

impl MockVectorStore {
    fn new() -> Self {
        Self {
            data: tokio::sync::Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl VectorStore for MockVectorStore {
    async fn ensure_collection(&self, _name: &str, _dimensions: usize) -> Result<()> {
        Ok(())
    }

    async fn upsert(
        &self,
        _collection: &str,
        ids: &[String],
        _vectors: &[Vec<f32>],
        payloads: &[HashMap<String, String>],
    ) -> Result<()> {
        let mut data = self.data.lock().await;
        for (id, payload) in ids.iter().zip(payloads.iter()) {
            let mut entry = payload.clone();
            entry.insert("_id".into(), id.clone());
            data.push(entry);
        }
        Ok(())
    }

    async fn search(
        &self,
        _collection: &str,
        _vector: &[f32],
        limit: usize,
        _score_threshold: Option<f32>,
        filter: Option<HashMap<String, String>>,
    ) -> Result<Vec<SearchResult>> {
        let data = self.data.lock().await;
        let results = data
            .iter()
            .filter(|p| {
                filter
                    .as_ref()
                    .is_none_or(|f| f.iter().all(|(k, v)| p.get(k).is_some_and(|pv| pv == v)))
            })
            .take(limit)
            .map(|p| SearchResult {
                content: p.get("text").cloned().unwrap_or_default(),
                score: 0.9,
                document_id: p.get("_id").cloned(),
                metadata: p.clone(),
            })
            .collect();
        Ok(results)
    }

    async fn list_documents(
        &self,
        _collection: &str,
        limit: usize,
        filter: Option<HashMap<String, String>>,
    ) -> Result<Vec<HashMap<String, String>>> {
        let data = self.data.lock().await;
        let iter = data.iter().filter(|entry| {
            filter.as_ref().is_none_or(|f| {
                f.iter()
                    .all(|(k, v)| entry.get(k).is_some_and(|ev| ev == v))
            })
        });
        Ok(iter.take(limit).cloned().collect())
    }
}

/// LLM that returns a fixed reflection response.
struct MockLlm {
    response: String,
}

#[async_trait]
impl LlmClient for MockLlm {
    async fn complete(&self, _messages: Vec<ChatMessage>, _system: &str) -> Result<String> {
        Ok(self.response.clone())
    }

    async fn complete_with_options(
        &self,
        _messages: Vec<ChatMessage>,
        _system: &str,
        _options: CompletionOptions,
    ) -> Result<String> {
        Ok(self.response.clone())
    }

    async fn complete_stream(
        &self,
        _messages: Vec<ChatMessage>,
        _system: &str,
    ) -> Result<orka_llm::LlmStream> {
        Err(orka_core::Error::Other("not implemented".into()))
    }

    async fn complete_with_tools(
        &self,
        _messages: &[orka_llm::ChatMessage],
        _system: &str,
        _tools: &[orka_llm::ToolDefinition],
        _options: CompletionOptions,
    ) -> Result<orka_llm::CompletionResponse> {
        Err(orka_core::Error::Other("not implemented".into()))
    }

    async fn complete_stream_with_tools(
        &self,
        _messages: &[orka_llm::ChatMessage],
        _system: &str,
        _tools: &[orka_llm::ToolDefinition],
        _options: CompletionOptions,
    ) -> Result<orka_llm::LlmToolStream> {
        Err(orka_core::Error::Other("not implemented".into()))
    }
}

fn make_service(llm_response: &str) -> ExperienceService {
    let embeddings = Arc::new(MockEmbeddings);
    let vector_store = Arc::new(MockVectorStore::new());
    let llm = Arc::new(MockLlm {
        response: llm_response.to_string(),
    });
    let mut config = orka_core::config::ExperienceConfig::default();
    config.enabled = true;
    config.reflect_on = "all".into();
    let principle_store = Arc::new(PrincipleStore::new(
        embeddings.clone(),
        vector_store.clone(),
        "test_principles".into(),
    ));
    let trajectory_store = Arc::new(TrajectoryStore::new(
        embeddings,
        vector_store,
        "test_trajectories".into(),
    ));
    ExperienceService::new(principle_store, trajectory_store, llm, config)
}

#[test]
fn trajectory_collector_full_lifecycle() {
    let mut collector = TrajectoryCollector::new(
        "session-1".into(),
        "default".into(),
        "What's the weather?".into(),
    );

    collector.record_skill("web_search".into(), 150, true, None, None);
    collector.record_iteration(1200);
    collector.record_skill("summarize".into(), 50, true, None, None);
    collector.record_iteration(800);
    collector.set_response("It's sunny today.".into());

    assert!(matches!(collector.outcome(), OutcomeSignal::Success));

    let trajectory = collector.finish();

    assert!(trajectory.success);
    assert_eq!(trajectory.iterations, 2);
    assert_eq!(trajectory.total_tokens, 2000);
    assert_eq!(trajectory.skills_used.len(), 2);
    assert_eq!(trajectory.skills_used[0].name, "web_search");
    assert_eq!(trajectory.skills_used[1].name, "summarize");
    assert!(trajectory.errors.is_empty());
    assert_eq!(trajectory.agent_response, "It's sunny today.");
    assert_eq!(trajectory.workspace, "default");
}

#[test]
fn trajectory_collector_failure_path() {
    let mut collector = TrajectoryCollector::new(
        "session-2".into(),
        "support".into(),
        "Delete everything".into(),
    );

    collector.record_skill("shell_exec".into(), 200, false, None, None);
    collector.record_error("permission denied".into());
    collector.record_iteration(500);
    collector.set_response("I can't do that.".into());

    assert!(matches!(collector.outcome(), OutcomeSignal::Failure));

    let trajectory = collector.finish();

    assert!(!trajectory.success);
    assert_eq!(trajectory.errors.len(), 1);
    assert_eq!(trajectory.errors[0], "permission denied");
}

#[test]
fn format_principles_empty() {
    let section = ExperienceService::format_principles_section(&[]);
    assert!(section.is_empty());
}

#[test]
fn format_principles_mixed() {
    use chrono::Utc;

    let principles = vec![
        orka_experience::Principle {
            id: "p1".into(),
            text: "Use web_search for current info.".into(),
            kind: PrincipleKind::Do,
            scope: "default".into(),
            created_at: Utc::now(),
            reinforcement_count: 0,
            relevance_score: 0.9,
        },
        orka_experience::Principle {
            id: "p2".into(),
            text: "Avoid shell_exec for read-only queries.".into(),
            kind: PrincipleKind::Avoid,
            scope: "default".into(),
            created_at: Utc::now(),
            reinforcement_count: 2,
            relevance_score: 0.8,
        },
    ];

    let section = ExperienceService::format_principles_section(&principles);
    assert!(section.contains("Learned Principles"));
    assert!(section.contains("[DO] Use web_search"));
    assert!(section.contains("[AVOID] Avoid shell_exec"));
    assert!(section.contains("1."));
    assert!(section.contains("2."));
}

// ---------------------------------------------------------------------------
// Async tests using mock infrastructure
// ---------------------------------------------------------------------------

#[tokio::test]
async fn record_trajectory_succeeds() {
    let svc = make_service("[]");
    let mut collector =
        TrajectoryCollector::new("sess-async-1".into(), "default".into(), "hello".into());
    collector.record_skill("web_search".into(), 50, true, None, None);
    collector.record_iteration(300);
    collector.set_response("Hi!".into());
    let trajectory = collector.finish();

    let result = svc.record_trajectory(&trajectory).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn maybe_reflect_returns_zero_on_empty_llm_response() {
    let svc = make_service("[]");
    let mut collector = TrajectoryCollector::new(
        "sess-async-2".into(),
        "default".into(),
        "do something".into(),
    );
    collector.record_skill("shell_exec".into(), 100, false, None, None);
    collector.record_error("permission denied".into());
    collector.record_iteration(400);
    collector.set_response("Sorry.".into());
    let trajectory = collector.finish();

    let result = svc.maybe_reflect(&trajectory).await.unwrap();
    assert_eq!(result.principles_created, 0);
}

#[tokio::test]
async fn maybe_reflect_stores_principles_from_llm() {
    let llm_resp = r#"[{"text": "Use web_search before summarize", "kind": "do"}]"#;
    let svc = make_service(llm_resp);
    let mut collector = TrajectoryCollector::new(
        "sess-async-3".into(),
        "default".into(),
        "search query".into(),
    );
    collector.record_skill("web_search".into(), 80, true, None, None);
    collector.record_iteration(500);
    collector.set_response("Here is the result.".into());
    let trajectory = collector.finish();

    let result = svc.maybe_reflect(&trajectory).await.unwrap();
    assert_eq!(result.principles_created, 1);
}

#[tokio::test]
async fn distill_returns_zero_when_no_trajectories() {
    let svc = make_service("[]");
    let count = svc.distill("empty_workspace").await.unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn record_then_distill_flow() {
    let llm_resp = r#"[{"text": "Always check permissions first", "kind": "do"}]"#;
    let svc = make_service(llm_resp);

    // Record two trajectories
    for i in 0..2 {
        let mut collector = TrajectoryCollector::new(
            format!("sess-distill-{i}"),
            "default".into(),
            format!("query {i}"),
        );
        collector.record_skill("shell_exec".into(), 50, true, None, None);
        collector.record_iteration(200);
        collector.set_response("Done.".into());
        let trajectory = collector.finish();
        svc.record_trajectory(&trajectory).await.unwrap();
    }

    let count = svc.distill("default").await.unwrap();
    assert_eq!(count, 1);
}
