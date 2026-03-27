use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;
use tracing::debug;

use super::{engine::TemplateError, registry::TemplateRegistry};

/// Events emitted by the template loader.
#[derive(Debug, Clone)]
pub enum TemplateLoaderEvent {
    /// A template file was modified and reloaded.
    TemplateReloaded(String),
    /// All templates were reloaded.
    AllReloaded,
    /// An error occurred during loading.
    Error(String),
}

/// File system loader for templates with optional hot-reload.
///
/// Watches the template directory for changes and automatically
/// reloads modified templates.
///
/// # Example
///
/// ```rust
/// use std::path::PathBuf;
///
/// use orka_prompts::template::{TemplateLoader, TemplateRegistry};
///
/// async fn example() {
///     let registry = TemplateRegistry::new();
///     let mut loader = TemplateLoader::new(registry, PathBuf::from("./templates"));
///
///     // Initial load
///     loader.load_all().await.unwrap();
///
///     // Start watching for changes
///     let mut events = loader.watch().unwrap();
///
///     // Process events
///     while let Some(event) = events.recv().await {
///         println!("Template event: {:?}", event);
///     }
/// }
/// ```
pub struct TemplateLoader {
    registry: TemplateRegistry,
    templates_dir: PathBuf,
    watcher: Option<Arc<std::sync::Mutex<RecommendedWatcher>>>,
}

impl TemplateLoader {
    /// Create a new template loader.
    ///
    /// # Arguments
    ///
    /// * `registry` - Template registry to populate
    /// * `templates_dir` - Directory containing template files
    pub fn new(registry: TemplateRegistry, templates_dir: PathBuf) -> Self {
        Self {
            registry,
            templates_dir,
            watcher: None,
        }
    }

    /// Load all templates from the configured directory.
    pub async fn load_all(&self) -> Result<usize, super::TemplateError> {
        if !self.templates_dir.exists() {
            debug!(directory = %self.templates_dir.display(), "templates directory does not exist");
            return Ok(0);
        }

        self.registry.load_from_dir(&self.templates_dir).await
    }

    /// Start watching the templates directory for changes.
    ///
    /// Returns a receiver for template loader events.
    pub fn watch(&mut self) -> Result<mpsc::Receiver<TemplateLoaderEvent>, TemplateError> {
        let (tx, rx) = mpsc::channel(100);
        let registry = self.registry.clone();
        let templates_dir = self.templates_dir.clone();

        let watcher_tx = tx;
        let mut watcher =
            notify::recommended_watcher(move |res: Result<Event, notify::Error>| match res {
                Ok(event) => {
                    if event.kind.is_modify() || event.kind.is_create() {
                        for path in &event.paths {
                            if path.extension().and_then(|e| e.to_str()) != Some("hbs") {
                                continue;
                            }

                            let name = match path.strip_prefix(&templates_dir) {
                                Ok(p) => p
                                    .to_str()
                                    .map(|s| s.trim_end_matches(".hbs").replace('\\', "/")),
                                Err(_) => continue,
                            };

                            if let Some(name) = name {
                                let registry = registry.clone();
                                let path = path.clone();
                                let tx = watcher_tx.clone();

                                tokio::spawn(async move {
                                    if let Err(e) = registry.register_file(&name, &path).await {
                                        let _ = tx
                                            .send(TemplateLoaderEvent::Error(format!(
                                                "failed to reload {name}: {e}"
                                            )))
                                            .await;
                                    } else {
                                        let _ = tx
                                            .send(TemplateLoaderEvent::TemplateReloaded(name))
                                            .await;
                                    }
                                });
                            }
                        }
                    }
                }
                Err(e) => {
                    let _ = watcher_tx
                        .try_send(TemplateLoaderEvent::Error(format!("watch error: {e}")));
                }
            })?;

        watcher.watch(&self.templates_dir, RecursiveMode::Recursive)?;

        self.watcher = Some(Arc::new(std::sync::Mutex::new(watcher)));

        Ok(rx)
    }

    /// Get the templates directory path.
    pub fn templates_dir(&self) -> &Path {
        &self.templates_dir
    }

    /// Get a reference to the underlying registry.
    pub fn registry(&self) -> &TemplateRegistry {
        &self.registry
    }

    /// Load built-in templates into the registry.
    pub async fn load_builtins(&self) -> Result<(), super::TemplateError> {
        // Load reflection template
        self.registry
            .register_inline(
                "system/reflection",
                include_str!("../../templates/system/reflection.hbs"),
            )
            .await?;

        // Load distillation template
        self.registry
            .register_inline(
                "system/distillation",
                include_str!("../../templates/system/distillation.hbs"),
            )
            .await?;

        // Load section templates
        self.registry
            .register_inline(
                "sections/principles",
                include_str!("../../templates/sections/principles.hbs"),
            )
            .await?;

        self.registry
            .register_inline(
                "sections/workspace",
                include_str!("../../templates/sections/workspace.hbs"),
            )
            .await?;

        self.registry
            .register_inline(
                "sections/persona",
                include_str!("../../templates/sections/persona.hbs"),
            )
            .await?;

        self.registry
            .register_inline(
                "sections/tools",
                include_str!("../../templates/sections/tools.hbs"),
            )
            .await?;

        self.registry
            .register_inline(
                "sections/datetime",
                include_str!("../../templates/sections/datetime.hbs"),
            )
            .await?;

        self.registry
            .register_inline(
                "sections/summary",
                include_str!("../../templates/sections/summary.hbs"),
            )
            .await?;

        self.registry
            .register_inline(
                "selection/soft_skill",
                include_str!("../../templates/selection/soft_skill.hbs"),
            )
            .await?;

        debug!("loaded built-in templates");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[tokio::test]
    async fn test_load_all_empty_dir() {
        let temp_dir = TempDir::new().unwrap();
        let registry = TemplateRegistry::new();
        let loader = TemplateLoader::new(registry, temp_dir.path().to_path_buf());

        let count = loader.load_all().await.unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_load_all_with_files() {
        let temp_dir = TempDir::new().unwrap();
        std::fs::write(temp_dir.path().join("test.hbs"), "Hello, {{name}}!").unwrap();

        let registry = TemplateRegistry::new();
        let loader = TemplateLoader::new(registry, temp_dir.path().to_path_buf());

        let count = loader.load_all().await.unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn test_load_builtins() {
        let registry = TemplateRegistry::new();
        let loader = TemplateLoader::new(registry, PathBuf::from("/tmp"));

        loader.load_builtins().await.unwrap();

        assert!(loader.registry().has_template("system/reflection").await);
        assert!(loader.registry().has_template("sections/principles").await);
    }
}
