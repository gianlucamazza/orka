//! Lightweight dependency injection container for Orka.
//!
//! Provides type-safe service resolution without external dependencies.
//! Inspired by modern DI patterns but kept simple for Rust's type system.
//!
//! # Example
//!
//! ```
//! use orka_core::container::ServiceContainer;
//! use std::sync::Arc;
//!
//! trait Database: Send + Sync {
//!     fn query(&self, sql: &str) -> Vec<String>;
//! }
//!
//! struct PostgresDb;
//! impl Database for PostgresDb {
//!     fn query(&self, sql: &str) -> Vec<String> {
//!         vec![format!("Result of: {sql}")]
//!     }
//! }
//!
//! # fn main() {
//! let mut container = ServiceContainer::new();
//! container.register::<Arc<dyn Database>>(Arc::new(PostgresDb));
//!
//! let db = container.get::<Arc<dyn Database>>().unwrap();
//! # }
//! ```

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::Arc;

/// Type-safe service container for dependency injection.
///
/// Stores services as `Arc<dyn Any>` and provides type-safe retrieval.
/// Thread-safe for read operations, requires mutable access for registration.
#[derive(Default)]
pub struct ServiceContainer {
    services: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
}

impl ServiceContainer {
    /// Create an empty container.
    pub fn new() -> Self {
        Self {
            services: HashMap::new(),
        }
    }

    /// Register a service in the container.
    ///
    /// # Type Parameters
    ///
    /// * `T`: The type under which the service will be registered and retrieved.
    ///        Typically `Arc<dyn Trait>` for trait objects.
    ///
    /// # Example
    ///
    /// ```
    /// use orka_core::container::ServiceContainer;
    /// use std::sync::Arc;
    ///
    /// let mut container = ServiceContainer::new();
    /// container.register::<Arc<str>>(Arc::from("config"));
    /// ```
    pub fn register<T: Send + Sync + 'static>(&mut self, service: T) {
        let type_id = TypeId::of::<T>();
        self.services.insert(type_id, Arc::new(service));
    }

    /// Retrieve a service from the container.
    ///
    /// Returns `None` if the service is not registered.
    ///
    /// # Type Parameters
    ///
    /// * `T`: The type under which the service was registered.
    pub fn get<T: Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        let type_id = TypeId::of::<T>();
        self.services
            .get(&type_id)
            .and_then(|svc| svc.clone().downcast::<T>().ok())
    }

    /// Check if a service is registered.
    pub fn contains<T: Send + Sync + 'static>(&self) -> bool {
        let type_id = TypeId::of::<T>();
        self.services.contains_key(&type_id)
    }

    /// Remove a service from the container.
    ///
    /// Returns `true` if a service was removed.
    pub fn remove<T: Send + Sync + 'static>(&mut self) -> bool {
        let type_id = TypeId::of::<T>();
        self.services.remove(&type_id).is_some()
    }

    /// Clear all services from the container.
    pub fn clear(&mut self) {
        self.services.clear();
    }

    /// Get the number of registered services.
    pub fn len(&self) -> usize {
        self.services.len()
    }

    /// Check if the container is empty.
    pub fn is_empty(&self) -> bool {
        self.services.is_empty()
    }
}

impl std::fmt::Debug for ServiceContainer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServiceContainer")
            .field("services_count", &self.services.len())
            .finish()
    }
}

/// Extension trait for ergonomic service resolution.
///
/// Provides `resolve()` method on `Arc<ServiceContainer>`.
pub trait ContainerExt {
    /// Resolve a service, panicking if not found.
    ///
    /// # Panics
    ///
    /// Panics if the service is not registered.
    fn resolve<T: Send + Sync + 'static>(&self) -> Arc<T>;
}

impl ContainerExt for ServiceContainer {
    fn resolve<T: Send + Sync + 'static>(&self) -> Arc<T> {
        self.get::<T>()
            .unwrap_or_else(|| panic!("Service {} not registered", std::any::type_name::<T>()))
    }
}

/// Factory function type for lazy initialization.
pub type ServiceFactory<T> = Box<dyn Fn(&ServiceContainer) -> T + Send + Sync>;

/// Container with lazy initialization support.
///
/// Services can be registered as factories that are called on first retrieval.
pub struct LazyContainer {
    services: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
    factories: HashMap<TypeId, Box<dyn Fn() -> Arc<dyn Any + Send + Sync> + Send + Sync>>,
}

impl LazyContainer {
    /// Create an empty lazy container.
    pub fn new() -> Self {
        Self {
            services: HashMap::new(),
            factories: HashMap::new(),
        }
    }

    /// Register a service factory for lazy initialization.
    pub fn register_lazy<T: Send + Sync + 'static>(
        &mut self,
        factory: impl Fn() -> T + Send + Sync + 'static,
    ) {
        let type_id = TypeId::of::<T>();
        self.factories.insert(type_id, Box::new(move || Arc::new(factory())));
    }

    /// Get or create a service.
    pub fn get<T: Send + Sync + 'static>(&mut self) -> Option<Arc<T>> {
        let type_id = TypeId::of::<T>();
        
        // Check if already instantiated
        if let Some(svc) = self.services.get(&type_id) {
            return svc.clone().downcast::<T>().ok();
        }
        
        // Try to instantiate from factory
        if let Some(factory) = self.factories.remove(&type_id) {
            let instance = factory();
            self.services.insert(type_id, instance.clone());
            return instance.downcast::<T>().ok();
        }
        
        None
    }
}

impl Default for LazyContainer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    trait Database: Send + Sync {
        fn query(&self, sql: &str) -> String;
    }

    struct MockDb;
    impl Database for MockDb {
        fn query(&self, sql: &str) -> String {
            format!("Mock: {sql}")
        }
    }

    #[test]
    fn container_register_and_get() {
        let mut container = ServiceContainer::new();
        let db: Arc<dyn Database> = Arc::new(MockDb);
        
        container.register::<Arc<dyn Database>>(db.clone());
        
        let retrieved = container.get::<Arc<dyn Database>>().unwrap();
        assert_eq!(retrieved.query("SELECT 1"), "Mock: SELECT 1");
    }

    #[test]
    fn container_returns_none_for_missing() {
        let container = ServiceContainer::new();
        assert!(container.get::<Arc<dyn Database>>().is_none());
    }

    #[test]
    fn container_contains_check() {
        let mut container = ServiceContainer::new();
        assert!(!container.contains::<Arc<dyn Database>>());
        
        container.register::<Arc<dyn Database>>(Arc::new(MockDb));
        assert!(container.contains::<Arc<dyn Database>>());
    }

    #[test]
    fn container_remove() {
        let mut container = ServiceContainer::new();
        container.register::<Arc<dyn Database>>(Arc::new(MockDb));
        
        assert!(container.remove::<Arc<dyn Database>>());
        assert!(!container.contains::<Arc<dyn Database>>());
        assert!(!container.remove::<Arc<dyn Database>>());
    }

    #[test]
    fn container_clear() {
        let mut container = ServiceContainer::new();
        container.register::<i32>(42);
        container.register::<String>("test".into());
        
        assert_eq!(container.len(), 2);
        container.clear();
        assert!(container.is_empty());
    }

    #[test]
    fn container_debug() {
        let mut container = ServiceContainer::new();
        container.register::<i32>(42);
        
        let debug = format!("{:?}", container);
        assert!(debug.contains("ServiceContainer"));
        assert!(debug.contains("1"));
    }

    #[test]
    fn container_resolve_success() {
        let mut container = ServiceContainer::new();
        container.register::<i32>(42);
        
        let value = container.resolve::<i32>();
        assert_eq!(*value, 42);
    }

    #[test]
    #[should_panic(expected = "not registered")]
    fn container_resolve_panic() {
        let container = ServiceContainer::new();
        let _ = container.resolve::<i32>();
    }

    #[test]
    fn lazy_container_factory() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        
        let mut container = LazyContainer::new();
        static CALL_COUNT: AtomicUsize = AtomicUsize::new(0);
        
        // Reset counter
        CALL_COUNT.store(0, Ordering::SeqCst);
        
        container.register_lazy::<i32>(|| {
            CALL_COUNT.fetch_add(1, Ordering::SeqCst);
            42
        });
        
        assert_eq!(CALL_COUNT.load(Ordering::SeqCst), 0);
        
        let val1 = container.get::<i32>().unwrap();
        assert_eq!(CALL_COUNT.load(Ordering::SeqCst), 1);
        assert_eq!(*val1, 42);
        
        // Second get should not call factory again
        let val2 = container.get::<i32>().unwrap();
        assert_eq!(CALL_COUNT.load(Ordering::SeqCst), 1);
        assert_eq!(*val2, 42);
    }
}
