//! `CheckpointStore` trait and in-memory implementation for testing.

use async_trait::async_trait;
use orka_core::Result;

use crate::types::{Checkpoint, CheckpointId};

/// Pluggable persistence backend for execution checkpoints.
///
/// Implementations must be `Send + Sync + 'static` so they can be shared
/// across tokio tasks inside `Arc`.
///
/// # Ordering guarantee
///
/// [`list`](CheckpointStore::list) must return checkpoint IDs in
/// **oldest-first** order. Implementations backed by time-ordered keys (e.g.
/// `UUIDv7`) satisfy this naturally; others must sort explicitly.
#[async_trait]
pub trait CheckpointStore: Send + Sync + 'static {
    /// Persist a checkpoint. Overwrites any existing checkpoint with the same
    /// [`CheckpointId`].
    async fn save(&self, checkpoint: &Checkpoint) -> Result<()>;

    /// Load the most recently saved checkpoint for a run.
    ///
    /// Returns `None` when no checkpoint exists for `run_id`.
    async fn load_latest(&self, run_id: &str) -> Result<Option<Checkpoint>>;

    /// Load a specific checkpoint by its ID.
    ///
    /// Returns `None` when the checkpoint does not exist.
    async fn load(&self, run_id: &str, id: &CheckpointId) -> Result<Option<Checkpoint>>;

    /// List all checkpoint IDs for a run, ordered oldest-first.
    async fn list(&self, run_id: &str) -> Result<Vec<CheckpointId>>;

    /// Delete all checkpoints for a run.
    ///
    /// Used for garbage collection after a run reaches a terminal state.
    async fn delete_run(&self, run_id: &str) -> Result<()>;
}

// ── In-memory implementation ───────────────────────────────────────────────

/// In-memory [`CheckpointStore`] for development, integration tests, and
/// single-process embeddings.
///
/// Not suitable for production — all state is lost when the process exits.
pub mod in_memory {
    use std::sync::Arc;

    use tokio::sync::RwLock;

    use super::{Checkpoint, CheckpointId, CheckpointStore, Result, async_trait};

    /// Inner map: `run_id` → ordered list of checkpoints.
    type Inner = Arc<RwLock<std::collections::HashMap<String, Vec<Checkpoint>>>>;

    /// A simple in-memory checkpoint store.
    #[derive(Debug, Clone, Default)]
    pub struct InMemoryCheckpointStore {
        inner: Inner,
    }

    impl InMemoryCheckpointStore {
        /// Create a new, empty in-memory store.
        pub fn new() -> Self {
            Self::default()
        }
    }

    #[async_trait]
    impl CheckpointStore for InMemoryCheckpointStore {
        async fn save(&self, checkpoint: &Checkpoint) -> Result<()> {
            let mut map = self.inner.write().await;
            let entry = map
                .entry(checkpoint.run_id.clone())
                .or_insert_with(Vec::new);

            // Replace existing checkpoint with same ID or append.
            if let Some(pos) = entry.iter().position(|c| c.id == checkpoint.id) {
                entry[pos] = checkpoint.clone();
            } else {
                entry.push(checkpoint.clone());
            }
            Ok(())
        }

        async fn load_latest(&self, run_id: &str) -> Result<Option<Checkpoint>> {
            Ok(self
                .inner
                .read()
                .await
                .get(run_id)
                .and_then(|v| v.last().cloned()))
        }

        async fn load(&self, run_id: &str, id: &CheckpointId) -> Result<Option<Checkpoint>> {
            Ok(self
                .inner
                .read()
                .await
                .get(run_id)
                .and_then(|v| v.iter().find(|c| &c.id == id).cloned()))
        }

        async fn list(&self, run_id: &str) -> Result<Vec<CheckpointId>> {
            Ok(self
                .inner
                .read()
                .await
                .get(run_id)
                .map(|v| v.iter().map(|c| c.id.clone()).collect())
                .unwrap_or_default())
        }

        async fn delete_run(&self, run_id: &str) -> Result<()> {
            self.inner.write().await.remove(run_id);
            Ok(())
        }
    }
}
