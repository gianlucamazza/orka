use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

use sha2::{Digest, Sha256};

/// Simple in-memory cache with TTL.
///
/// Designed for development and single-instance deployments.
/// For production multi-instance setups, replace with a Redis-backed impl.
pub struct WebCache {
    entries: Mutex<HashMap<String, CacheEntry>>,
    ttl: Duration,
}

struct CacheEntry {
    value: String,
    inserted_at: Instant,
}

impl WebCache {
    pub fn new(ttl_secs: u64) -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            ttl: Duration::from_secs(ttl_secs),
        }
    }

    /// Get a cached value by key prefix and raw key.
    pub fn get(&self, prefix: &str, raw_key: &str) -> Option<String> {
        let key = cache_key(prefix, raw_key);
        let entries = self.entries.lock().ok()?;
        let entry = entries.get(&key)?;
        if entry.inserted_at.elapsed() > self.ttl {
            return None;
        }
        Some(entry.value.clone())
    }

    /// Store a value in the cache.
    pub fn set(&self, prefix: &str, raw_key: &str, value: String) {
        let key = cache_key(prefix, raw_key);
        if let Ok(mut entries) = self.entries.lock() {
            // Evict expired entries periodically (every 100 inserts)
            if entries.len() % 100 == 0 {
                let ttl = self.ttl;
                entries.retain(|_, e| e.inserted_at.elapsed() <= ttl);
            }
            entries.insert(
                key,
                CacheEntry {
                    value,
                    inserted_at: Instant::now(),
                },
            );
        }
    }
}

fn cache_key(prefix: &str, raw_key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(raw_key.as_bytes());
    let hash = hex::encode(hasher.finalize());
    format!("orka:web:{prefix}:{hash}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_set_and_get() {
        let cache = WebCache::new(3600);
        cache.set("search", "rust lang", "result1".into());
        assert_eq!(cache.get("search", "rust lang"), Some("result1".into()));
    }

    #[test]
    fn cache_miss() {
        let cache = WebCache::new(3600);
        assert_eq!(cache.get("search", "missing"), None);
    }

    #[test]
    fn cache_expired() {
        let cache = WebCache::new(0); // 0 second TTL
        cache.set("search", "key", "value".into());
        // With 0 TTL, the entry is immediately expired
        std::thread::sleep(std::time::Duration::from_millis(1));
        assert_eq!(cache.get("search", "key"), None);
    }
}
