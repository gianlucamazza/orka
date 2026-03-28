#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;

use orka_knowledge::vector_store::{VectorStore, qdrant::QdrantStore};
use testcontainers::{ContainerAsync, GenericImage, core::WaitFor, runners::AsyncRunner};

async fn setup_qdrant() -> (QdrantStore, ContainerAsync<GenericImage>) {
    let container = GenericImage::new("qdrant/qdrant", "v1.12.1")
        .with_exposed_port(6334.into())
        .with_wait_for(WaitFor::message_on_stderr("gRPC listening"))
        .start()
        .await
        .unwrap();
    let port = container.get_host_port_ipv4(6334).await.unwrap();
    let store = QdrantStore::new(&format!("http://127.0.0.1:{port}")).unwrap();
    (store, container)
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

#[tokio::test]
#[ignore = "requires a running Qdrant container"]
async fn ensure_collection_idempotent() {
    let (store, _c) = setup_qdrant().await;
    store.ensure_collection("test_col", 4).await.unwrap();
    store.ensure_collection("test_col", 4).await.unwrap();
}

#[tokio::test]
#[ignore = "requires a running Qdrant container"]
async fn upsert_and_search_basic() {
    let (store, _c) = setup_qdrant().await;
    store.ensure_collection("test_col", 4).await.unwrap();

    let ids = vec!["id1".to_string(), "id2".to_string(), "id3".to_string()];
    let vectors = vec![vec4(0.1), vec4(0.5), vec4(0.9)];
    let payloads = vec![
        payload(&[("content", "doc one"), ("document_id", "doc1")]),
        payload(&[("content", "doc two"), ("document_id", "doc2")]),
        payload(&[("content", "doc three"), ("document_id", "doc3")]),
    ];
    store
        .upsert("test_col", &ids, &vectors, &payloads)
        .await
        .unwrap();

    let results = store
        .search("test_col", &vec4(0.5), 2, None, None)
        .await
        .unwrap();
    assert!(!results.is_empty());
    assert!(results[0].score > 0.0);
}

#[tokio::test]
#[ignore = "requires a running Qdrant container"]
async fn search_with_filter() {
    let (store, _c) = setup_qdrant().await;
    store.ensure_collection("test_col", 4).await.unwrap();

    let ids = vec!["s1".to_string(), "s2".to_string(), "s3".to_string()];
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
        .upsert("test_col", &ids, &vectors, &payloads)
        .await
        .unwrap();

    let mut filter = HashMap::new();
    filter.insert("scope".to_string(), "global".to_string());
    let results = store
        .search("test_col", &vec4(0.1), 10, None, Some(filter))
        .await
        .unwrap();

    assert!(!results.is_empty());
    for r in &results {
        assert_eq!(r.metadata.get("scope").map(String::as_str), Some("global"));
    }
}

#[tokio::test]
#[ignore = "requires a running Qdrant container"]
async fn search_score_threshold() {
    let (store, _c) = setup_qdrant().await;
    store.ensure_collection("test_col", 4).await.unwrap();

    let ids = vec!["t1".to_string(), "t2".to_string()];
    let vectors = vec![vec![1.0, 0.0, 0.0, 0.0], vec![0.0, 1.0, 0.0, 0.0]];
    let payloads = vec![
        payload(&[("content", "close"), ("document_id", "t1")]),
        payload(&[("content", "far"), ("document_id", "t2")]),
    ];
    store
        .upsert("test_col", &ids, &vectors, &payloads)
        .await
        .unwrap();

    // High threshold: only the identical vector should match
    let results = store
        .search("test_col", &[1.0, 0.0, 0.0, 0.0], 10, Some(0.99), None)
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].content, "close");
}

#[tokio::test]
#[ignore = "requires a running Qdrant container"]
async fn list_documents_basic() {
    let (store, _c) = setup_qdrant().await;
    store.ensure_collection("test_col", 4).await.unwrap();

    let ids = vec!["l1".to_string(), "l2".to_string()];
    let vectors = vec![vec4(0.1), vec4(0.2)];
    let payloads = vec![
        payload(&[("document_id", "doc-a"), ("title", "Alpha")]),
        payload(&[("document_id", "doc-b"), ("title", "Beta")]),
    ];
    store
        .upsert("test_col", &ids, &vectors, &payloads)
        .await
        .unwrap();

    let docs = store.list_documents("test_col", 10, None).await.unwrap();
    assert_eq!(docs.len(), 2);
}

#[tokio::test]
#[ignore = "requires a running Qdrant container"]
async fn list_documents_with_filter() {
    let (store, _c) = setup_qdrant().await;
    store.ensure_collection("test_col", 4).await.unwrap();

    let ids = vec!["lf1".to_string(), "lf2".to_string(), "lf3".to_string()];
    let vectors = vec![vec4(0.1), vec4(0.2), vec4(0.3)];
    let payloads = vec![
        payload(&[("document_id", "doc-x"), ("workspace", "ws-a")]),
        payload(&[("document_id", "doc-y"), ("workspace", "ws-b")]),
        payload(&[("document_id", "doc-z"), ("workspace", "ws-a")]),
    ];
    store
        .upsert("test_col", &ids, &vectors, &payloads)
        .await
        .unwrap();

    let mut filter = HashMap::new();
    filter.insert("workspace".to_string(), "ws-a".to_string());
    let docs = store
        .list_documents("test_col", 10, Some(filter))
        .await
        .unwrap();

    assert_eq!(docs.len(), 2);
    for doc in &docs {
        assert_eq!(doc.get("workspace").map(String::as_str), Some("ws-a"));
    }
}
