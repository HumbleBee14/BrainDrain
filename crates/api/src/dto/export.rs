use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::ToSchema;

/// Request to start a GGUF export.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ExportRequest {
    /// Quantization type: Q4_K_M, Q5_K_M, Q6_K, Q8_0
    #[serde(default = "default_quant_type")]
    pub quant_type: String,
}

fn default_quant_type() -> String {
    "Q5_K_M".to_string()
}

/// Export response (returned from API).
#[derive(Debug, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct ExportResponse {
    pub id: String,
    pub model_id: String,
    pub format: String,
    pub quant_type: String,
    pub status: String,
    /// Whether a download is available (true when status is "completed").
    pub has_download: bool,
    pub file_size_bytes: Option<i64>,
    pub error: Option<String>,
    pub created_at: String,
    pub completed_at: Option<String>,
}

impl From<platform_db::models::ModelExport> for ExportResponse {
    fn from(e: platform_db::models::ModelExport) -> Self {
        Self {
            id: e.id.to_string(),
            model_id: e.model_id.to_string(),
            format: e.format,
            quant_type: e.quant_type,
            has_download: e.storage_path.is_some(),
            status: e.status,
            file_size_bytes: e.file_size_bytes,
            error: e.error,
            created_at: e.created_at.to_rfc3339(),
            completed_at: e.completed_at.map(|t| t.to_rfc3339()),
        }
    }
}

/// Presigned download URL response.
#[derive(Debug, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct ExportDownloadResponse {
    pub download_url: String,
    pub file_size_bytes: Option<i64>,
    pub filename: String,
}

/// Valid quantization types.
pub const VALID_QUANT_TYPES: &[&str] = &["Q4_K_M", "Q5_K_M", "Q6_K", "Q8_0"];
