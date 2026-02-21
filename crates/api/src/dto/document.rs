use chrono::{DateTime, Utc};
use platform_shared::enums::DocumentStatus;
use serde::Serialize;
use ts_rs::TS;
use uuid::Uuid;

/// API response for a document.
#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct DocumentResponse {
    pub id: Uuid,
    pub project_id: Uuid,
    pub filename: String,
    pub file_size: i64,
    pub mime_type: String,
    pub status: DocumentStatus,
    pub parse_quality: Option<f64>,
    pub page_count: Option<i32>,
    pub language: Option<String>,
    pub domain: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<platform_db::models::Document> for DocumentResponse {
    fn from(d: platform_db::models::Document) -> Self {
        Self {
            id: d.id,
            project_id: d.project_id,
            filename: d.filename,
            file_size: d.file_size,
            mime_type: d.mime_type,
            status: d.status.parse().unwrap_or(DocumentStatus::Uploaded),
            parse_quality: d.parse_quality,
            page_count: d.page_count,
            language: d.language,
            domain: d.domain,
            created_at: d.created_at,
            updated_at: d.updated_at,
        }
    }
}

/// Response after successful document upload.
#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct UploadResponse {
    pub id: Uuid,
    pub filename: String,
    pub file_size: i64,
    pub status: DocumentStatus,
}
