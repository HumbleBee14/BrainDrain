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
