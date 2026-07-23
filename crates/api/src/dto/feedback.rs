use chrono::{DateTime, Utc};
use platform_db::models::InferenceSample;
use platform_shared::enums::FeedbackRating;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::ToSchema;

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
            created_at: s.created_at,
        }
    }
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
