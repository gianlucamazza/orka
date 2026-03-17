use async_trait::async_trait;
use orka_core::traits::Skill;
use orka_core::{Result, SkillInput, SkillOutput, SkillSchema};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

use crate::chunking;
use crate::embeddings::EmbeddingProvider;
use crate::parsers;
use crate::vector_store::VectorStore;

pub struct DocIngestSkill {
    embeddings: Arc<dyn EmbeddingProvider>,
    store: Arc<dyn VectorStore>,
    default_collection: String,
    default_chunk_size: usize,
    default_chunk_overlap: usize,
}

impl DocIngestSkill {
    pub fn new(
        embeddings: Arc<dyn EmbeddingProvider>,
        store: Arc<dyn VectorStore>,
        default_collection: String,
        default_chunk_size: usize,
        default_chunk_overlap: usize,
    ) -> Self {
        Self {
            embeddings,
            store,
            default_collection,
            default_chunk_size,
            default_chunk_overlap,
        }
    }
}

#[async_trait]
impl Skill for DocIngestSkill {
    fn name(&self) -> &str {
        "doc_ingest"
    }

    fn description(&self) -> &str {
        "Ingest a document (PDF/HTML/MD/TXT) by parsing, chunking, embedding, and storing it"
    }

    fn schema(&self) -> SkillSchema {
        SkillSchema::new(serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Local file path to the document"
                },
                "url": {
                    "type": "string",
                    "description": "URL to fetch the document from"
                },
                "collection": {
                    "type": "string",
                    "description": "Collection to store in (optional)"
                },
                "chunk_size": {
                    "type": "integer",
                    "description": "Characters per chunk (optional)"
                },
                "chunk_overlap": {
                    "type": "integer",
                    "description": "Overlap between chunks (optional)"
                }
            }
        }))
    }

    async fn execute(&self, input: SkillInput) -> Result<SkillOutput> {
        let collection = input
            .args
            .get("collection")
            .and_then(|v| v.as_str())
            .unwrap_or(&self.default_collection);

        let chunk_size = input
            .args
            .get("chunk_size")
            .and_then(|v| v.as_u64())
            .unwrap_or(self.default_chunk_size as u64) as usize;

        let chunk_overlap = input
            .args
            .get("chunk_overlap")
            .and_then(|v| v.as_u64())
            .unwrap_or(self.default_chunk_overlap as u64) as usize;

        // Read document data
        let (data, source) = if let Some(path) = input.args.get("path").and_then(|v| v.as_str()) {
            let data = tokio::fs::read(path).await.map_err(|e| {
                orka_core::Error::Knowledge(format!("failed to read file '{path}': {e}"))
            })?;
            (data, path.to_string())
        } else if let Some(url) = input.args.get("url").and_then(|v| v.as_str()) {
            let resp = reqwest::get(url).await.map_err(|e| {
                orka_core::Error::Knowledge(format!("failed to fetch URL '{url}': {e}"))
            })?;
            let data = resp
                .bytes()
                .await
                .map_err(|e| orka_core::Error::Knowledge(format!("failed to read response: {e}")))?
                .to_vec();
            (data, url.to_string())
        } else {
            return Err(orka_core::Error::Skill(
                "either 'path' or 'url' is required".into(),
            ));
        };

        // Parse document
        let parser = parsers::detect_format(&source);
        let text = parser.parse(&data)?;

        if text.is_empty() {
            return Ok(SkillOutput::new(serde_json::json!({
                "ingested": false,
                "reason": "document is empty after parsing",
            })));
        }

        // Chunk text
        let chunks = chunking::split_text(&text, chunk_size, chunk_overlap);

        // Ensure collection
        self.store
            .ensure_collection(collection, self.embeddings.dimensions())
            .await?;

        let document_id = Uuid::new_v4().to_string();

        // Embed and store chunks in batches
        let batch_size = 32;
        let mut total_stored = 0;

        for batch in chunks.chunks(batch_size) {
            let texts: Vec<String> = batch.to_vec();
            let embeddings = self.embeddings.embed(&texts).await?;

            let ids: Vec<String> = (0..batch.len())
                .map(|i| format!("{}-{}", document_id, total_stored + i))
                .collect();

            let payloads: Vec<HashMap<String, String>> = batch
                .iter()
                .enumerate()
                .map(|(i, chunk)| {
                    let mut m = HashMap::new();
                    m.insert("content".into(), chunk.clone());
                    m.insert("document_id".into(), document_id.clone());
                    m.insert("source".into(), source.clone());
                    m.insert("chunk_index".into(), (total_stored + i).to_string());
                    m.insert("ingested_at".into(), chrono::Utc::now().to_rfc3339());
                    m
                })
                .collect();

            self.store
                .upsert(collection, &ids, &embeddings, &payloads)
                .await?;
            total_stored += batch.len();
        }

        Ok(SkillOutput::new(serde_json::json!({
            "ingested": true,
            "document_id": document_id,
            "source": source,
            "collection": collection,
            "chunks": total_stored,
        })))
    }
}
