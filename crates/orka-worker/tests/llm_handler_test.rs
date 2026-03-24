use std::{
    collections::HashSet,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use async_trait::async_trait;
use orka_core::{
    Envelope, Error, Payload, Session,
    config::AgentConfig,
    testing::{InMemoryEventSink, InMemoryMemoryStore, InMemorySecretManager},
    traits::MemoryStore,
};
use orka_llm::client::{ChatContent, ChatMessage, LlmClient, Role};
use orka_skills::SkillRegistry;
use orka_worker::{
    AgentHandler, CommandRegistry, StreamRegistry, WorkspaceHandler, WorkspaceHandlerConfig,
};
use orka_workspace::{
    WorkspaceLoader, WorkspaceRegistry, config::SoulFrontmatter, parse::Document,
    state::WorkspaceState,
};

struct MockLlmClient {
    response: String,
    should_fail: AtomicBool,
}

impl MockLlmClient {
    fn new(response: &str) -> Self {
        Self {
            response: response.to_string(),
            should_fail: AtomicBool::new(false),
        }
    }

    fn failing() -> Self {
        Self {
            response: String::new(),
            should_fail: AtomicBool::new(true),
        }
    }
}

#[async_trait]
impl LlmClient for MockLlmClient {
    async fn complete(
        &self,
        _messages: Vec<ChatMessage>,
        _system: &str,
    ) -> orka_core::Result<String> {
        if self.should_fail.load(Ordering::SeqCst) {
            Err(Error::Other("LLM mock failure".into()))
        } else {
            Ok(self.response.clone())
        }
    }
}

async fn test_workspace_registry(name: &str, body: &str) -> Arc<WorkspaceRegistry> {
    let loader = Arc::new(WorkspaceLoader::new("."));
    let state = WorkspaceState {
        soul: Some(Document {
            frontmatter: SoulFrontmatter {
                name: Some(name.to_string()),
                ..Default::default()
            },
            body: body.to_string(),
        }),
        ..Default::default()
    };
    let state_lock = loader.state();
    *state_lock.write().await = state;

    let mut registry = WorkspaceRegistry::new("default".into());
    registry.register("default".into(), loader);
    Arc::new(registry)
}

async fn make_handler(llm: Option<Arc<dyn LlmClient>>) -> WorkspaceHandler {
    let registry = test_workspace_registry("TestBot", "You are a helpful bot.").await;
    let skills = Arc::new(SkillRegistry::new());
    let memory = Arc::new(InMemoryMemoryStore::new());
    let secrets = Arc::new(InMemorySecretManager::new());
    WorkspaceHandler::new(
        registry,
        skills,
        memory,
        secrets,
        llm,
        Arc::new(InMemoryEventSink::new()),
        WorkspaceHandlerConfig {
            agent_config: AgentConfig::default(),
            disabled_tools: HashSet::new(),
            default_context_window: 128_000,
        },
        None,
        Arc::new(CommandRegistry::new()),
        StreamRegistry::new(),
        None,
    )
}

#[tokio::test]
async fn llm_responds_with_reply() {
    let llm = Arc::new(MockLlmClient::new("Hello from LLM!"));
    let handler = make_handler(Some(llm)).await;

    let session = Session::new("custom", "user1");
    let envelope = Envelope::text("custom", session.id, "hi");

    let replies = handler.handle(&envelope, &session).await.unwrap();
    assert_eq!(replies.len(), 1);
    match &replies[0].payload {
        Payload::Text(t) => assert_eq!(t, "Hello from LLM!"),
        _ => panic!("expected text"),
    }
}

#[tokio::test]
async fn llm_failure_falls_back_to_echo() {
    let llm = Arc::new(MockLlmClient::failing());
    let handler = make_handler(Some(llm)).await;

    let session = Session::new("custom", "user1");
    let envelope = Envelope::text("custom", session.id, "hi");

    let replies = handler.handle(&envelope, &session).await.unwrap();
    assert_eq!(replies.len(), 1);
    match &replies[0].payload {
        Payload::Text(t) => {
            // LLM error is surfaced to the user
            assert!(t.contains("Sorry, the LLM request failed"));
            assert!(t.contains("mock failure"));
        }
        _ => panic!("expected text"),
    }
}

#[tokio::test]
async fn multi_turn_conversation_saves_history() {
    let llm = Arc::new(MockLlmClient::new("response"));
    let memory = Arc::new(InMemoryMemoryStore::new());
    let registry = test_workspace_registry("Bot", "").await;
    let skills = Arc::new(SkillRegistry::new());
    let secrets = Arc::new(InMemorySecretManager::new());
    let handler = WorkspaceHandler::new(
        registry,
        skills,
        memory.clone(),
        secrets,
        Some(llm),
        Arc::new(InMemoryEventSink::new()),
        WorkspaceHandlerConfig {
            agent_config: AgentConfig::default(),
            disabled_tools: HashSet::new(),
            default_context_window: 128_000,
        },
        None,
        Arc::new(CommandRegistry::new()),
        StreamRegistry::new(),
        None,
    );

    let session = Session::new("custom", "user1");
    let env1 = Envelope::text("custom", session.id, "first message");
    let env2 = Envelope::text("custom", session.id, "second message");

    // First turn
    handler.handle(&env1, &session).await.unwrap();

    // Second turn
    handler.handle(&env2, &session).await.unwrap();

    // Verify history was saved in memory
    let memory_key = format!("conversation:{}", session.id);
    let entry = memory.recall(&memory_key).await.unwrap().unwrap();
    let history: Vec<ChatMessage> = serde_json::from_value(entry.value).unwrap();

    // Should have 4 messages: user1, assistant1, user2, assistant2
    assert_eq!(history.len(), 4);
    assert_eq!(history[0].role, Role::User);
    assert!(matches!(&history[0].content, ChatContent::Text(t) if t == "first message"));
    assert_eq!(history[1].role, Role::Assistant);
    assert!(matches!(&history[1].content, ChatContent::Text(t) if t == "response"));
    assert_eq!(history[2].role, Role::User);
    assert!(matches!(&history[2].content, ChatContent::Text(t) if t == "second message"));
    assert_eq!(history[3].role, Role::Assistant);
    assert!(matches!(&history[3].content, ChatContent::Text(t) if t == "response"));
}
