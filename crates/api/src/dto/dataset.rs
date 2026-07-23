use chrono::{DateTime, Utc};
use platform_db::models::Dataset;
use platform_shared::enums::DatasetStatus;
use serde::Serialize;
use ts_rs::TS;
use utoipa::ToSchema;

/// Dataset information returned by API.
#[derive(Debug, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct DatasetResponse {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub format: String,
    pub status: DatasetStatus,
    pub pair_count: Option<i32>,
    #[schema(value_type = Object)]
    pub stats: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<Dataset> for DatasetResponse {
    fn from(d: Dataset) -> Self {
        Self {
            id: d.id.to_string(),
            project_id: d.project_id.to_string(),
            name: d.name,
            format: d.format,
            status: d.status.parse().unwrap_or(DatasetStatus::Generating),
            pair_count: d.pair_count,
            stats: d.stats,
            created_at: d.created_at,
            updated_at: d.updated_at,
        }
    }
}

/// A single rejected row from a dataset import.
#[derive(Debug, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct DatasetImportRowError {
    /// 1-based line number within the uploaded file.
    pub line: u32,
    /// Why the row was rejected.
    pub error: String,
}

/// Result of importing an OpenAI-format chat JSONL dataset. The created dataset
/// enters the same review flow as a generated one (`review_pending`); rejected
/// rows are reported per row rather than failing the whole file.
#[derive(Debug, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct DatasetImportResponse {
    pub dataset: DatasetResponse,
    /// Rows accepted and stored.
    pub imported_rows: u32,
    /// Rows rejected as malformed.
    pub rejected_rows: u32,
    /// Per-row errors (may be truncated; `rejected_rows` is the true total).
    pub errors: Vec<DatasetImportRowError>,
}
