//! Message Bus Benchmarks
//!
//! Run with: `cargo bench --bench message_bus`

use criterion::{Criterion, criterion_group, criterion_main, BenchmarkId, Throughput};
use orka_core::testing::InMemoryBus;
use orka_core::traits::MessageBus;
use orka_core::types::{Envelope, SessionId};
use std::time::Duration;
use tokio::runtime::Runtime;

/// Benchmark single-threaded message publishing
fn bench_publish(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let bus = InMemoryBus::new();

    let mut group = c.benchmark_group("message_bus_publish");
    
    for size in [10, 100, 1000].iter() {
        group.throughput(Throughput::Elements(*size as u64));
        group.bench_with_input(BenchmarkId::new("publish", size), size, |b, &size| {
            b.to_async(&rt).iter(|| async {
                for _ in 0..size {
                    let envelope = Envelope::text("test", SessionId::new(), "benchmark");
                    bus.publish("test_topic", &envelope).await.unwrap();
                }
            });
        });
    }
    
    group.finish();
}

/// Benchmark publish-subscribe with single subscriber
fn bench_pub_sub(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    let mut group = c.benchmark_group("message_bus_pub_sub");
    group.measurement_time(Duration::from_secs(10));

    for msg_count in [100, 1000].iter() {
        group.throughput(Throughput::Elements(*msg_count as u64));
        group.bench_with_input(
            BenchmarkId::new("pub_sub", msg_count),
            msg_count,
            |b, &msg_count| {
                b.to_async(&rt).iter(|| async {
                    let bus = InMemoryBus::new();
                    let mut rx = bus.subscribe("test_topic").await.unwrap();

                    // Spawn consumer
                    let consumer = tokio::spawn(async move {
                        let mut count = 0;
                        while let Some(_msg) = rx.recv().await {
                            count += 1;
                            if count >= msg_count {
                                break;
                            }
                        }
                    });

                    // Publish messages
                    for _ in 0..msg_count {
                        let envelope = Envelope::text("test", SessionId::new(), "benchmark");
                        bus.publish("test_topic", &envelope).await.unwrap();
                    }

                    consumer.await.unwrap();
                });
            },
        );
    }

    group.finish();
}

/// Benchmark with multiple concurrent publishers
fn bench_concurrent_publish(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    let mut group = c.benchmark_group("message_bus_concurrent");
    group.measurement_time(Duration::from_secs(10));

    for publishers in [2, 4, 8].iter() {
        let messages_per_publisher = 1000 / publishers;
        group.throughput(Throughput::Elements(1000));
        group.bench_with_input(
            BenchmarkId::new("concurrent_publish", publishers),
            publishers,
            |b, &publishers| {
                b.to_async(&rt).iter(|| async {
                    let bus = std::sync::Arc::new(InMemoryBus::new());
                    let mut handles = vec![];

                    for _ in 0..publishers {
                        let bus = bus.clone();
                        let handle = tokio::spawn(async move {
                            for _ in 0..messages_per_publisher {
                                let envelope = Envelope::text("test", SessionId::new(), "benchmark");
                                bus.publish("test_topic", &envelope).await.unwrap();
                            }
                        });
                        handles.push(handle);
                    }

                    for handle in handles {
                        handle.await.unwrap();
                    }
                });
            },
        );
    }

    group.finish();
}

/// Benchmark message size impact
fn bench_message_size(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let bus = InMemoryBus::new();

    let mut group = c.benchmark_group("message_bus_size");

    for size in [100, 1000, 10000].iter() {
        let text = "x".repeat(*size);
        group.throughput(Throughput::Bytes(*size as u64));
        group.bench_with_input(
            BenchmarkId::new("message_size", size),
            &text,
            |b, text| {
                b.to_async(&rt).iter(|| async {
                    let envelope = Envelope::text("test", SessionId::new(), text);
                    bus.publish("test_topic", &envelope).await.unwrap();
                });
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_publish,
    bench_pub_sub,
    bench_concurrent_publish,
    bench_message_size
);
criterion_main!(benches);
