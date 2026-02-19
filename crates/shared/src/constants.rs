/// Temporal task queue for main orchestration workflows.
pub const TEMPORAL_TASK_QUEUE_MAIN: &str = "ml-pipeline-main";

/// Temporal task queue for GPU-bound activities (training, evaluation).
pub const TEMPORAL_TASK_QUEUE_GPU: &str = "ml-pipeline-gpu";

/// Temporal namespace.
pub const TEMPORAL_NAMESPACE: &str = "default";

/// Redis key prefix for rate limiting.
pub const REDIS_RATE_LIMIT_PREFIX: &str = "rl:";

/// Redis key prefix for job status cache.
pub const REDIS_JOB_STATUS_PREFIX: &str = "job:";

/// Redis stream name template for real-time training metrics.
/// Replace `{job_id}` with the actual training job UUID.
pub const REDIS_TRAINING_METRICS_STREAM: &str = "training:metrics:";

/// Redis key prefix for session cache.
pub const REDIS_SESSION_PREFIX: &str = "session:";

/// Default max upload file size: 500 MB.
pub const MAX_UPLOAD_SIZE_BYTES: u64 = 500 * 1024 * 1024;

/// Default max batch upload size: 10 GB.
pub const MAX_BATCH_UPLOAD_SIZE_BYTES: u64 = 10 * 1024 * 1024 * 1024;

/// Supported file extensions for document upload.
pub const SUPPORTED_EXTENSIONS: &[&str] = &[
    "pdf", "docx", "doc", "txt", "html", "htm", "epub", "md", "csv", "png", "jpg", "jpeg", "tiff",
    "bmp",
];

/// Maximum pages for document parsing before chunking.
pub const DEFAULT_CHUNK_SIZE_TOKENS: usize = 1500;

/// API key prefix for display (e.g., pl_sk_xxxx...).
pub const API_KEY_PREFIX: &str = "pl_sk_";

/// Default rate limit per API key (requests per minute).
pub const DEFAULT_RATE_LIMIT_RPM: u32 = 60;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upload_limit_is_500mb() {
        assert_eq!(MAX_UPLOAD_SIZE_BYTES, 500 * 1024 * 1024);
    }

    #[test]
    fn supported_extensions_include_common_types() {
        assert!(SUPPORTED_EXTENSIONS.contains(&"pdf"));
        assert!(SUPPORTED_EXTENSIONS.contains(&"docx"));
        assert!(SUPPORTED_EXTENSIONS.contains(&"txt"));
        assert!(SUPPORTED_EXTENSIONS.contains(&"csv"));
    }

    #[test]
    fn api_key_prefix_is_not_empty() {
        assert!(!API_KEY_PREFIX.is_empty());
        assert!(API_KEY_PREFIX.ends_with('_'));
    }

    #[test]
    fn temporal_queues_are_distinct() {
        assert_ne!(TEMPORAL_TASK_QUEUE_MAIN, TEMPORAL_TASK_QUEUE_GPU);
    }
}
