//! Integration tests for the S3 streaming (multipart) upload path.
//!
//! These run against a live S3-compatible endpoint (MinIO in local dev). They
//! are skipped unless `S3_TEST_ENDPOINT` is set, so `cargo test` stays green in
//! environments without object storage. To run locally:
//!
//! ```bash
//! docker compose up -d minio
//! S3_TEST_ENDPOINT=http://localhost:9000 \
//!   S3_TEST_ACCESS_KEY=minioadmin S3_TEST_SECRET_KEY=minioadmin \
//!   cargo test -p platform-storage --test s3_streaming -- --nocapture
//! ```

use bytes::Bytes;
use platform_storage::ObjectStorage;
use platform_storage::s3::{S3Config, S3Storage};

async fn test_storage() -> Option<S3Storage> {
    let endpoint = std::env::var("S3_TEST_ENDPOINT")
        .ok()
        .filter(|s| !s.is_empty())?;
    let access_key = std::env::var("S3_TEST_ACCESS_KEY").unwrap_or_else(|_| "minioadmin".into());
    let secret_key = std::env::var("S3_TEST_SECRET_KEY").unwrap_or_else(|_| "minioadmin".into());

    let config = S3Config {
        endpoint: Some(endpoint),
        access_key,
        secret_key,
        region: "us-east-1".to_string(),
        bucket: "platform-streaming-test".to_string(),
        force_path_style: true,
    };
    Some(S3Storage::new(config).await)
}

/// Feed `total` bytes as a stream of `chunk` sized pieces.
fn byte_stream(
    total: usize,
    chunk: usize,
) -> impl futures::Stream<Item = Result<Bytes, platform_storage::StorageError>> {
    let chunks: Vec<Result<Bytes, platform_storage::StorageError>> = (0..total)
        .step_by(chunk)
        .map(|start| {
            let end = (start + chunk).min(total);
            // Deterministic, position-dependent bytes so a corrupted/misordered
            // part is caught on read-back.
            let data: Vec<u8> = (start..end).map(|i| (i % 251) as u8).collect();
            Ok(Bytes::from(data))
        })
        .collect();
    futures::stream::iter(chunks)
}

fn expected_bytes(total: usize) -> Bytes {
    Bytes::from((0..total).map(|i| (i % 251) as u8).collect::<Vec<u8>>())
}

#[tokio::test]
async fn multipart_round_trip_large_object() {
    let Some(storage) = test_storage().await else {
        eprintln!("skipping: S3_TEST_ENDPOINT not set");
        return;
    };
    storage.ensure_bucket().await.expect("ensure bucket");

    // 20 MiB in 64 KiB chunks → 2 full 8 MiB parts + a 4 MiB final part.
    let total = 20 * 1024 * 1024;
    let key = "streaming/large.bin";
    let written = storage
        .put_streaming(
            key,
            byte_stream(total, 64 * 1024),
            "application/octet-stream",
        )
        .await
        .expect("streaming upload");
    assert_eq!(written, total as u64);

    let got = storage.get(key).await.expect("download");
    assert_eq!(got.len(), total);
    assert_eq!(got, expected_bytes(total));

    storage.delete(key).await.expect("cleanup");
}

#[tokio::test]
async fn streaming_small_object_uses_single_put() {
    let Some(storage) = test_storage().await else {
        eprintln!("skipping: S3_TEST_ENDPOINT not set");
        return;
    };
    storage.ensure_bucket().await.expect("ensure bucket");

    let total = 1024; // well under one part
    let key = "streaming/small.bin";
    let written = storage
        .put_streaming(key, byte_stream(total, 128), "application/octet-stream")
        .await
        .expect("streaming upload");
    assert_eq!(written, total as u64);
    assert_eq!(
        storage.get(key).await.expect("download"),
        expected_bytes(total)
    );

    storage.delete(key).await.expect("cleanup");
}

#[tokio::test]
async fn streaming_empty_object_round_trips() {
    let Some(storage) = test_storage().await else {
        eprintln!("skipping: S3_TEST_ENDPOINT not set");
        return;
    };
    storage.ensure_bucket().await.expect("ensure bucket");

    let key = "streaming/empty.bin";
    let empty: Vec<Result<Bytes, platform_storage::StorageError>> = vec![];
    let written = storage
        .put_streaming(
            key,
            futures::stream::iter(empty),
            "application/octet-stream",
        )
        .await
        .expect("streaming upload");
    assert_eq!(written, 0);
    assert_eq!(storage.get(key).await.expect("download").len(), 0);

    storage.delete(key).await.expect("cleanup");
}

#[tokio::test]
async fn streaming_error_aborts_without_committing() {
    let Some(storage) = test_storage().await else {
        eprintln!("skipping: S3_TEST_ENDPOINT not set");
        return;
    };
    storage.ensure_bucket().await.expect("ensure bucket");

    let key = "streaming/aborted.bin";
    // One full part, then an error before completion.
    let big = expected_bytes(9 * 1024 * 1024);
    let chunks: Vec<Result<Bytes, platform_storage::StorageError>> = vec![
        Ok(big),
        Err(platform_storage::StorageError::UploadFailed(
            "cap tripped".into(),
        )),
    ];
    let result = storage
        .put_streaming(
            key,
            futures::stream::iter(chunks),
            "application/octet-stream",
        )
        .await;
    assert!(result.is_err());
    // The multipart upload was aborted, so nothing was committed at the key.
    assert!(
        storage.get(key).await.is_err(),
        "aborted upload must not be readable"
    );
}
