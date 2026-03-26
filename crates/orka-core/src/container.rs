//! Lightweight dependency injection container for Orka.
//!
//! Provides type-safe service resolution without external dependencies.
//! Inspired by modern DI patterns but kept simple for Rust's type system.
//!
//! # Example
//!
//! ```
//! use std::sync::Arc;
//!
//! use orka_core::container::ServiceContainer;
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

use std::{
    any::{Any, TypeId},
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::Arc,
};

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
    /// * `T`: The type under which the service will be registered and
    ///   retrieved. Typically `Arc<dyn Trait>` for trait objects.
    ///
    /// # Example
    ///
    /// ```
    /// use std::sync::Arc;
    ///
    /// use orka_core::container::ServiceContainer;
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
        self.factories
            .insert(type_id, Box::new(move || Arc::new(factory())));
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

/// Async factory function type for async lazy initialization.
pub type AsyncServiceFactory<T> =
    Box<dyn Fn() -> Pin<Box<dyn Future<Output = T> + Send>> + Send + Sync>;

/// Erased factory type used inside [`AsyncServiceEntry`].
type ErasedAsyncFactory =
    Box<dyn Fn() -> Pin<Box<dyn Future<Output = Arc<dyn Any + Send + Sync>> + Send>> + Send + Sync>;

/// Internal representation of a pending or initialized async service.
enum AsyncServiceEntry {
    /// Service is pending initialization. Contains the factory.
    Pending(ErasedAsyncFactory),
    /// Service is currently being initialized by another task.
    /// Tasks should wait on this notify.
    Initializing(Arc<tokio::sync::Notify>),
    /// Service is initialized.
    Initialized(Arc<dyn Any + Send + Sync>),
}

/// Async container with lazy initialization support.
///
/// Services can be registered as async factories that are called on first
/// retrieval. Thread-safe and suitable for async applications.
///
/// # Concurrency
///
/// When multiple tasks request the same service concurrently:
/// - The first task triggers the factory
/// - Other tasks wait for completion
/// - All tasks receive the same `Arc<T>` instance
pub struct AsyncServiceContainer {
    services: tokio::sync::RwLock<HashMap<TypeId, AsyncServiceEntry>>,
}

impl AsyncServiceContainer {
    /// Create an empty async container.
    pub fn new() -> Self {
        Self {
            services: tokio::sync::RwLock::new(HashMap::new()),
        }
    }

    /// Register a service directly.
    pub async fn register<T: Send + Sync + 'static>(&self, service: T) {
        let type_id = TypeId::of::<T>();
        let mut services = self.services.write().await;
        services.insert(type_id, AsyncServiceEntry::Initialized(Arc::new(service)));
    }

    /// Register an async service factory for lazy initialization.
    ///
    /// # Example
    ///
    /// ```rust
    /// use std::{future::Future, pin::Pin, sync::Arc};
    ///
    /// use orka_core::container::AsyncServiceContainer;
    ///
    /// # async fn example() {
    /// let container = Arc::new(AsyncServiceContainer::new());
    ///
    /// let factory: Box<dyn Fn() -> Pin<Box<dyn Future<Output = Arc<str>> + Send>> + Send + Sync> =
    ///     Box::new(|| {
    ///         Box::pin(async {
    ///             // Async initialization
    ///             Arc::from("configured")
    ///         })
    ///     });
    /// container.register_async::<Arc<str>>(factory).await;
    /// # }
    /// ```
    pub async fn register_async<T: Send + Sync + 'static>(
        &self,
        factory: impl Fn() -> Pin<Box<dyn Future<Output = T> + Send>> + Send + Sync + 'static,
    ) {
        let type_id = TypeId::of::<T>();
        let mut services = self.services.write().await;

        let wrapped_factory = Box::new(move || {
            let fut = factory();
            Box::pin(async move {
                let result: Arc<dyn Any + Send + Sync> = Arc::new(fut.await);
                result
            }) as Pin<Box<dyn Future<Output = Arc<dyn Any + Send + Sync>> + Send>>
        });

        services.insert(type_id, AsyncServiceEntry::Pending(wrapped_factory));
    }

    /// Get or create a service asynchronously.
    ///
    /// If the service is already initialized, returns immediately.
    /// Otherwise, calls the async factory and caches the result.
    ///
    /// # Concurrency
    ///
    /// Multiple concurrent calls to `get` for the same service will:
    /// 1. Wait for the service to be initialized by the first caller
    /// 2. All receive the same `Arc<T>` instance
    pub async fn get<T: Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        let type_id = TypeId::of::<T>();

        // Fast path: check if already initialized.
        // Each match uses a short-lived borrow so we can drop(services) when needed.
        {
            let services = self.services.read().await;

            // Already initialized?
            let initialized = match services.get(&type_id) {
                Some(AsyncServiceEntry::Initialized(arc)) => Some(arc.clone()),
                _ => None,
            };
            if let Some(arc) = initialized {
                return arc.downcast::<T>().ok();
            }

            // Someone else is initializing? Subscribe to their Notify BEFORE releasing the
            // read lock so we cannot miss the notify_waiters() call.
            let wait_notify: Option<Arc<tokio::sync::Notify>> = match services.get(&type_id) {
                Some(AsyncServiceEntry::Initializing(n)) => Some(n.clone()),
                _ => None,
            };
            if let Some(n) = wait_notify {
                let notified = n.notified(); // Subscribe while still holding the read lock
                drop(services); // Release lock — safe, notified is already subscribed
                notified.await;

                // Try again - should be initialized now
                let services = self.services.read().await;
                if let Some(AsyncServiceEntry::Initialized(arc)) = services.get(&type_id) {
                    return arc.clone().downcast::<T>().ok();
                }
                return None;
            }

            // Pending or missing — fall through to slow path
        }

        // Slow path: we need to initialize
        let notify = Arc::new(tokio::sync::Notify::new());

        let factory = {
            let mut services = self.services.write().await;

            // Double-check after acquiring write lock.
            // Each check uses a short-lived borrow that ends before the next statement,
            // so we can safely drop(services) when needed.

            // Already initialized?
            let initialized = match services.get(&type_id) {
                Some(AsyncServiceEntry::Initialized(arc)) => Some(arc.clone()),
                _ => None,
            };
            if let Some(arc) = initialized {
                return arc.downcast::<T>().ok();
            }

            // Someone else is initializing? Subscribe to their Notify BEFORE releasing the
            // write lock so we cannot miss the notify_waiters() call.
            let wait_notify: Option<Arc<tokio::sync::Notify>> = match services.get(&type_id) {
                Some(AsyncServiceEntry::Initializing(n)) => Some(n.clone()),
                _ => None,
            };
            if let Some(n) = wait_notify {
                let notified = n.notified(); // Subscribe while we still hold the write lock
                drop(services); // Release lock — safe, notified is already subscribed
                notified.await;

                let services = self.services.read().await;
                return if let Some(AsyncServiceEntry::Initialized(arc)) = services.get(&type_id) {
                    arc.clone().downcast::<T>().ok()
                } else {
                    None
                };
            }

            // Must be Pending — take ownership of initialization
            if let Some(AsyncServiceEntry::Pending(factory)) = services.remove(&type_id) {
                services.insert(type_id, AsyncServiceEntry::Initializing(notify.clone()));
                Some(factory)
            } else {
                None
            }
        };

        if let Some(factory) = factory {
            // Initialize the service
            let instance = factory().await;

            // Store the initialized service
            {
                let mut services = self.services.write().await;
                services.insert(type_id, AsyncServiceEntry::Initialized(instance));
            }

            // Notify waiters
            notify.notify_waiters();

            // Return the service
            let services = self.services.read().await;
            if let Some(AsyncServiceEntry::Initialized(arc)) = services.get(&type_id) {
                return arc.clone().downcast::<T>().ok();
            }
        }

        None
    }

    /// Resolve a service, panicking if not found.
    pub async fn resolve<T: Send + Sync + 'static>(&self) -> Arc<T> {
        self.get::<T>()
            .await
            .unwrap_or_else(|| panic!("Service {} not registered", std::any::type_name::<T>()))
    }

    /// Check if a service is registered or has a pending factory.
    pub async fn contains<T: Send + Sync + 'static>(&self) -> bool {
        let type_id = TypeId::of::<T>();
        let services = self.services.read().await;
        services.contains_key(&type_id)
    }

    /// Get the number of initialized services.
    pub async fn initialized_count(&self) -> usize {
        let services = self.services.read().await;
        services
            .values()
            .filter(|e| matches!(e, AsyncServiceEntry::Initialized(_)))
            .count()
    }

    /// Get the number of pending factories.
    pub async fn pending_count(&self) -> usize {
        let services = self.services.read().await;
        services
            .values()
            .filter(|e| matches!(e, AsyncServiceEntry::Pending(_)))
            .count()
    }
}

impl Default for AsyncServiceContainer {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for AsyncServiceContainer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AsyncServiceContainer")
            .field("type", &std::any::type_name::<Self>())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    static LAZY_CALL_COUNT: AtomicUsize = AtomicUsize::new(0);
    static ASYNC_LAZY_CALL_COUNT: AtomicUsize = AtomicUsize::new(0);
    static CONCURRENT_CALL_COUNT: AtomicUsize = AtomicUsize::new(0);

    fn some<T>(value: Option<T>, label: &str) -> T {
        match value {
            Some(value) => value,
            None => panic!("expected {label} to be present"),
        }
    }

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

        let retrieved = some(container.get::<Arc<dyn Database>>(), "database service");
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

        let debug = format!("{container:?}");
        assert!(debug.contains("ServiceContainer"));
        assert!(debug.contains('1'));
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
        let mut container = LazyContainer::new();

        // Reset counter
        LAZY_CALL_COUNT.store(0, Ordering::SeqCst);

        container.register_lazy::<i32>(|| {
            LAZY_CALL_COUNT.fetch_add(1, Ordering::SeqCst);
            42
        });

        assert_eq!(LAZY_CALL_COUNT.load(Ordering::SeqCst), 0);

        let val1 = some(container.get::<i32>(), "lazy i32");
        assert_eq!(LAZY_CALL_COUNT.load(Ordering::SeqCst), 1);
        assert_eq!(*val1, 42);

        // Second get should not call factory again
        let val2 = some(container.get::<i32>(), "cached lazy i32");
        assert_eq!(LAZY_CALL_COUNT.load(Ordering::SeqCst), 1);
        assert_eq!(*val2, 42);
    }

    #[tokio::test]
    async fn async_container_register_and_get() {
        let container = AsyncServiceContainer::new();

        container
            .register::<Arc<dyn Database>>(Arc::new(MockDb))
            .await;

        let retrieved = some(
            container.get::<Arc<dyn Database>>().await,
            "database service",
        );
        assert_eq!(retrieved.query("SELECT 1"), "Mock: SELECT 1");
    }

    #[tokio::test]
    async fn async_container_lazy_factory() {
        let container = Arc::new(AsyncServiceContainer::new());

        ASYNC_LAZY_CALL_COUNT.store(0, Ordering::SeqCst);

        // Factory function with explicit type
        let factory: Box<dyn Fn() -> Pin<Box<dyn Future<Output = i32> + Send>> + Send + Sync> =
            Box::new(|| {
                Box::pin(async move {
                    ASYNC_LAZY_CALL_COUNT.fetch_add(1, Ordering::SeqCst);
                    42
                })
            });
        container.register_async::<i32>(factory).await;

        // Factory not called yet
        assert_eq!(ASYNC_LAZY_CALL_COUNT.load(Ordering::SeqCst), 0);
        assert_eq!(container.initialized_count().await, 0);
        assert_eq!(container.pending_count().await, 1);

        // First get triggers factory
        let val1 = some(container.get::<i32>().await, "async lazy i32");
        assert_eq!(ASYNC_LAZY_CALL_COUNT.load(Ordering::SeqCst), 1);
        assert_eq!(*val1, 42);
        assert_eq!(container.initialized_count().await, 1);
        assert_eq!(container.pending_count().await, 0);

        // Second get returns cached
        let val2 = some(container.get::<i32>().await, "cached async lazy i32");
        assert_eq!(ASYNC_LAZY_CALL_COUNT.load(Ordering::SeqCst), 1);
        assert_eq!(*val2, 42);
    }

    #[tokio::test]
    async fn async_container_lazy_get() {
        let container = Arc::new(AsyncServiceContainer::new());

        // Factory function with explicit type
        let factory: Box<dyn Fn() -> Pin<Box<dyn Future<Output = String> + Send>> + Send + Sync> =
            Box::new(|| {
                Box::pin(async move {
                    // Simulate async work
                    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
                    "initialized".to_string()
                })
            });
        container.register_async::<String>(factory).await;

        // First get triggers factory
        let val1 = some(container.get::<String>().await, "async lazy string");
        assert_eq!(*val1, "initialized");

        // Second get returns cached
        let val2 = some(container.get::<String>().await, "cached async lazy string");
        assert!(Arc::ptr_eq(&val1, &val2));
    }

    #[tokio::test]
    async fn async_container_concurrent_get() {
        let container = Arc::new(AsyncServiceContainer::new());

        CONCURRENT_CALL_COUNT.store(0, Ordering::SeqCst);

        // Factory that tracks how many times it's called
        #[allow(clippy::type_complexity)]
        let factory: Box<
            dyn Fn() -> Pin<Box<dyn Future<Output = Arc<String>> + Send>> + Send + Sync,
        > = Box::new(|| {
            Box::pin(async move {
                CONCURRENT_CALL_COUNT.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
                Arc::new("concurrent_test".to_string())
            })
        });
        container.register_async::<Arc<String>>(factory).await;

        // Spawn multiple concurrent gets
        let handles: Vec<_> = (0..5)
            .map(|_| {
                let container = container.clone();
                tokio::spawn(async move { container.get::<Arc<String>>().await })
            })
            .collect();

        // All should complete successfully
        let results: Vec<_> = futures_util::future::join_all(handles).await;

        // All should succeed
        for result in &results {
            let Ok(Some(value)) = result.as_ref() else {
                panic!("expected task success with initialized service");
            };
            assert_eq!(value.as_str(), "concurrent_test");
        }

        // Factory should only be called once
        assert_eq!(
            CONCURRENT_CALL_COUNT.load(Ordering::SeqCst),
            1,
            "Factory should be called exactly once"
        );

        // All should point to the same Arc
        let Ok(Some(first)) = results[0].as_ref() else {
            panic!("expected first concurrent result");
        };
        for result in &results {
            let Ok(Some(value)) = result.as_ref() else {
                panic!("expected concurrent result");
            };
            assert!(Arc::ptr_eq(first, value), "All should share the same Arc");
        }
    }

    #[tokio::test]
    async fn async_container_resolve() {
        let container = AsyncServiceContainer::new();
        container.register::<i32>(42).await;

        let value = container.resolve::<i32>().await;
        assert_eq!(*value, 42);
    }

    #[tokio::test]
    #[should_panic(expected = "not registered")]
    async fn async_container_resolve_panic() {
        let container = AsyncServiceContainer::new();
        let _ = container.resolve::<i32>().await;
    }
}
