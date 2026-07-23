//! Data flywheel: capture production inference traffic and collect feedback.
//!
//! Capture is opt-in per model (`models.capture_traffic`) and strictly
//! best-effort — a failed capture write must never fail or slow the inference
//! request that produced it. Feedback writes, by contrast, are the caller's
//! primary operation and propagate errors normally.

use uuid::Uuid;

use crate::dto::common::PaginatedResponse;
use crate::dto::feedback::InferenceSampleResponse;
use crate::error::{AppError, AppResult};
use crate::repositories::traits::{InferenceSampleRepository, ModelRepository};
use platform_shared::enums::FeedbackRating;

/// Captures larger than this are skipped (not truncated — a cut JSON body is
/// worthless as a training example). Generous for chat traffic.
pub const MAX_CAPTURE_BYTES: usize = 256 * 1024;

pub struct InferenceSampleService;

impl InferenceSampleService {
    /// Persist a captured request/response pair. Never returns an error:
    /// capture is telemetry, not a critical write — failures are logged.
    pub async fn capture_best_effort(
        repo: &dyn InferenceSampleRepository,
        tenant_id: Uuid,
        sample_id: Uuid,
        model_id: Uuid,
        api_key_id: Option<Uuid>,
        messages: serde_json::Value,
        response: &str,
    ) {
        let approx_size = messages.to_string().len() + response.len();
        if approx_size > MAX_CAPTURE_BYTES {
            tracing::debug!(
                model_id = %model_id,
                approx_size,
                "Skipping traffic capture: sample exceeds size cap"
            );
            return;
        }
        if let Err(e) = repo
            .insert(
                tenant_id, sample_id, model_id, api_key_id, messages, response,
            )
            .await
        {
            tracing::warn!(model_id = %model_id, error = %e, "Failed to capture inference sample");
        }
    }

    pub async fn list(
        repo: &dyn InferenceSampleRepository,
        tenant_id: Uuid,
        model_id: Uuid,
        rating: Option<FeedbackRating>,
        unrated_only: bool,
        offset: i64,
        limit: i64,
    ) -> AppResult<PaginatedResponse<InferenceSampleResponse>> {
        let rating = rating.map(|r| r.to_string());
        let (samples, total) = tokio::try_join!(
            repo.list_by_model(
                tenant_id,
                model_id,
                rating.clone(),
                unrated_only,
                offset,
                limit
            ),
            repo.count_by_model(tenant_id, model_id, rating, unrated_only),
        )?;

        Ok(PaginatedResponse {
            data: samples.into_iter().map(Into::into).collect(),
            total,
            offset,
            limit,
        })
    }

    /// Dashboard feedback: sample id comes from the path, tenant scoping via repo.
    pub async fn submit_feedback(
        repo: &dyn InferenceSampleRepository,
        tenant_id: Uuid,
        sample_id: Uuid,
        rating: FeedbackRating,
        comment: Option<String>,
    ) -> AppResult<()> {
        let updated = repo
            .set_rating(tenant_id, sample_id, &rating.to_string(), comment)
            .await?;
        if !updated {
            return Err(AppError::NotFound {
                message: "Sample not found".to_string(),
            });
        }
        Ok(())
    }

    /// API-key feedback (`POST /v1/feedback`): the key is bound to one model,
    /// so the sample must belong to that model — a key must not be able to
    /// rate another model's traffic within the same tenant.
    pub async fn submit_api_feedback(
        repo: &dyn InferenceSampleRepository,
        tenant_id: Uuid,
        key_model_id: Uuid,
        sample_id: Uuid,
        rating: FeedbackRating,
        comment: Option<String>,
    ) -> AppResult<()> {
        let sample = repo
            .get_by_id(tenant_id, sample_id)
            .await?
            .ok_or(AppError::NotFound {
                message: "Sample not found".to_string(),
            })?;
        if sample.model_id != key_model_id {
            return Err(AppError::NotFound {
                message: "Sample not found".to_string(),
            });
        }
        Self::submit_feedback(repo, tenant_id, sample_id, rating, comment).await
    }

    pub async fn set_capture(
        model_repo: &dyn ModelRepository,
        tenant_id: Uuid,
        model_id: Uuid,
        enabled: bool,
    ) -> AppResult<()> {
        let updated = model_repo
            .set_capture_traffic(tenant_id, model_id, enabled)
            .await?;
        if !updated {
            return Err(AppError::NotFound {
                message: "Model not found".to_string(),
            });
        }
        Ok(())
    }
}

/// Extract the assistant completion text from a non-streaming
/// OpenAI-compatible response body.
pub fn extract_completion_text(response: &serde_json::Value) -> Option<String> {
    response["choices"][0]["message"]["content"]
        .as_str()
        .map(str::to_string)
}

/// Accumulate assistant content deltas from an OpenAI-compatible SSE chunk.
/// `chunk_text` may hold multiple `data: {...}` lines; non-JSON lines and
/// `[DONE]` are ignored.
pub fn append_stream_content(acc: &mut String, chunk_text: &str) {
    for line in chunk_text.lines() {
        if let Some(data) = line.strip_prefix("data: ")
            && let Ok(parsed) = serde_json::from_str::<serde_json::Value>(data)
            && let Some(delta) = parsed["choices"][0]["delta"]["content"].as_str()
        {
            acc.push_str(delta);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_completion_from_openai_response() {
        let resp = serde_json::json!({
            "choices": [{"index": 0, "message": {"role": "assistant", "content": "hello"}}]
        });
        assert_eq!(extract_completion_text(&resp), Some("hello".to_string()));
    }

    #[test]
    fn missing_choices_yields_none() {
        assert_eq!(extract_completion_text(&serde_json::json!({})), None);
        let no_content = serde_json::json!({"choices": [{"message": {"role": "assistant"}}]});
        assert_eq!(extract_completion_text(&no_content), None);
    }

    #[test]
    fn accumulates_stream_deltas_across_chunks() {
        let mut acc = String::new();
        append_stream_content(
            &mut acc,
            "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\"}}]}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\n",
        );
        append_stream_content(
            &mut acc,
            "data: {\"choices\":[{\"delta\":{\"content\":\"lo\"}}]}\n\ndata: [DONE]\n\n",
        );
        assert_eq!(acc, "Hello");
    }

    #[test]
    fn ignores_usage_only_and_malformed_chunks() {
        let mut acc = String::new();
        append_stream_content(&mut acc, "data: {\"usage\":{\"prompt_tokens\":3}}\n\n");
        append_stream_content(&mut acc, "not sse at all");
        append_stream_content(&mut acc, "data: {broken json");
        assert_eq!(acc, "");
    }
}
