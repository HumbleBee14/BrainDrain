use chrono::{DateTime, Utc};
use platform_db::models::InferenceSample;
use platform_shared::enums::FeedbackRating;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::ToSchema;

use crate::dto::dataset::DatasetResponse;

/// One message of a captured inference request.
#[derive(Debug, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct SampleMessage {
    pub role: String,
    pub content: String,
}

/// A captured production inference request/response pair (data flywheel).
#[derive(Debug, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct InferenceSampleResponse {
    pub id: String,
    pub model_id: String,
    pub messages: Vec<SampleMessage>,
    pub response: String,
    pub rating: Option<FeedbackRating>,
    pub rating_comment: Option<String>,
    pub promoted_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl From<InferenceSample> for InferenceSampleResponse {
    fn from(s: InferenceSample) -> Self {
        Self {
            id: s.id.to_string(),
            model_id: s.model_id.to_string(),
            messages: serde_json::from_value(s.messages).unwrap_or_default(),
            response: s.response,
            rating: s.rating.and_then(|r| r.parse().ok()),
            rating_comment: s.rating_comment,
            promoted_at: s.promoted_at,
            created_at: s.created_at,
        }
    }
}

/// One sample selected for promotion into a training dataset.
#[derive(Debug, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct PromoteSampleItem {
    pub sample_id: String,
    /// Replacement assistant response. Required for negative-rated samples.
    #[ts(optional)]
    pub corrected_response: Option<String>,
}

/// Request to promote captured samples into a new training dataset.
#[derive(Debug, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct PromoteSamplesRequest {
    pub samples: Vec<PromoteSampleItem>,
    /// Dataset name; defaults to "Production Feedback — {model name}".
    #[ts(optional)]
    pub name: Option<String>,
}

/// Result of promoting samples: the created dataset (in `review_pending`).
#[derive(Debug, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct PromoteSamplesResponse {
    pub dataset: DatasetResponse,
    pub promoted_count: u32,
}

/// Feedback on a sample, submitted from the dashboard (sample id in path).
#[derive(Debug, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct SubmitFeedbackRequest {
    pub rating: FeedbackRating,
    #[ts(optional)]
    pub comment: Option<String>,
}

/// Feedback submitted by the tenant's own application through the
/// OpenAI-compatible API (`POST /v1/feedback`), referencing the `x-sample-id`
/// returned by a chat completion.
#[derive(Debug, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct ApiFeedbackRequest {
    pub sample_id: String,
    pub rating: FeedbackRating,
    #[ts(optional)]
    pub comment: Option<String>,
}

/// Request to toggle traffic capture on a model.
#[derive(Debug, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct SetCaptureRequest {
    pub enabled: bool,
}
