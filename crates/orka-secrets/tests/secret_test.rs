#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]

use orka_core::{SecretValue, traits::SecretManager};
use orka_secrets::RedisSecretManager;
use orka_test_support::{RedisService, unique_name};

async fn setup() -> (RedisSecretManager, RedisService) {
    let redis = RedisService::discover().await.unwrap();
    let store = RedisSecretManager::new(redis.url()).expect("create manager");
    (store, redis)
}

#[tokio::test]
#[ignore = "requires Redis"]
async fn set_get_roundtrip() {
    let (mgr, _container) = setup().await;
    let secret = SecretValue::new(b"my-secret-value".to_vec());
    let path = unique_name("api-key");

    mgr.set_secret(&path, &secret).await.unwrap();

    let retrieved = mgr.get_secret(&path).await.unwrap();
    assert_eq!(retrieved.expose(), b"my-secret-value");
}

#[tokio::test]
#[ignore = "requires Redis"]
async fn get_nonexistent_returns_error() {
    let (mgr, _container) = setup().await;
    let result = mgr.get_secret("does/not/exist").await;
    assert!(result.is_err());
}

#[tokio::test]
#[ignore = "requires Redis"]
async fn overwrite_works() {
    let (mgr, _container) = setup().await;
    let path = unique_name("db-pass");

    mgr.set_secret(&path, &SecretValue::new(b"old".to_vec()))
        .await
        .unwrap();
    mgr.set_secret(&path, &SecretValue::new(b"new".to_vec()))
        .await
        .unwrap();

    let retrieved = mgr.get_secret(&path).await.unwrap();
    assert_eq!(retrieved.expose(), b"new");
}

#[tokio::test]
#[ignore = "requires Redis"]
async fn binary_data_preserved() {
    let (mgr, _container) = setup().await;
    let binary: Vec<u8> = (0..=255).collect();
    let secret = SecretValue::new(binary.clone());
    let path = unique_name("binary-data");

    mgr.set_secret(&path, &secret).await.unwrap();

    let retrieved = mgr.get_secret(&path).await.unwrap();
    assert_eq!(retrieved.expose(), binary.as_slice());
}
