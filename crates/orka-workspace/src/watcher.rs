use crate::loader::WorkspaceLoader;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::sync::Arc;
use tokio::sync::mpsc;


pub struct WorkspaceWatcher {
    _watcher: RecommendedWatcher,
    handle: tokio::task::JoinHandle<()>,
}

impl WorkspaceWatcher {
    pub fn start(loader: Arc<WorkspaceLoader>) -> orka_core::Result<Self> {
        let (tx, mut rx) = mpsc::channel::<notify::Event>(256);
        let root = loader.root().to_path_buf();

        let mut watcher = RecommendedWatcher::new(
            move |res: Result<notify::Event, notify::Error>| {
                if let Ok(event) = res {
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
                        let known = [
                            "SOUL.md",
                            "TOOLS.md",
                            "IDENTITY.md",
                            "HEARTBEAT.md",
                            "MEMORY.md",
                        ];
                        if !known.contains(&filename) {
                            continue;
                        }

                        let now = Instant::now();
                        if let Some(last) = last_seen.get(filename) {
                            if now.duration_since(*last) < debounce {
                                continue;
                            }
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

    pub async fn stop(self) {
        self.handle.abort();
    }
}
