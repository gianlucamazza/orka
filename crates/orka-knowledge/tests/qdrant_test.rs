#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;

use orka_knowledge::vector_store::{VectorStore, qdrant::QdrantStore};
use orka_test_support::{QdrantService, unique_name};
use uuid::Uuid;

async fn setup_qdrant() -> (QdrantStore, QdrantService) {
    let qdrant = QdrantService::discover().await.unwrap();
    let store = QdrantStore::new(qdrant.url());
    (store, qdrant)
}

fn vec4(x: f32) -> Vec<f32> {
    vec![x, x, x, x]
}

fn payload(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn collection_name(prefix: &str) -> String {
    unique_name(&format!("test-col-{prefix}"))
}

fn point_id() -> String {
    Uuid::new_v4().to_string()
}

#[serial_test::serial]
#[tokio::test]
#[ignore = "requires a running Qdrant container"]
async fn ensure_collection_idempotent() {
    let (store, _c) = setup_qdrant().await;
    let collection = collection_name("ensure");
    store.ensure_collection(&collection, 4).await.unwrap();
    store.ensure_collection(&collection, 4).await.unwrap();
}

#[serial_test::serial]
#[tokio::test]
#[ignore = "requires a running Qdrant container"]
async fn upsert_and_search_basic() {
    let (store, _c) = setup_qdrant().await;
    let collection = collection_name("basic");
    store.ensure_collection(&collection, 4).await.unwrap();

    let ids = vec![point_id(), point_id(), point_id()];
    let vectors = vec![vec4(0.1), vec4(0.5), vec4(0.9)];
    let payloads = vec![
        payload(&[("content", "doc one"), ("document_id", "doc1")]),
        payload(&[("content", "doc two"), ("document_id", "doc2")]),
        payload(&[("content", "doc three"), ("document_id", "doc3")]),
    ];
    store
        .upsert(&collection, &ids, &vectors, &payloads)
        .await
        .unwrap();

    let results = store
        .search(&collection, &vec4(0.5), 2, None, None)
        .await
        .unwrap();
    assert!(!results.is_empty());
    assert!(results[0].score > 0.0);
}

#[serial_test::serial]
#[tokio::test]
#[ignore = "requires a running Qdrant container"]
async fn search_with_filter() {
    let (store, _c) = setup_qdrant().await;
    let collection = collection_name("filter");
    store.ensure_collection(&collection, 4).await.unwrap();

    let ids = vec![point_id(), point_id(), point_id()];
    let vectors = vec![vec4(0.1), vec4(0.1), vec4(0.1)];
    let payloads = vec![
        payload(&[
            ("content", "alpha"),
            ("scope", "global"),
            ("document_id", "d1"),
        ]),
        payload(&[
            ("content", "beta"),
            ("scope", "workspace"),
            ("document_id", "d2"),
        ]),
        payload(&[
            ("content", "gamma"),
            ("scope", "global"),
            ("document_id", "d3"),
        ]),
    ];
    store
        .upsert(&collection, &ids, &vectors, &payloads)
        .await
        .unwrap();

    let mut filter = HashMap::new();
    filter.insert("scope".to_string(), "global".to_string());
    let results = store
        .search(&collection, &vec4(0.1), 10, None, Some(filter))
        .await
        .unwrap();

    assert!(!results.is_empty());
    for r in &results {
        assert_eq!(r.metadata.get("scope").map(String::as_str), Some("global"));
    }
}

#[serial_test::serial]
#[tokio::test]
#[ignore = "requires a running Qdrant container"]
async fn search_score_threshold() {
    let (store, _c) = setup_qdrant().await;
    let collection = collection_name("threshold");
    store.ensure_collection(&collection, 4).await.unwrap();

    let ids = vec![point_id(), point_id()];
    let vectors = vec![vec![1.0, 0.0, 0.0, 0.0], vec![0.0, 1.0, 0.0, 0.0]];
    let payloads = vec![
        payload(&[("content", "close"), ("document_id", "t1")]),
        payload(&[("content", "far"), ("document_id", "t2")]),
    ];
    store
        .upsert(&collection, &ids, &vectors, &payloads)
        .await
        .unwrap();

    // High threshold: only the identical vector should match
    let results = store
        .search(&collection, &[1.0, 0.0, 0.0, 0.0], 10, Some(0.99), None)
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].content, "close");
}

#[serial_test::serial]
#[tokio::test]
#[ignore = "requires a running Qdrant container"]
async fn list_documents_basic() {
    let (store, _c) = setup_qdrant().await;
    let collection = collection_name("list-basic");
    store.ensure_collection(&collection, 4).await.unwrap();

    let ids = vec![point_id(), point_id()];
    let vectors = vec![vec4(0.1), vec4(0.2)];
    let payloads = vec![
        payload(&[("document_id", "doc-a"), ("title", "Alpha")]),
        payload(&[("document_id", "doc-b"), ("title", "Beta")]),
    ];
    store
        .upsert(&collection, &ids, &vectors, &payloads)
        .await
        .unwrap();

    let docs = store.list_documents(&collection, 10, None).await.unwrap();
    assert_eq!(docs.len(), 2);
}

#[serial_test::serial]
#[tokio::test]
#[ignore = "requires a running Qdrant container"]
async fn list_documents_with_filter() {
    let (store, _c) = setup_qdrant().await;
    let collection = collection_name("list-filter");
    store.ensure_collection(&collection, 4).await.unwrap();

    let ids = vec![point_id(), point_id(), point_id()];
    let vectors = vec![vec4(0.1), vec4(0.2), vec4(0.3)];
    let payloads = vec![
        payload(&[("document_id", "doc-x"), ("workspace", "ws-a")]),
        payload(&[("document_id", "doc-y"), ("workspace", "ws-b")]),
        payload(&[("document_id", "doc-z"), ("workspace", "ws-a")]),
    ];
    store
        .upsert(&collection, &ids, &vectors, &payloads)
        .await
        .unwrap();

    let mut filter = HashMap::new();
    filter.insert("workspace".to_string(), "ws-a".to_string());
    let docs = store
        .list_documents(&collection, 10, Some(filter))
        .await
        .unwrap();

    assert_eq!(docs.len(), 2);
    for doc in &docs {
        assert_eq!(doc.get("workspace").map(String::as_str), Some("ws-a"));
    }
}
