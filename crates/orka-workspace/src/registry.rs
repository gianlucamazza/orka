use std::{collections::HashMap, path::PathBuf, sync::Arc};

use tokio::sync::RwLock;

use crate::{
    loader::WorkspaceLoader, parse::Document, state::WorkspaceState, watcher::WorkspaceWatcher,
};

// ---------------------------------------------------------------------------
// Internal storage
// ---------------------------------------------------------------------------

struct RegistryInner {
    loaders: HashMap<String, Arc<WorkspaceLoader>>,
    watchers: HashMap<String, WorkspaceWatcher>,
}

impl RegistryInner {
    fn new() -> Self {
        Self {
            loaders: HashMap::new(),
            watchers: HashMap::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Public registry
// ---------------------------------------------------------------------------

/// Registry of named workspaces, used for multi-workspace support.
///
/// Each workspace has a name and an associated [`WorkspaceLoader`].
/// One workspace is designated as the default.
///
/// Interior mutability via [`tokio::sync::RwLock`] allows new workspaces to be
/// registered at runtime through a shared `Arc<WorkspaceRegistry>`.
pub struct WorkspaceRegistry {
    inner: RwLock<RegistryInner>,
    default_name: String,
    /// Base directory for dynamically created workspaces.
    /// `None` in single-workspace mode — create/delete operations are disabled.
    base_dir: Option<PathBuf>,
}

impl WorkspaceRegistry {
    /// Create an empty registry with the given default workspace name.
    ///
    /// `base_dir` is the directory where new workspaces are created. Pass
    /// `None` to disable runtime create/delete (single-workspace mode).
    pub fn new(default_name: String, base_dir: Option<PathBuf>) -> Self {
        Self {
            inner: RwLock::new(RegistryInner::new()),
            default_name,
            base_dir,
        }
    }

    // ── Reads ────────────────────────────────────────────────────────────────

    /// Return the name of the default workspace.
    pub fn default_name(&self) -> &str {
        &self.default_name
    }

    /// Look up a workspace loader by name. Returns an owned `Arc` clone.
    pub async fn get(&self, name: &str) -> Option<Arc<WorkspaceLoader>> {
        self.inner.read().await.loaders.get(name).cloned()
    }

    /// Return the loader for the default workspace, or `None` if not registered
    /// yet.
    pub async fn default_loader(&self) -> Option<Arc<WorkspaceLoader>> {
        self.get(&self.default_name).await
    }

    /// List all registered workspace names, sorted alphabetically.
    pub async fn list_names(&self) -> Vec<String> {
        let inner = self.inner.read().await;
        let mut names: Vec<String> = inner.loaders.keys().cloned().collect();
        names.sort_unstable();
        names
    }

    /// Return the state handle for the named workspace, if registered.
    pub async fn state(&self, name: &str) -> Option<Arc<RwLock<WorkspaceState>>> {
        self.get(name).await.map(|l| l.state())
    }

    /// Return the state handle for the default workspace.
    pub async fn default_state(&self) -> Option<Arc<RwLock<WorkspaceState>>> {
        self.default_loader().await.map(|l| l.state())
    }

    /// Return `true` if a workspace with the given name is registered.
    pub async fn contains(&self, name: &str) -> bool {
        self.inner.read().await.loaders.contains_key(name)
    }

    // ── Writes ───────────────────────────────────────────────────────────────

    /// Register a loader (without a watcher) under the given name.
    pub async fn register(&self, name: String, loader: Arc<WorkspaceLoader>) {
        self.inner.write().await.loaders.insert(name, loader);
    }

    /// Register a loader together with its filesystem watcher.
    pub async fn register_with_watcher(
        &self,
        name: String,
        loader: Arc<WorkspaceLoader>,
        watcher: Option<WorkspaceWatcher>,
    ) {
        let mut inner = self.inner.write().await;
        inner.loaders.insert(name.clone(), loader);
        if let Some(w) = watcher {
            inner.watchers.insert(name, w);
        }
    }

    /// Remove a workspace from the registry, returning the loader and watcher.
    pub async fn unregister(
        &self,
        name: &str,
    ) -> Option<(Arc<WorkspaceLoader>, Option<WorkspaceWatcher>)> {
        let mut inner = self.inner.write().await;
        let loader = inner.loaders.remove(name)?;
        let watcher = inner.watchers.remove(name);
        Some((loader, watcher))
    }

    // ── Lifecycle operations ─────────────────────────────────────────────────

    /// Create a new workspace directory, write an initial `SOUL.md` (and
    /// optionally `TOOLS.md`), load it, start a watcher, and register it.
    ///
    /// Returns an error if:
    /// - The registry is in single-workspace mode (`base_dir` is `None`).
    /// - The name is invalid (see [`validate_workspace_name`]).
    /// - A workspace with the same name already exists.
    /// - Filesystem operations fail.
    pub async fn create_workspace(
        &self,
        name: &str,
        soul: Option<Document<crate::config::SoulFrontmatter>>,
        tools_body: Option<&str>,
    ) -> orka_core::Result<Arc<WorkspaceLoader>> {
        let base_dir = self.base_dir.as_deref().ok_or_else(|| {
            orka_core::Error::Workspace(
                "workspace creation is not supported in single-workspace mode".into(),
            )
        })?;

        validate_workspace_name(name)?;

        if self.contains(name).await {
            return Err(orka_core::Error::Workspace(format!(
                "workspace '{name}' already exists"
            )));
        }

        let dir = base_dir.join(name);
        tokio::fs::create_dir(&dir).await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                orka_core::Error::Workspace(format!(
                    "workspace directory '{name}' already exists on disk"
                ))
            } else {
                orka_core::Error::Workspace(format!("failed to create workspace directory: {e}"))
            }
        })?;

        // On failure after dir creation, clean up.
        let loader = Arc::new(WorkspaceLoader::new(&dir));
        let result = self.init_workspace_files(&loader, soul, tools_body).await;
        if let Err(e) = result {
            let _ = tokio::fs::remove_dir_all(&dir).await;
            return Err(e);
        }

        loader.load_all().await?;

        let watcher = match WorkspaceWatcher::start(loader.clone()) {
            Ok(w) => Some(w),
            Err(e) => {
                tracing::warn!(workspace = %name, %e, "failed to start watcher for new workspace");
                None
            }
        };
        self.register_with_watcher(name.to_string(), loader.clone(), watcher)
            .await;

        Ok(loader)
    }

    /// Write the initial SOUL.md (and optionally TOOLS.md) files into the
    /// loader's directory.
    async fn init_workspace_files(
        &self,
        loader: &WorkspaceLoader,
        soul: Option<Document<crate::config::SoulFrontmatter>>,
        tools_body: Option<&str>,
    ) -> orka_core::Result<()> {
        let doc = soul.unwrap_or_else(|| Document {
            frontmatter: crate::config::SoulFrontmatter::default(),
            body: String::new(),
        });
        loader.save_soul(&doc).await?;
        if let Some(body) = tools_body {
            loader.save_tools(body).await?;
        }
        Ok(())
    }

    /// Remove a workspace from the registry, stop its watcher, and archive its
    /// directory on disk by renaming it to `<name>.archived-<unix_timestamp>`.
    ///
    /// Returns an error if:
    /// - The registry is in single-workspace mode.
    /// - The workspace is the default (cannot delete the default workspace).
    /// - The workspace is not registered.
    pub async fn remove_workspace(&self, name: &str) -> orka_core::Result<()> {
        self.base_dir.as_ref().ok_or_else(|| {
            orka_core::Error::Workspace(
                "workspace deletion is not supported in single-workspace mode".into(),
            )
        })?;

        if name == self.default_name {
            return Err(orka_core::Error::Workspace(format!(
                "cannot delete the default workspace '{name}'"
            )));
        }

        let (loader, watcher) = self
            .unregister(name)
            .await
            .ok_or_else(|| orka_core::Error::Workspace(format!("workspace '{name}' not found")))?;

        if let Some(w) = watcher {
            w.stop();
        }

        // Archive the directory: rename to <dir>.archived-<timestamp>
        let dir = loader.root().to_path_buf();
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let archive = dir.with_file_name(format!(
            "{}.archived-{ts}",
            dir.file_name().and_then(|n| n.to_str()).unwrap_or(name)
        ));
        tokio::fs::rename(&dir, &archive).await.map_err(|e| {
            orka_core::Error::Workspace(format!("failed to archive workspace directory: {e}"))
        })?;

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Name validation
// ---------------------------------------------------------------------------

/// Validate a workspace name: lowercase alphanumeric + hyphens, 1-64 chars,
/// hyphens not at start/end, not a reserved name.
fn validate_workspace_name(name: &str) -> orka_core::Result<()> {
    if name.is_empty() || name.len() > 64 {
        return Err(orka_core::Error::Workspace(
            "workspace name must be 1–64 characters".into(),
        ));
    }
    if name == "default" {
        return Err(orka_core::Error::Workspace(
            "'default' is a reserved workspace name".into(),
        ));
    }
    let bytes = name.as_bytes();
    let last = bytes.len() - 1;
    let valid = bytes.iter().enumerate().all(|(i, &b)| {
        let alnum = b.is_ascii_lowercase() || b.is_ascii_digit();
        let hyphen = b == b'-' && i != 0 && i != last;
        alnum || hyphen
    });
    if !valid {
        return Err(orka_core::Error::Workspace(
            "workspace name must contain only lowercase letters, digits, and hyphens \
             (hyphens not at start or end)"
                .into(),
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::field_reassign_with_default,
    clippy::default_trait_access,
    clippy::needless_pass_by_value,
    clippy::stable_sort_primitive
)]
mod tests {
    use super::*;

    fn make_loader(dir: &str) -> Arc<WorkspaceLoader> {
        Arc::new(WorkspaceLoader::new(dir))
    }

    fn make_registry(default_name: &str) -> WorkspaceRegistry {
        WorkspaceRegistry::new(default_name.into(), None)
    }

    #[tokio::test]
    async fn register_and_get() {
        let registry = make_registry("default");
        registry.register("default".into(), make_loader(".")).await;
        registry
            .register("support".into(), make_loader("workspaces/support"))
            .await;

        assert!(registry.get("default").await.is_some());
        assert!(registry.get("support").await.is_some());
        assert!(registry.get("unknown").await.is_none());
    }

    #[tokio::test]
    async fn default_loader_works() {
        let registry = make_registry("main");
        registry.register("main".into(), make_loader(".")).await;
        let loader = registry
            .default_loader()
            .await
            .expect("main was registered");
        assert_eq!(loader.root().to_str().unwrap(), ".");
    }

    #[tokio::test]
    async fn default_loader_returns_none_when_not_registered() {
        let registry = make_registry("missing");
        assert!(registry.default_loader().await.is_none());
        assert!(registry.default_state().await.is_none());
    }

    #[tokio::test]
    async fn list_names_sorted() {
        let registry = make_registry("a");
        registry.register("c".into(), make_loader(".")).await;
        registry.register("a".into(), make_loader(".")).await;
        registry.register("b".into(), make_loader(".")).await;
        assert_eq!(registry.list_names().await, vec!["a", "b", "c"]);
    }

    #[tokio::test]
    async fn state_returns_some_for_known() {
        let registry = make_registry("default");
        registry.register("default".into(), make_loader(".")).await;
        assert!(registry.state("default").await.is_some());
        assert!(registry.state("unknown").await.is_none());
    }

    #[tokio::test]
    async fn default_name_getter() {
        let registry = make_registry("myws");
        assert_eq!(registry.default_name(), "myws");
    }

    #[tokio::test]
    async fn contains_returns_correct_value() {
        let registry = make_registry("default");
        assert!(!registry.contains("default").await);
        registry.register("default".into(), make_loader(".")).await;
        assert!(registry.contains("default").await);
    }

    #[tokio::test]
    async fn unregister_removes_loader() {
        let registry = make_registry("default");
        registry.register("default".into(), make_loader(".")).await;
        assert!(registry.get("default").await.is_some());
        let result = registry.unregister("default").await;
        assert!(result.is_some());
        assert!(registry.get("default").await.is_none());
    }

    // ── Name validation ──────────────────────────────────────────────────────

    #[test]
    fn valid_names_accepted() {
        for name in &["abc", "a1b2", "my-workspace", "ws-v2", "x"] {
            validate_workspace_name(name).unwrap_or_else(|_| panic!("{name} should be valid"));
        }
    }

    #[test]
    fn invalid_names_rejected() {
        for name in &[
            "",
            "default",
            "My-Workspace",
            "-starts-with-hyphen",
            "ends-with-hyphen-",
            "has space",
            "has_underscore",
            &"a".repeat(65),
        ] {
            assert!(
                validate_workspace_name(name).is_err(),
                "{name} should be rejected"
            );
        }
    }
}
