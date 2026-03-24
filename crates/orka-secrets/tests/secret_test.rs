use orka_core::{SecretValue, traits::SecretManager};
use orka_secrets::RedisSecretManager;

async fn setup() -> (
    RedisSecretManager,
    testcontainers::ContainerAsync<testcontainers_modules::redis::Redis>,
) {
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::redis::Redis;

    let container = Redis::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(6379).await.unwrap();
    let url = format!("redis://127.0.0.1:{port}");
    let store = RedisSecretManager::new(&url).expect("create manager");
    (store, container)
}

#[tokio::test]
#[ignore] // requires Redis
async fn set_get_roundtrip() {
    let (mgr, _container) = setup().await;
    let secret = SecretValue::new(b"my-secret-value".to_vec());

    mgr.set_secret("api/key", &secret).await.unwrap();

    let retrieved = mgr.get_secret("api/key").await.unwrap();
    assert_eq!(retrieved.expose(), b"my-secret-value");
}

#[tokio::test]
#[ignore] // requires Redis
async fn get_nonexistent_returns_error() {
    let (mgr, _container) = setup().await;
    let result = mgr.get_secret("does/not/exist").await;
    assert!(result.is_err());
}

#[tokio::test]
#[ignore] // requires Redis
async fn overwrite_works() {
    let (mgr, _container) = setup().await;

    mgr.set_secret("db/pass", &SecretValue::new(b"old".to_vec()))
        .await
        .unwrap();
    mgr.set_secret("db/pass", &SecretValue::new(b"new".to_vec()))
        .await
        .unwrap();

    let retrieved = mgr.get_secret("db/pass").await.unwrap();
    assert_eq!(retrieved.expose(), b"new");
}

#[tokio::test]
#[ignore] // requires Redis
async fn binary_data_preserved() {
    let (mgr, _container) = setup().await;
    let binary: Vec<u8> = (0..=255).collect();
    let secret = SecretValue::new(binary.clone());

    mgr.set_secret("binary/data", &secret).await.unwrap();

    let retrieved = mgr.get_secret("binary/data").await.unwrap();
    assert_eq!(retrieved.expose(), binary.as_slice());
}
