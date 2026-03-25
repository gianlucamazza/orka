//! Outbound A2A discovery client.
//!
//! Polls known agent endpoints at a configurable interval, fetches their
//! `/.well-known/agent.json` cards, and keeps an in-memory directory that
//! route handlers can query.

use std::{collections::HashMap, sync::Arc, time::Duration};

use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use crate::types::AgentCard;

// ── AgentDirectory
// ────────────────────────────────────────────────────────────

/// Thread-safe, in-memory cache of discovered [`AgentCard`]s keyed by base URL.
#[derive(Debug, Default)]
pub struct AgentDirectory {
    cards: RwLock<HashMap<String, AgentCard>>,
}

impl AgentDirectory {
    /// Create an empty directory.
    pub fn new() -> Self {
        Self::default()
    }

    /// Store or replace the card for `base_url`.
    pub async fn upsert(&self, base_url: String, card: AgentCard) {
        self.cards.write().await.insert(base_url, card);
    }

    /// Retrieve the card for `base_url`, if present.
    pub async fn get(&self, base_url: &str) -> Option<AgentCard> {
        self.cards.read().await.get(base_url).cloned()
    }

    /// Return all cached cards.
    pub async fn all(&self) -> Vec<AgentCard> {
        self.cards.read().await.values().cloned().collect()
    }
}

// ── DiscoveryClient
// ───────────────────────────────────────────────────────────

/// Polls a list of known agent base URLs and refreshes [`AgentDirectory`].
///
/// Each poll cycle GETs `{base_url}/.well-known/agent.json` for every
/// configured agent. Unreachable or invalid endpoints emit a warning and are
/// skipped — the directory retains the last successful card.
pub struct DiscoveryClient {
    known_agents: Vec<String>,
    interval: Duration,
    client: reqwest::Client,
    directory: Arc<AgentDirectory>,
}

impl DiscoveryClient {
    /// Create a new client.
    ///
    /// `known_agents` — base URLs of agents to discover (e.g. `"http://agent:8080"`).
    /// `interval_secs` — polling interval; pass `0` to disable.
    pub fn new(
        known_agents: Vec<String>,
        interval_secs: u64,
        directory: Arc<AgentDirectory>,
    ) -> Self {
        Self {
            known_agents,
            interval: Duration::from_secs(interval_secs.max(1)),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .expect("reqwest client is always buildable"),
            directory,
        }
    }

    /// Run the discovery loop until `cancel` is triggered.
    pub async fn run(self, cancel: CancellationToken) {
        loop {
            self.poll_all().await;

            tokio::select! {
                _ = tokio::time::sleep(self.interval) => {}
                _ = cancel.cancelled() => {
                    debug!("discovery client shutting down");
                    break;
                }
            }
        }
    }

    /// Fetch cards for all known agents once.
    async fn poll_all(&self) {
        for base_url in &self.known_agents {
            let url = format!("{base_url}/.well-known/agent.json");
            match self.client.get(&url).send().await {
                Ok(resp) if resp.status().is_success() => match resp.json::<AgentCard>().await {
                    Ok(card) => {
                        debug!(%base_url, name = %card.name, "discovered agent");
                        self.directory.upsert(base_url.clone(), card).await;
                    }
                    Err(e) => {
                        warn!(%base_url, %e, "failed to parse agent card");
                    }
                },
                Ok(resp) => {
                    warn!(%base_url, status = %resp.status(), "agent card endpoint returned error");
                }
                Err(e) => {
                    warn!(%base_url, %e, "failed to reach agent for discovery");
                }
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::types::{AgentCard, InterfaceCapabilities, SupportedInterface};

    fn sample_card(name: &str) -> AgentCard {
        AgentCard {
            name: name.to_string(),
            description: "test".to_string(),
            version: "1.0.0".to_string(),
            supported_interfaces: vec![SupportedInterface {
                uri: "http://example.com/a2a".to_string(),
                protocol_version: "1.0".to_string(),
                capabilities: InterfaceCapabilities {
                    streaming: true,
                    push_notifications: false,
                    state_transition_history: false,
                },
                security_schemes: HashMap::new(),
            }],
            skills: vec![],
            default_input_modes: vec![],
            default_output_modes: vec![],
            metadata: None,
        }
    }

    #[tokio::test]
    async fn agent_directory_upsert_and_get() {
        let dir = AgentDirectory::new();
        dir.upsert("http://agent1".to_string(), sample_card("agent1"))
            .await;
        let card = dir.get("http://agent1").await.unwrap();
        assert_eq!(card.name, "agent1");
    }

    #[tokio::test]
    async fn agent_directory_get_missing_returns_none() {
        let dir = AgentDirectory::new();
        assert!(dir.get("http://nonexistent").await.is_none());
    }

    #[tokio::test]
    async fn agent_directory_all_returns_all_cards() {
        let dir = AgentDirectory::new();
        dir.upsert("http://a".to_string(), sample_card("alpha"))
            .await;
        dir.upsert("http://b".to_string(), sample_card("beta"))
            .await;
        let all = dir.all().await;
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn agent_directory_upsert_replaces() {
        let dir = AgentDirectory::new();
        dir.upsert("http://a".to_string(), sample_card("v1")).await;
        dir.upsert("http://a".to_string(), sample_card("v2")).await;
        let card = dir.get("http://a").await.unwrap();
        assert_eq!(card.name, "v2");
        assert_eq!(dir.all().await.len(), 1);
    }
}
