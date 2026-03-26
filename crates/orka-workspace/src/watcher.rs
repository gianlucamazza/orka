use std::sync::Arc;

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;

use crate::loader::WorkspaceLoader;

/// Filesystem watcher that triggers workspace reloads on file changes.
pub struct WorkspaceWatcher {
    _watcher: RecommendedWatcher,
    handle: tokio::task::JoinHandle<()>,
}

impl WorkspaceWatcher {
    /// Start watching the workspace directory associated with `loader`.
    ///
    /// Workspace files (`SOUL.md`, `TOOLS.md`) are reloaded with a 500 ms
    /// debounce.
    pub fn start(loader: Arc<WorkspaceLoader>) -> orka_core::Result<Self> {
        let (tx, mut rx) = mpsc::channel::<notify::Event>(256);
        let root = loader.root().to_path_buf();

        let mut watcher = RecommendedWatcher::new(
            move |res: Result<notify::Event, notify::Error>| {
                if let Ok(event) = res {
                    // Receiver dropped means the watcher task is shutting down — ignore.
                    let _ = tx.blocking_send(event);
                }
            },
            notify::Config::default(),
        )
        .map_err(|e| orka_core::Error::Workspace(e.to_string()))?;

        watcher
            .watch(&root, RecursiveMode::NonRecursive)
            .map_err(|e| orka_core::Error::Workspace(e.to_string()))?;

        let handle = tokio::spawn(async move {
            use std::collections::HashMap;

            use tokio::time::{Duration, Instant};

            let mut last_seen: HashMap<String, Instant> = HashMap::new();
            let debounce = Duration::from_millis(500);

            while let Some(event) = rx.recv().await {
                if !matches!(event.kind, EventKind::Modify(_) | EventKind::Create(_)) {
                    continue;
                }
                for path in &event.paths {
                    if let Some(filename) = path.file_name().and_then(|f| f.to_str()) {
                        let known = ["SOUL.md", "TOOLS.md"];
                        if !known.contains(&filename) {
                            continue;
                        }

                        let now = Instant::now();
                        if let Some(last) = last_seen.get(filename)
                            && now.duration_since(*last) < debounce
                        {
                            continue;
                        }
                        last_seen.insert(filename.to_string(), now);
                        loader.load_file(filename).await;
                    }
                }
            }
        });

        Ok(Self {
            _watcher: watcher,
            handle,
        })
    }

    /// Stop the file watcher and abort the background task.
    pub fn stop(self) {
        self.handle.abort();
    }
}
