//! Data flywheel: capture production inference traffic and collect feedback.
//!
//! Capture is opt-in per model (`models.capture_traffic`) and strictly
//! best-effort — a failed capture write must never fail or slow the inference
//! request that produced it. Feedback writes, by contrast, are the caller's
//! primary operation and propagate errors normally.

use uuid::Uuid;

use crate::dto::common::PaginatedResponse;
use crate::dto::feedback::{
    InferenceSampleResponse, PromoteSamplesRequest, PromoteSamplesResponse,
};
use crate::error::{AppError, AppResult};
use crate::repositories::traits::{DatasetRepository, InferenceSampleRepository, ModelRepository};
use crate::services::dataset_service::DatasetService;
use platform_shared::enums::FeedbackRating;
use platform_storage::ObjectStorage;

/// Captures larger than this are skipped (not truncated — a cut JSON body is
/// worthless as a training example). Generous for chat traffic.
pub const MAX_CAPTURE_BYTES: usize = 256 * 1024;

/// Cap per promote call — a dataset is created synchronously in the request.
pub const MAX_PROMOTE_SAMPLES: usize = 500;

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

    /// Promote captured samples into a new `review_pending` training dataset
    /// in the model's project (stage 2 of the flywheel).
    ///
    /// A negative-rated sample must carry a corrected response — promoting a
    /// response the user flagged as bad would poison the training set. The
    /// captured response is used as the assistant turn otherwise.
    pub async fn promote(
        sample_repo: &dyn InferenceSampleRepository,
        model_repo: &dyn ModelRepository,
        dataset_repo: &dyn DatasetRepository,
        storage: &impl ObjectStorage,
        tenant_id: Uuid,
        model_id: Uuid,
        req: PromoteSamplesRequest,
    ) -> AppResult<PromoteSamplesResponse> {
        if req.samples.is_empty() {
            return Err(AppError::BadRequest {
                message: "No samples selected".to_string(),
            });
        }
        if req.samples.len() > MAX_PROMOTE_SAMPLES {
            return Err(AppError::BadRequest {
                message: format!("At most {MAX_PROMOTE_SAMPLES} samples per promotion"),
            });
        }

        let model = model_repo
            .get_by_id(tenant_id, model_id)
            .await?
            .ok_or(AppError::NotFound {
                message: "Model not found".to_string(),
            })?;

        let mut records = Vec::with_capacity(req.samples.len());
        let mut sample_ids = Vec::with_capacity(req.samples.len());
        for item in &req.samples {
            let sample_id: Uuid = item.sample_id.parse().map_err(|_| AppError::BadRequest {
                message: format!("sample_id '{}' is not a UUID", item.sample_id),
            })?;
            let sample = sample_repo
                .get_by_id(tenant_id, sample_id)
                .await?
                .filter(|s| s.model_id == model_id)
                .ok_or(AppError::NotFound {
                    message: format!("Sample {sample_id} not found"),
                })?;
            if sample.promoted_at.is_some() {
                return Err(AppError::BadRequest {
                    message: format!("Sample {sample_id} was already promoted"),
                });
            }
            let correction = item
                .corrected_response
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty());
            if sample.rating.as_deref() == Some("negative") && correction.is_none() {
                return Err(AppError::BadRequest {
                    message: format!(
                        "Sample {sample_id} is rated negative — provide a corrected response before promoting"
                    ),
                });
            }
            let response_text = correction.unwrap_or(&sample.response);
            records.push(sample_to_record(&sample.messages, response_text, sample_id));
            sample_ids.push(sample_id);
        }

        let default_name = format!("Production Feedback — {}", model.name);
        let name = req
            .name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(&default_name);

        let dataset = DatasetService::store_records_as_dataset(
            dataset_repo,
            storage,
            tenant_id,
            model.project_id,
            name,
            &records,
            serde_json::json!({
                "source": "production_feedback",
                "model_id": model_id.to_string(),
            }),
        )
        .await?;

        let promoted = sample_repo.mark_promoted(tenant_id, &sample_ids).await?;

        Ok(PromoteSamplesResponse {
            dataset: dataset.into(),
            promoted_count: promoted as u32,
        })
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

/// Build a training record from a captured sample: the captured conversation
/// plus the (possibly corrected) response as the final assistant turn, in the
/// same internal shape as imported/generated dataset records.
pub fn sample_to_record(
    messages: &serde_json::Value,
    response_text: &str,
    sample_id: Uuid,
) -> serde_json::Value {
    let mut msgs = messages.as_array().cloned().unwrap_or_default();
    msgs.push(serde_json::json!({"role": "assistant", "content": response_text}));
    serde_json::json!({
        "messages": msgs,
        "metadata": {"source": "production_feedback", "sample_id": sample_id.to_string()},
    })
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
    fn sample_record_appends_assistant_turn() {
        let messages = serde_json::json!([
            {"role": "system", "content": "You are helpful."},
            {"role": "user", "content": "What is the refund window?"}
        ]);
        let id = Uuid::nil();
        let record = sample_to_record(&messages, "30 days.", id);
        let msgs = record["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[2]["role"], "assistant");
        assert_eq!(msgs[2]["content"], "30 days.");
        assert_eq!(record["metadata"]["source"], "production_feedback");
        assert_eq!(record["metadata"]["sample_id"], id.to_string());
    }

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
