use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use orka_core::Result;
use orka_knowledge::embeddings::EmbeddingProvider;
use orka_knowledge::vector_store::VectorStore;
use tracing::debug;

use crate::types::{SkillTrace, Trajectory};

/// Persists raw trajectories in the vector store for offline distillation.
///
/// The user message is used as the embedding text so that semantically similar
/// trajectories can be found. All structured fields are stored in the payload.
pub struct TrajectoryStore {
    embeddings: Arc<dyn EmbeddingProvider>,
    vector_store: Arc<dyn VectorStore>,
    collection: String,
    initialized: tokio::sync::OnceCell<()>,
}

impl TrajectoryStore {
    /// Create a new trajectory store backed by the given vector store and embedding provider.
    pub fn new(
        embeddings: Arc<dyn EmbeddingProvider>,
        vector_store: Arc<dyn VectorStore>,
        collection: String,
    ) -> Self {
        Self {
            embeddings,
            vector_store,
            collection,
            initialized: tokio::sync::OnceCell::new(),
        }
    }

    async fn ensure_init(&self) -> Result<()> {
        self.initialized
            .get_or_try_init(|| async {
                let dims = self.embeddings.dimensions();
                self.vector_store
                    .ensure_collection(&self.collection, dims)
                    .await
            })
            .await?;
        Ok(())
    }

    /// Persist a trajectory. The user message is embedded for later similarity search.
    pub async fn store(&self, trajectory: &Trajectory) -> Result<()> {
        self.ensure_init().await?;

        let embeddings = self
            .embeddings
            .embed(std::slice::from_ref(&trajectory.user_message))
            .await?;
        let vector = embeddings
            .into_iter()
            .next()
            .ok_or_else(|| orka_core::Error::Knowledge("empty embedding result".into()))?;

        let payload = trajectory_to_payload(trajectory);

        self.vector_store
            .upsert(
                &self.collection,
                std::slice::from_ref(&trajectory.id),
                &[vector],
                &[payload],
            )
            .await?;

        debug!(id = %trajectory.id, workspace = %trajectory.workspace, "stored trajectory");
        Ok(())
    }

    /// Load up to `limit` recent trajectories, optionally filtered by workspace.
    pub async fn load_recent(
        &self,
        workspace: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Trajectory>> {
        self.ensure_init().await?;

        let payloads = self
            .vector_store
            .list_documents(&self.collection, limit)
            .await?;

        let mut trajectories: Vec<Trajectory> = payloads
            .into_iter()
            .filter_map(|p| payload_to_trajectory(&p))
            .filter(|t| workspace.is_none_or(|w| t.workspace == w))
            .collect();

        // Sort by timestamp descending (most recent first)
        trajectories.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        trajectories.truncate(limit);

        debug!(
            count = trajectories.len(),
            workspace = workspace.unwrap_or("*"),
            "loaded recent trajectories"
        );
        Ok(trajectories)
    }
}

fn trajectory_to_payload(t: &Trajectory) -> HashMap<String, String> {
    let mut m = HashMap::new();
    m.insert("id".into(), t.id.clone());
    m.insert("session_id".into(), t.session_id.clone());
    m.insert("workspace".into(), t.workspace.clone());
    m.insert("timestamp".into(), t.timestamp.to_rfc3339());
    m.insert("user_message".into(), truncate(&t.user_message, 500));
    m.insert("agent_response".into(), truncate(&t.agent_response, 500));
    m.insert("iterations".into(), t.iterations.to_string());
    m.insert("total_tokens".into(), t.total_tokens.to_string());
    m.insert("success".into(), t.success.to_string());
    m.insert("duration_ms".into(), t.duration_ms.to_string());
    // Serialize structured fields as JSON strings
    m.insert(
        "skills_used".into(),
        serde_json::to_string(&t.skills_used).unwrap_or_default(),
    );
    m.insert(
        "errors".into(),
        serde_json::to_string(&t.errors).unwrap_or_default(),
    );
    m
}

fn payload_to_trajectory(p: &HashMap<String, String>) -> Option<Trajectory> {
    let id = p.get("id")?.clone();
    let session_id = p.get("session_id").cloned().unwrap_or_default();
    let workspace = p
        .get("workspace")
        .cloned()
        .unwrap_or_else(|| "global".into());
    let timestamp = p
        .get("timestamp")
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(Utc::now);
    let user_message = p.get("user_message").cloned().unwrap_or_default();
    let agent_response = p.get("agent_response").cloned().unwrap_or_default();
    let iterations = p
        .get("iterations")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let total_tokens = p
        .get("total_tokens")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let success = p
        .get("success")
        .and_then(|s| s.parse().ok())
        .unwrap_or(false);
    let duration_ms = p
        .get("duration_ms")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let skills_used: Vec<SkillTrace> = p
        .get("skills_used")
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    let errors: Vec<String> = p
        .get("errors")
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();

    Some(Trajectory {
        id,
        session_id,
        workspace,
        timestamp,
        user_message,
        agent_response,
        skills_used,
        iterations,
        total_tokens,
        success,
        duration_ms,
        errors,
    })
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn make_trajectory(id: &str, workspace: &str, success: bool) -> Trajectory {
        Trajectory {
            id: id.into(),
            session_id: "sess".into(),
            workspace: workspace.into(),
            timestamp: Utc::now(),
            user_message: "hello".into(),
            agent_response: "hi".into(),
            skills_used: vec![SkillTrace {
                name: "web_search".into(),
                duration_ms: 100,
                success,
            }],
            iterations: 1,
            total_tokens: 500,
            success,
            duration_ms: 200,
            errors: if success { vec![] } else { vec!["err".into()] },
        }
    }

    #[test]
    fn round_trip_payload() {
        let t = make_trajectory("traj-1", "default", true);
        let payload = trajectory_to_payload(&t);
        let recovered = payload_to_trajectory(&payload).unwrap();

        assert_eq!(recovered.id, t.id);
        assert_eq!(recovered.workspace, t.workspace);
        assert_eq!(recovered.success, t.success);
        assert_eq!(recovered.iterations, t.iterations);
        assert_eq!(recovered.skills_used.len(), 1);
        assert_eq!(recovered.skills_used[0].name, "web_search");
        assert!(recovered.errors.is_empty());
    }

    #[test]
    fn round_trip_payload_failure() {
        let t = make_trajectory("traj-2", "support", false);
        let payload = trajectory_to_payload(&t);
        let recovered = payload_to_trajectory(&payload).unwrap();

        assert!(!recovered.success);
        assert_eq!(recovered.errors.len(), 1);
        assert_eq!(recovered.errors[0], "err");
    }

    #[test]
    fn truncate_long_string() {
        let long = "a".repeat(600);
        let result = truncate(&long, 500);
        assert!(result.len() <= 503); // 500 + "..."
        assert!(result.ends_with("..."));
    }

    #[test]
    fn truncate_short_string() {
        let short = "hello";
        assert_eq!(truncate(short, 500), "hello");
    }

    #[test]
    fn payload_missing_id_returns_none() {
        let p: HashMap<String, String> = HashMap::new();
        assert!(payload_to_trajectory(&p).is_none());
    }
}
