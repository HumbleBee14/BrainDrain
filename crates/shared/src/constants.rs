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

/// Supported file extensions for document upload. Every entry must have a real
/// text-extracting parser — no image/scanned formats, since there is no OCR path.
pub const SUPPORTED_EXTENSIONS: &[&str] = &["pdf", "docx", "txt", "html", "htm", "md", "csv"];

/// Maximum pages for document parsing before chunking.
pub const DEFAULT_CHUNK_SIZE_TOKENS: usize = 1500;

/// API key prefix for display (e.g., pl_sk_xxxx...).
pub const API_KEY_PREFIX: &str = "pl_sk_";

/// Default rate limit per API key (requests per minute).
pub const DEFAULT_RATE_LIMIT_RPM: u32 = 60;

/// Redis key prefix for IP-based rate limiting.
pub const REDIS_IP_RATE_LIMIT_PREFIX: &str = "ip_rl:";

/// Default rate limit per IP address (requests per minute).
pub const DEFAULT_IP_RATE_LIMIT_RPM: u32 = 200;

/// GPU hourly rates (USD) for training cost estimation.
/// These are approximate market rates as of early 2026.
/// Order matches `GpuClass` enum variants.
pub const GPU_HOURLY_RATES: &[(&str, f64)] = &[
    ("t4", 0.80),
    ("a10g", 1.20),
    ("l40s", 1.80),
    ("a10040gb", 2.00),
    ("a10080gb", 3.00),
    ("h100", 4.50),
];

/// Default GPU hourly rate when class is unknown.
pub const GPU_DEFAULT_HOURLY_RATE: f64 = 0.80;

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
