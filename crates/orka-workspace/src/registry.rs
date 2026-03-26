use std::{collections::HashMap, sync::Arc};

use tokio::sync::RwLock;

use crate::{loader::WorkspaceLoader, state::WorkspaceState};

/// Registry of named workspaces, used for multi-workspace support.
///
/// Each workspace has a name and an associated [`WorkspaceLoader`].
/// One workspace is designated as the default.
pub struct WorkspaceRegistry {
    loaders: HashMap<String, Arc<WorkspaceLoader>>,
    default_name: String,
}

impl WorkspaceRegistry {
    /// Create an empty registry with the given default workspace name.
    pub fn new(default_name: String) -> Self {
        Self {
            loaders: HashMap::new(),
            default_name,
        }
    }

    /// Add a workspace loader under the given name.
    pub fn register(&mut self, name: String, loader: Arc<WorkspaceLoader>) {
        self.loaders.insert(name, loader);
    }

    /// Look up a workspace loader by name.
    pub fn get(&self, name: &str) -> Option<&Arc<WorkspaceLoader>> {
        self.loaders.get(name)
    }

    /// Return the name of the default workspace.
    pub fn default_name(&self) -> &str {
        &self.default_name
    }

    /// Return the loader for the default workspace. Panics if not registered.
    #[allow(clippy::expect_used)]
    pub fn default_loader(&self) -> &Arc<WorkspaceLoader> {
        self.loaders
            .get(&self.default_name)
            .expect("default workspace must be registered")
    }

    /// List all registered workspace names, sorted alphabetically.
    pub fn list_names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self
            .loaders
            .keys()
            .map(std::string::String::as_str)
            .collect();
        names.sort_unstable();
        names
    }

    /// Return the state handle for the named workspace, if registered.
    pub fn state(&self, name: &str) -> Option<Arc<RwLock<WorkspaceState>>> {
        self.loaders.get(name).map(|l| l.state())
    }

    /// Return the state handle for the default workspace.
    pub fn default_state(&self) -> Arc<RwLock<WorkspaceState>> {
        self.default_loader().state()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_loader(dir: &str) -> Arc<WorkspaceLoader> {
        Arc::new(WorkspaceLoader::new(dir))
    }

    #[test]
    fn register_and_get() {
        let mut registry = WorkspaceRegistry::new("default".into());
        registry.register("default".into(), make_loader("."));
        registry.register("support".into(), make_loader("workspaces/support"));

        assert!(registry.get("default").is_some());
        assert!(registry.get("support").is_some());
        assert!(registry.get("unknown").is_none());
    }

    #[test]
    fn default_loader_works() {
        let mut registry = WorkspaceRegistry::new("main".into());
        registry.register("main".into(), make_loader("."));
        let loader = registry.default_loader();
        assert_eq!(loader.root().to_str().unwrap(), ".");
    }

    #[test]
    fn list_names_sorted() {
        let mut registry = WorkspaceRegistry::new("a".into());
        registry.register("c".into(), make_loader("."));
        registry.register("a".into(), make_loader("."));
        registry.register("b".into(), make_loader("."));
        assert_eq!(registry.list_names(), vec!["a", "b", "c"]);
    }

    #[test]
    fn state_returns_some_for_known() {
        let mut registry = WorkspaceRegistry::new("default".into());
        registry.register("default".into(), make_loader("."));
        assert!(registry.state("default").is_some());
        assert!(registry.state("unknown").is_none());
    }

    #[test]
    fn default_name_getter() {
        let registry = WorkspaceRegistry::new("myws".into());
        assert_eq!(registry.default_name(), "myws");
    }
}
