use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use orka_core::Result;
use qdrant_client::{
    Qdrant,
    qdrant::{
        Condition, CreateCollectionBuilder, Distance, Filter, PointStruct, ScrollPointsBuilder,
        SearchPointsBuilder, UpsertPointsBuilder, VectorParamsBuilder,
    },
};

use super::VectorStore;
use crate::types::SearchResult;

/// Qdrant vector store implementation.
pub struct QdrantStore {
    client: Arc<Qdrant>,
}

impl QdrantStore {
    /// Connect to a Qdrant instance at the given gRPC URL (e.g. `http://localhost:6334`).
    pub fn new(url: &str) -> Result<Self> {
        let client = Qdrant::from_url(url).build().map_err(|e| {
            orka_core::Error::Knowledge(format!("failed to connect to Qdrant: {e}"))
        })?;

        Ok(Self {
            client: Arc::new(client),
        })
    }
}

#[async_trait]
impl VectorStore for QdrantStore {
    async fn ensure_collection(&self, name: &str, dimensions: usize) -> Result<()> {
        let exists = self.client.collection_exists(name).await.map_err(|e| {
            orka_core::Error::Knowledge(format!("qdrant collection_exists failed: {e}"))
        })?;

        if !exists {
            self.client
                .create_collection(CreateCollectionBuilder::new(name).vectors_config(
                    VectorParamsBuilder::new(dimensions as u64, Distance::Cosine),
                ))
                .await
                .map_err(|e| {
                    orka_core::Error::Knowledge(format!(
                        "failed to create collection '{name}': {e}"
                    ))
                })?;
        }

        Ok(())
    }

    async fn upsert(
        &self,
        collection: &str,
        ids: &[String],
        vectors: &[Vec<f32>],
        payloads: &[HashMap<String, String>],
    ) -> Result<()> {
        let points: Vec<PointStruct> = ids
            .iter()
            .zip(vectors.iter())
            .zip(payloads.iter())
            .map(|((id, vector), payload)| {
                let payload_json: HashMap<String, qdrant_client::qdrant::Value> = payload
                    .iter()
                    .map(|(k, v)| {
                        (
                            k.clone(),
                            qdrant_client::qdrant::Value {
                                kind: Some(qdrant_client::qdrant::value::Kind::StringValue(
                                    v.clone(),
                                )),
                            },
                        )
                    })
                    .collect();

                PointStruct::new(id.clone(), vector.clone(), payload_json)
            })
            .collect();

        self.client
            .upsert_points(UpsertPointsBuilder::new(collection, points))
            .await
            .map_err(|e| orka_core::Error::Knowledge(format!("qdrant upsert failed: {e}")))?;

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
        let mut search =
            SearchPointsBuilder::new(collection, vector.to_vec(), limit as u64).with_payload(true);

        if let Some(threshold) = score_threshold {
            search = search.score_threshold(threshold);
        }

        if let Some(ref conditions) = filter {
            let must: Vec<Condition> = conditions
                .iter()
                .map(|(key, value)| Condition::matches(key.as_str(), value.clone()))
                .collect();
            search = search.filter(Filter::must(must));
        }

        let results = self
            .client
            .search_points(search)
            .await
            .map_err(|e| orka_core::Error::Knowledge(format!("qdrant search failed: {e}")))?;

        let search_results = results
            .result
            .into_iter()
            .map(|point| {
                let payload = point.payload;
                let mut metadata = HashMap::new();
                let mut content = String::new();
                let mut document_id = None;

                for (key, value) in &payload {
                    if let Some(qdrant_client::qdrant::value::Kind::StringValue(s)) = &value.kind {
                        if key == "content" {
                            content = s.clone();
                        } else if key == "document_id" {
                            document_id = Some(s.clone());
                        } else {
                            metadata.insert(key.clone(), s.clone());
                        }
                    }
                }

                SearchResult {
                    content,
                    score: point.score,
                    document_id,
                    metadata,
                }
            })
            .collect();

        Ok(search_results)
    }

    async fn list_documents(
        &self,
        collection: &str,
        limit: usize,
        filter: Option<HashMap<String, String>>,
    ) -> Result<Vec<HashMap<String, String>>> {
        let exists = self
            .client
            .collection_exists(collection)
            .await
            .map_err(|e| {
                orka_core::Error::Knowledge(format!("qdrant collection_exists failed: {e}"))
            })?;

        if !exists {
            return Ok(Vec::new());
        }

        let mut scroll = ScrollPointsBuilder::new(collection)
            .with_payload(true)
            .limit(limit as u32);

        if let Some(ref conditions) = filter {
            let must: Vec<Condition> = conditions
                .iter()
                .map(|(key, value)| Condition::matches(key.as_str(), value.clone()))
                .collect();
            scroll = scroll.filter(Filter::must(must));
        }

        let result = self
            .client
            .scroll(scroll)
            .await
            .map_err(|e| orka_core::Error::Knowledge(format!("qdrant scroll failed: {e}")))?;

        // Collect unique document_ids
        let mut seen_docs: HashMap<String, HashMap<String, String>> = HashMap::new();

        for point in &result.result {
            let mut doc_meta = HashMap::new();
            let mut doc_id = None;

            for (key, value) in &point.payload {
                if let Some(qdrant_client::qdrant::value::Kind::StringValue(s)) = &value.kind {
                    if key == "document_id" {
                        doc_id = Some(s.clone());
                    }
                    doc_meta.insert(key.clone(), s.clone());
                }
            }

            if let Some(id) = doc_id {
                let entry = seen_docs.entry(id.clone()).or_insert_with(|| {
                    let mut m = doc_meta.clone();
                    m.insert("chunk_count".into(), "0".into());
                    m
                });
                let count: usize = entry
                    .get("chunk_count")
                    .and_then(|c| c.parse().ok())
                    .unwrap_or(0);
                entry.insert("chunk_count".into(), (count + 1).to_string());
            }
        }

        Ok(seen_docs.into_values().collect())
    }
}
