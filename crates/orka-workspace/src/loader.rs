use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use tokio::sync::{RwLock, broadcast};
use tracing::{debug, warn};

use crate::state::WorkspaceState;

/// Events broadcast by [`WorkspaceLoader`] when workspace files change.
#[derive(Debug, Clone)]
pub enum WorkspaceEvent {
    /// A specific workspace file was modified and reloaded.
    FileChanged(String),
    /// All workspace files were reloaded (e.g. initial load or full reload).
    Reloaded,
}

/// Loads and hot-reloads workspace files from a directory.
pub struct WorkspaceLoader {
    root: PathBuf,
    state: Arc<RwLock<WorkspaceState>>,
    tx: broadcast::Sender<WorkspaceEvent>,
}

impl WorkspaceLoader {
    /// Create a loader pointing at the given workspace root directory.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let (tx, _) = broadcast::channel(64);
        Self {
            root: root.into(),
            state: Arc::new(RwLock::new(WorkspaceState::default())),
            tx,
        }
    }

    /// Return the workspace root directory path.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Return a shared handle to the current workspace state.
    pub fn state(&self) -> Arc<RwLock<WorkspaceState>> {
        self.state.clone()
    }

    /// Subscribe to workspace change events.
    pub fn subscribe(&self) -> broadcast::Receiver<WorkspaceEvent> {
        self.tx.subscribe()
    }

    /// Load all workspace files from disk.
    pub async fn load_all(&self) -> orka_core::Result<()> {
        self.load_file("SOUL.md").await;
        self.load_file("TOOLS.md").await;
        // broadcast::send fails only when there are no active subscribers — safe to
        // ignore.
        let _ = self.tx.send(WorkspaceEvent::Reloaded);
        Ok(())
    }

    /// Write a SOUL.md document to disk atomically, then reload in-memory state.
    pub async fn save_soul(
        &self,
        doc: &crate::parse::Document<crate::config::SoulFrontmatter>,
    ) -> orka_core::Result<()> {
        let content = crate::parse::serialize_document(doc)?;
        self.atomic_write("SOUL.md", &content).await?;
        self.load_file("SOUL.md").await;
        Ok(())
    }

    /// Write TOOLS.md content to disk atomically, then reload in-memory state.
    pub async fn save_tools(&self, body: &str) -> orka_core::Result<()> {
        self.atomic_write("TOOLS.md", body).await?;
        self.load_file("TOOLS.md").await;
        Ok(())
    }

    /// Write content to a temporary file in the workspace root, then rename
    /// atomically over the target (same filesystem, so rename is atomic).
    async fn atomic_write(&self, filename: &str, content: &str) -> orka_core::Result<()> {
        let target = self.root.join(filename);
        let tmp = self.root.join(format!(".{filename}.tmp"));
        tokio::fs::write(&tmp, content)
            .await
            .map_err(|e| orka_core::Error::Workspace(format!("failed to write {filename}: {e}")))?;
        tokio::fs::rename(&tmp, &target)
            .await
            .map_err(|e| orka_core::Error::Workspace(format!("failed to rename {filename}: {e}")))?;
        Ok(())
    }

    /// Load a single file by name, updating state. Logs warnings on errors.
    pub async fn load_file(&self, filename: &str) {
        let path = self.root.join(filename);
        let content = match tokio::fs::read_to_string(&path).await {
            Ok(c) => c,
            Err(e) => {
                debug!(file = %filename, error = %e, "workspace file not found (optional)");
                return;
            }
        };

        let mut state = self.state.write().await;
        match filename {
            "SOUL.md" => match crate::parse::parse_document(&content) {
                Ok(doc) => state.soul = Some(doc),
                Err(e) => warn!(file = %filename, error = %e, "failed to parse"),
            },
            "TOOLS.md" => {
                state.tools_body = Some(crate::parse::strip_frontmatter(&content));
            }
            other => warn!(file = %other, "unknown workspace file"),
        }
        // broadcast::send fails only when there are no active subscribers — safe to
        // ignore.
        let _ = self
            .tx
            .send(WorkspaceEvent::FileChanged(filename.to_string()));
    }
}
