use chrono::{DateTime, Utc};
use platform_db::models::DataGuide;
use platform_shared::enums::{DataGuideStatus, SampleRating, TaskType};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::ToSchema;

/// A candidate topic/angle extracted from source documents, surfaced for user review.
#[derive(Debug, Clone, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct Facet {
    pub id: String,
    pub label: String,
    pub source_doc_id: Option<String>,
    pub keep: bool,
}

/// A generated prompt/response pair shown to the user before full dataset generation.
#[derive(Debug, Clone, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct PreviewSample {
    pub id: String,
    pub facet_id: Option<String>,
    pub prompt: String,
    pub response: String,
    pub rating: Option<SampleRating>,
}

/// Data guide information returned by API.
#[derive(Debug, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct DataGuideResponse {
    pub id: String,
    pub project_id: String,
    pub task_type: TaskType,
    pub status: DataGuideStatus,
    pub guidance: String,
    pub facets: Vec<Facet>,
    pub preview_samples: Vec<PreviewSample>,
    pub dataset_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<DataGuide> for DataGuideResponse {
    fn from(g: DataGuide) -> Self {
        Self {
            id: g.id.to_string(),
            project_id: g.project_id.to_string(),
            task_type: g.task_type.parse().unwrap_or(TaskType::QuestionAnswering),
            status: g.status.parse().unwrap_or(DataGuideStatus::Draft),
            guidance: g.guidance,
            facets: serde_json::from_value(g.facets).unwrap_or_default(),
            preview_samples: serde_json::from_value(g.preview_samples).unwrap_or_default(),
            dataset_id: g.dataset_id.map(|id| id.to_string()),
            created_at: g.created_at,
            updated_at: g.updated_at,
        }
    }
}

/// Request body for creating a data guide.
#[derive(Debug, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct CreateDataGuideRequest {
    #[ts(optional)]
    pub task_type: Option<TaskType>,
}

/// Request body for generating candidate facets from source documents.
#[derive(Debug, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct GenerateFacetsRequest {
    #[ts(optional)]
    pub num_facets: Option<u32>,
}

/// Request body for updating the kept/discarded facets after user review.
#[derive(Debug, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct UpdateFacetsRequest {
    pub facets: Vec<Facet>,
}

/// Request body for generating preview samples from the kept facets.
#[derive(Debug, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct GeneratePreviewRequest {
    #[ts(optional)]
    pub num_samples: Option<u32>,
}

/// A single sample rating submitted by the user.
#[derive(Debug, Clone, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct SampleRatingItem {
    pub sample_id: String,
    pub rating: SampleRating,
}

/// Request body for rating preview samples.
#[derive(Debug, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct RateSamplesRequest {
    pub ratings: Vec<SampleRatingItem>,
}

/// Request body for updating the free-form guidance text.
#[derive(Debug, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct UpdateGuidanceRequest {
    pub guidance: String,
}

/// Request body for generating the full dataset from the stored data guide session.
#[derive(Debug, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct GenerateDatasetRequest {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_guide_response_exports_ts() {
        // ts-rs export is invoked in the workspace test run; here assert serde shape.
        let r = DataGuideResponse {
            id: String::new(),
            project_id: String::new(),
            task_type: TaskType::QuestionAnswering,
            status: DataGuideStatus::Draft,
            guidance: String::new(),
            facets: Vec::new(),
            preview_samples: Vec::new(),
            dataset_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let v = serde_json::to_value(&r).unwrap();
        assert!(v.get("preview_samples").is_some());
    }
}
