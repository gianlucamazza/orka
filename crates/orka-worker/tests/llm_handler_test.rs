use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use orka_core::testing::{InMemoryEventSink, InMemoryMemoryStore, InMemorySecretManager};
use orka_core::traits::MemoryStore;
use orka_core::{Envelope, Error, Payload, Session};
use orka_llm::client::{ChatMessage, ChatContent, ChatMessageExt, LlmClient};
use orka_skills::SkillRegistry;
use orka_worker::{AgentHandler, WorkspaceHandler};
use orka_workspace::config::SoulFrontmatter;
use orka_workspace::parse::Document;
use orka_workspace::state::WorkspaceState;
use tokio::sync::RwLock;

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
    async fn complete(&self, _messages: Vec<ChatMessage>, _system: &str) -> orka_core::Result<String> {
        if self.should_fail.load(Ordering::SeqCst) {
            Err(Error::Other("LLM mock failure".into()))
        } else {
            Ok(self.response.clone())
        }
    }
}

fn test_workspace_state(name: &str, body: &str) -> Arc<RwLock<WorkspaceState>> {
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
    Arc::new(RwLock::new(state))
}

fn make_handler(llm: Option<Arc<dyn LlmClient>>) -> WorkspaceHandler {
    let state = test_workspace_state("TestBot", "You are a helpful bot.");
    let skills = Arc::new(SkillRegistry::new());
    let memory = Arc::new(InMemoryMemoryStore::new());
    let secrets = Arc::new(InMemorySecretManager::new());
    WorkspaceHandler::new(state, skills, memory, secrets, llm, Arc::new(InMemoryEventSink::new()), 128_000, None)
}

#[tokio::test]
async fn llm_responds_with_reply() {
    let llm = Arc::new(MockLlmClient::new("Hello from LLM!"));
    let handler = make_handler(Some(llm));

    let session = Session::new("custom", "user1");
    let envelope = Envelope::text("custom", session.id.clone(), "hi");

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
    let handler = make_handler(Some(llm));

    let session = Session::new("custom", "user1");
    let envelope = Envelope::text("custom", session.id.clone(), "hi");

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
    let state = test_workspace_state("Bot", "");
    let skills = Arc::new(SkillRegistry::new());
    let secrets = Arc::new(InMemorySecretManager::new());
    let handler = WorkspaceHandler::new(
        state,
        skills,
        memory.clone(),
        secrets,
        Some(llm),
        Arc::new(InMemoryEventSink::new()),
        128_000,
        None,
    );

    let session = Session::new("custom", "user1");
    let env1 = Envelope::text("custom", session.id.clone(), "first message");
    let env2 = Envelope::text("custom", session.id.clone(), "second message");

    // First turn
    handler.handle(&env1, &session).await.unwrap();

    // Second turn
    handler.handle(&env2, &session).await.unwrap();

    // Verify history was saved in memory
    let memory_key = format!("conversation:{}", session.id);
    let entry = memory.recall(&memory_key).await.unwrap().unwrap();
    let history: Vec<ChatMessageExt> = serde_json::from_value(entry.value).unwrap();

    // Should have 4 messages: user1, assistant1, user2, assistant2
    assert_eq!(history.len(), 4);
    assert_eq!(history[0].role, "user");
    assert!(matches!(&history[0].content, ChatContent::Text(t) if t == "first message"));
    assert_eq!(history[1].role, "assistant");
    assert!(matches!(&history[1].content, ChatContent::Text(t) if t == "response"));
    assert_eq!(history[2].role, "user");
    assert!(matches!(&history[2].content, ChatContent::Text(t) if t == "second message"));
    assert_eq!(history[3].role, "assistant");
    assert!(matches!(&history[3].content, ChatContent::Text(t) if t == "response"));
}
