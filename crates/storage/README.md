# crates/storage

**S3-compatible object storage with a swappable trait abstraction.**

|            |                                                  |
| ---------- | ------------------------------------------------ |
| Language   | Rust                                             |
| Type       | Library crate                                    |
| Backend    | AWS S3, Cloudflare R2, MinIO (any S3-compatible) |
| SDK        | `aws-sdk-s3`                                     |
| Depends on | Nothing (standalone)                             |
| Used by    | `crates/api`                                     |

## What's Inside

| File     | Purpose                                                          |
| -------- | ---------------------------------------------------------------- |
| `lib.rs` | `ObjectStorage` trait — the abstraction interface with 5 methods |
| `s3.rs`  | `S3Storage` — production implementation using `aws-sdk-s3`       |

## The `ObjectStorage` Trait

```rust
trait ObjectStorage {
    async fn put(&self, key: &str, data: Bytes, content_type: &str) -> Result<()>;
    async fn get(&self, key: &str) -> Result<Bytes>;
    async fn exists(&self, key: &str) -> Result<bool>;
    async fn delete(&self, key: &str) -> Result<()>;
    async fn presigned_url(&self, key: &str, expiry_secs: u64) -> Result<String>;
}
```

## Trait-Based

Swap the storage backend without changing any business logic:

- **Local dev**: MinIO (via Docker Compose, `force_path_style: true`)
- **Production**: AWS S3 or Cloudflare R2
- **Tests**: In-memory implementation (not yet built)

The API service layer depends on `impl ObjectStorage`, never on `S3Storage` directly.

## Usage

This is a **library crate** — not runnable on its own. Used by `crates/api`.

Add to any crate's `Cargo.toml`:
```toml
[dependencies]
platform-storage = { workspace = true }
```

Instantiate in code:
```rust
use platform_storage::s3::{S3Storage, S3Config};

let storage = S3Storage::new(S3Config {
    endpoint: Some("http://localhost:9000".into()),  // MinIO
    access_key: "minioadmin".into(),
    secret_key: "minioadmin".into(),
    region: "us-east-1".into(),
    bucket: "platform".into(),
    force_path_style: true,  // Required for MinIO
}).await;

storage.ensure_bucket().await?;  // Create bucket if missing
```

### Run Tests

```bash
cargo test -p platform-storage
```
