use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use orka_core::Result;
use tokio::sync::Mutex;

use super::VectorStore;
use crate::types::SearchResult;

/// Stored vector with its payload.
struct VectorEntry {
    id: String,
    vector: Vec<f32>,
    payload: HashMap<String, String>,
}

/// In-memory [`VectorStore`] for use in tests (no Qdrant required).
pub struct InMemoryVectorStore {
    collections: Arc<Mutex<HashMap<String, Vec<VectorEntry>>>>,
}

impl InMemoryVectorStore {
    /// Create an empty vector store.
    pub fn new() -> Self {
        Self {
            collections: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl Default for InMemoryVectorStore {
    fn default() -> Self {
        Self::new()
    }
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a * norm_b)
}

#[async_trait]
impl VectorStore for InMemoryVectorStore {
    async fn ensure_collection(&self, name: &str, _dimensions: usize) -> Result<()> {
        let mut collections = self.collections.lock().await;
        collections.entry(name.to_string()).or_default();
        Ok(())
    }

    async fn upsert(
        &self,
        collection: &str,
        ids: &[String],
        vectors: &[Vec<f32>],
        payloads: &[HashMap<String, String>],
    ) -> Result<()> {
        let mut collections = self.collections.lock().await;
        let entries = collections.entry(collection.to_string()).or_default();

        for (i, id) in ids.iter().enumerate() {
            // Remove existing entry with same id
            entries.retain(|e| e.id != *id);
            entries.push(VectorEntry {
                id: id.clone(),
                vector: vectors.get(i).cloned().unwrap_or_default(),
                payload: payloads.get(i).cloned().unwrap_or_default(),
            });
        }
        Ok(())
    }

    async fn search(
        &self,
        collection: &str,
        vector: &[f32],
        limit: usize,
        score_threshold: Option<f32>,
        filter: Option<HashMap<String, String>>,
    ) -> Result<Vec<SearchResult>> {
        let collections = self.collections.lock().await;
        let entries = match collections.get(collection) {
            Some(e) => e,
            None => return Ok(Vec::new()),
        };

        let mut scored: Vec<(f32, &VectorEntry)> = entries
            .iter()
            .filter(|entry| {
                if let Some(ref f) = filter {
                    f.iter().all(|(k, v)| entry.payload.get(k) == Some(v))
                } else {
                    true
                }
            })
            .map(|entry| (cosine_similarity(vector, &entry.vector), entry))
            .filter(|(score, _)| score_threshold.is_none_or(|t| *score >= t))
            .collect();

        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(limit);

        Ok(scored
            .into_iter()
            .map(|(score, entry)| SearchResult {
                content: entry.payload.get("content").cloned().unwrap_or_default(),
                score,
                document_id: entry.payload.get("document_id").cloned(),
                metadata: entry.payload.clone(),
            })
            .collect())
    }

    async fn list_documents(
        &self,
        collection: &str,
        limit: usize,
        filter: Option<HashMap<String, String>>,
    ) -> Result<Vec<HashMap<String, String>>> {
        let collections = self.collections.lock().await;
        let entries = match collections.get(collection) {
            Some(e) => e,
            None => return Ok(Vec::new()),
        };

        let mut seen = std::collections::HashSet::new();
        let mut docs = Vec::new();

        for entry in entries {
            if let Some(ref f) = filter
                && !f.iter().all(|(k, v)| entry.payload.get(k) == Some(v))
            {
                continue;
            }
            let doc_id = entry
                .payload
                .get("document_id")
                .cloned()
                .unwrap_or_else(|| entry.id.clone());
            if seen.insert(doc_id) {
                docs.push(entry.payload.clone());
                if docs.len() >= limit {
                    break;
                }
            }
        }
        Ok(docs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn ensure_collection_is_idempotent() {
        let store = InMemoryVectorStore::new();
        store.ensure_collection("test", 3).await.unwrap();
        store.ensure_collection("test", 3).await.unwrap();
    }

    #[tokio::test]
    async fn upsert_and_search() {
        let store = InMemoryVectorStore::new();
        store.ensure_collection("test", 3).await.unwrap();

        let ids = vec!["v1".into(), "v2".into()];
        let vectors = vec![vec![1.0, 0.0, 0.0], vec![0.0, 1.0, 0.0]];
        let payloads = vec![
            HashMap::from([("content".into(), "first".into())]),
            HashMap::from([("content".into(), "second".into())]),
        ];

        store
            .upsert("test", &ids, &vectors, &payloads)
            .await
            .unwrap();

        // Search for vector similar to v1
        let results = store
            .search("test", &[1.0, 0.0, 0.0], 10, None, None)
            .await
            .unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].content, "first");
        assert!((results[0].score - 1.0).abs() < 0.001);
    }

    #[tokio::test]
    async fn search_with_threshold() {
        let store = InMemoryVectorStore::new();
        store.ensure_collection("test", 3).await.unwrap();

        let ids = vec!["v1".into(), "v2".into()];
        let vectors = vec![vec![1.0, 0.0, 0.0], vec![0.0, 1.0, 0.0]];
        let payloads = vec![HashMap::new(), HashMap::new()];

        store
            .upsert("test", &ids, &vectors, &payloads)
            .await
            .unwrap();

        let results = store
            .search("test", &[1.0, 0.0, 0.0], 10, Some(0.9), None)
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
    }

    #[tokio::test]
    async fn search_with_filter() {
        let store = InMemoryVectorStore::new();
        store.ensure_collection("test", 3).await.unwrap();

        let ids = vec!["v1".into(), "v2".into()];
        let vectors = vec![vec![1.0, 0.0, 0.0], vec![1.0, 0.1, 0.0]];
        let payloads = vec![
            HashMap::from([("type".into(), "a".into())]),
            HashMap::from([("type".into(), "b".into())]),
        ];

        store
            .upsert("test", &ids, &vectors, &payloads)
            .await
            .unwrap();

        let filter = HashMap::from([("type".into(), "b".into())]);
        let results = store
            .search("test", &[1.0, 0.0, 0.0], 10, None, Some(filter))
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
    }

    #[tokio::test]
    async fn upsert_replaces_existing() {
        let store = InMemoryVectorStore::new();
        store.ensure_collection("test", 3).await.unwrap();

        let ids = vec!["v1".into()];
        let vectors = vec![vec![1.0, 0.0, 0.0]];
        let payloads = vec![HashMap::from([("content".into(), "old".into())])];
        store
            .upsert("test", &ids, &vectors, &payloads)
            .await
            .unwrap();

        let payloads = vec![HashMap::from([("content".into(), "new".into())])];
        store
            .upsert("test", &ids, &vectors, &payloads)
            .await
            .unwrap();

        let results = store
            .search("test", &[1.0, 0.0, 0.0], 10, None, None)
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].content, "new");
    }

    #[tokio::test]
    async fn list_documents_deduplicates() {
        let store = InMemoryVectorStore::new();
        store.ensure_collection("test", 3).await.unwrap();

        let ids = vec!["c1".into(), "c2".into()];
        let vectors = vec![vec![1.0, 0.0, 0.0], vec![0.0, 1.0, 0.0]];
        let payloads = vec![
            HashMap::from([("document_id".into(), "doc1".into())]),
            HashMap::from([("document_id".into(), "doc1".into())]),
        ];

        store
            .upsert("test", &ids, &vectors, &payloads)
            .await
            .unwrap();

        let docs = store.list_documents("test", 10, None).await.unwrap();
        assert_eq!(docs.len(), 1);
    }

    #[tokio::test]
    async fn search_empty_collection() {
        let store = InMemoryVectorStore::new();
        let results = store
            .search("nonexistent", &[1.0], 10, None, None)
            .await
            .unwrap();
        assert!(results.is_empty());
    }
}
