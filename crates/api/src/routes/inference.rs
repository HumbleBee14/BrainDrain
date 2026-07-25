use axum::body::Body;
use axum::extract::State;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use futures::StreamExt;
use platform_shared::enums::DeploymentStatus;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::app_state::AppState;
use crate::auth_api_key::ApiKeyAuth;
use crate::error::{AppError, AppResult};
use crate::services::billing_outbox;
use crate::services::inference_backend::InferenceBackend;
use crate::services::inference_instance_service::InferenceInstanceService;
use crate::services::inference_sample_service::{
    InferenceSampleService, append_stream_content, extract_completion_text,
};
use crate::services::plan_service::PlanService;
use crate::services::token_estimator;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

/// Response header carrying the captured sample id, so API callers can submit
/// feedback on this specific completion via `POST /v1/feedback`.
const SAMPLE_ID_HEADER: &str = "x-sample-id";

/// Maximum number of items in a single batch request.
const MAX_BATCH_SIZE: usize = 50;

/// Concurrent inference requests per batch.
const BATCH_CONCURRENCY: usize = 5;

async fn resolve_backend_for_model(
    state: &AppState,
    model: &platform_db::models::Model,
) -> AppResult<Arc<dyn InferenceBackend>> {
    if let Some(instance_id) = model.inference_instance_id {
        let instance = InferenceInstanceService::get_routable_instance(state, instance_id).await?;
        let backend_name =
            model.deployment_config["backend"]
                .as_str()
                .ok_or(AppError::Internal(anyhow::anyhow!(
                    "Model deployment config missing backend"
                )))?;
        if backend_name != instance.backend_type {
            return Err(AppError::BadRequest {
                message: format!(
                    "Model deployment backend '{}' does not match assigned instance backend '{}'",
                    backend_name, instance.backend_type
                ),
            });
        }

        Ok(state.build_inference_backend_for_instance(&instance.backend_type, &instance.base_url))
    } else {
        // Single-instance fallback: use the global backend with its shared
        // circuit breaker. Do NOT construct an ephemeral backend here —
        // that creates a fresh breaker per request that never accumulates state.
        Ok(state.inference_backend_arc())
    }
}

/// Inference routes — OpenAI-compatible API.
/// These are mounted at `/v1/` (not `/api/v1/`) and use API key auth.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/chat/completions", post(chat_completions))
        .route("/v1/chat/completions/batch", post(batch_chat_completions))
}

/// OpenAI-compatible chat completion request (subset of fields we support).
#[derive(Debug, Deserialize, ToSchema)]
pub struct ChatCompletionRequest {
    /// Client-specified model name (ignored — we route by API key's model).
    #[serde(default)]
    #[allow(dead_code)]
    model: Option<String>,
    messages: Vec<ChatMessage>,
    #[serde(default = "default_temperature")]
    temperature: f64,
    #[serde(default = "default_max_tokens")]
    max_tokens: i64,
    #[serde(default)]
    top_p: Option<f64>,
    #[serde(default)]
    stream: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, Clone, ToSchema)]
pub struct ChatMessage {
    role: String,
    content: String,
}

/// Prepend the model's trained system prompt when the caller sent none, so the
/// model is served under the same system prompt it was fine-tuned on. A
/// caller-supplied system message always wins; an empty/absent stored prompt is
/// a no-op (preserving the prior pass-through behavior).
fn with_default_system_prompt(messages: &[ChatMessage], default_prompt: &str) -> Vec<ChatMessage> {
    let has_system = messages.iter().any(|m| m.role == "system");
    if default_prompt.is_empty() || has_system {
        return messages.to_vec();
    }
    let mut out = Vec::with_capacity(messages.len() + 1);
    out.push(ChatMessage {
        role: "system".to_string(),
        content: default_prompt.to_string(),
    });
    out.extend_from_slice(messages);
    out
}

fn default_temperature() -> f64 {
    0.7
}

fn default_max_tokens() -> i64 {
    512
}

/// How a completed non-streaming (or batch) inference should be billed, given
/// whether the durable reservation path is active and the token usage observed.
///
/// Kept pure so the reserve/finalize/cancel decision is unit-tested without a
/// live database; the handler carries the reservation row id separately.
#[derive(Debug, PartialEq, Eq)]
enum BillingAction {
    /// Relay active: finalize the reserved row with these token counts.
    FinalizeReservation(i64, i64),
    /// Relay active but no billable usage occurred: cancel the reservation.
    CancelReservation,
    /// Relay inactive: write a single durable billing event with these counts.
    SingleWrite(i64, i64),
    /// Nothing to bill (relay inactive and no usage observed).
    NoWrite,
}

/// Decide how to bill a single successful non-streaming completion.
///
/// The upstream call already succeeded (failures cancel the reservation before
/// reaching here), so GPU work was consumed: when the backend omits usage we
/// finalize with the conservative pre-call estimate rather than dropping the
/// charge.
fn non_streaming_billing_action(
    relay_enabled: bool,
    reported_in: i64,
    reported_out: i64,
    estimate_in: i64,
    estimate_out: i64,
) -> BillingAction {
    let has_usage = reported_in > 0 || reported_out > 0;
    if relay_enabled {
        let (bill_in, bill_out) = if has_usage {
            (reported_in, reported_out)
        } else {
            (estimate_in, estimate_out)
        };
        BillingAction::FinalizeReservation(bill_in, bill_out)
    } else if has_usage {
        BillingAction::SingleWrite(reported_in, reported_out)
    } else {
        BillingAction::NoWrite
    }
}

/// Decide how to bill a batch given the aggregate usage across its items.
///
/// A batch may have every item fail, so with no aggregate usage the reservation
/// is cancelled instead of finalized to a conservative charge for work that
/// never happened.
fn batch_billing_action(relay_enabled: bool, total_in: i64, total_out: i64) -> BillingAction {
    let has_usage = total_in > 0 || total_out > 0;
    if relay_enabled {
        if has_usage {
            BillingAction::FinalizeReservation(total_in, total_out)
        } else {
            BillingAction::CancelReservation
        }
    } else if has_usage {
        BillingAction::SingleWrite(total_in, total_out)
    } else {
        BillingAction::NoWrite
    }
}

/// OpenAI-compatible chat completion response (schema-only for docs).
#[derive(Serialize, ToSchema)]
pub struct ChatCompletionResponse {
    id: String,
    object: String,
    created: i64,
    model: String,
    choices: Vec<ChatChoice>,
    usage: ChatUsage,
}

#[derive(Serialize, ToSchema)]
pub struct ChatChoice {
    index: i32,
    message: ChatMessage,
    finish_reason: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct ChatUsage {
    prompt_tokens: i64,
    completion_tokens: i64,
    total_tokens: i64,
}

/// OpenAI-compatible chat completion endpoint.
///
/// Proxies the request to the inference backend, routing to the correct
/// LoRA adapter based on the API key's associated model.
/// Supports both streaming (SSE) and non-streaming responses.
#[utoipa::path(
    post,
    path = "/v1/chat/completions",
    tag = "Inference",
    request_body = ChatCompletionRequest,
    responses(
        (status = 200, description = "Chat completion response", body = ChatCompletionResponse),
        (status = 400, body = crate::error::ErrorEnvelope),
        (status = 401, body = crate::error::ErrorEnvelope),
    ),
    security(("api_key" = []))
)]
pub async fn chat_completions(
    State(state): State<AppState>,
    api_key: ApiKeyAuth,
    Json(body): Json<ChatCompletionRequest>,
) -> AppResult<Response> {
    // Verify model is actively deployed
    let model = state
        .model_repo()
        .get_by_id(api_key.tenant_id, api_key.model_id)
        .await?
        .ok_or(AppError::NotFound {
            message: "Model not found".to_string(),
        })?;

    if model.deployment_status != DeploymentStatus::Active.to_string() {
        return Err(AppError::BadRequest {
            message: format!(
                "Model is not deployed (status: {}). Deploy the model first.",
                model.deployment_status
            ),
        });
    }

    // Reject before doing billable work once the tenant is over its monthly cap.
    PlanService::check_spend_cap(
        state.tenant_repo(),
        state.billing_event_repo(),
        api_key.tenant_id,
    )
    .await?;

    let adapter_name = model.deployment_config["adapter_ref"]
        .as_str()
        .ok_or(AppError::Internal(anyhow::anyhow!(
            "Model deployment config missing adapter_ref"
        )))?
        .to_string();

    let backend = resolve_backend_for_model(&state, &model).await?;

    // Cap max_tokens to prevent GPU abuse (configurable via INFERENCE_MAX_TOKENS)
    let max_tokens = body.max_tokens.min(state.config().inference_max_tokens);

    let is_streaming = body.stream.unwrap_or(false);

    let messages = with_default_system_prompt(
        &body.messages,
        model.deployment_config["system_prompt"]
            .as_str()
            .unwrap_or(""),
    );

    // Data flywheel: when capture is enabled the sample id is minted up front
    // so it can be returned in the response headers before the body/stream.
    let sample_id = model.capture_traffic.then(Uuid::new_v4);
    let capture_messages = sample_id.map(|_| serde_json::json!(&messages));

    // Build the inference request via the backend (handles adapter selection correctly)
    let mut inference_request = backend.build_inference_body(
        &adapter_name,
        &serde_json::json!(messages),
        body.temperature,
        max_tokens,
        body.top_p.unwrap_or(1.0),
        is_streaming,
    );

    // For streaming, request usage in the final chunk
    if is_streaming {
        inference_request["stream_options"] = serde_json::json!({"include_usage": true});
    }

    let inference_url = backend.chat_completions_url();
    let http_client = state.inference_http_client().clone();

    // Prompt-token estimate used as the conservative billing fallback for both
    // the streaming reservation (below) and the non-streaming reservation.
    let estimated_prompt_tokens = token_estimator::estimate_tokens_from_messages(
        body.messages.iter().map(|m| m.content.as_str()),
    );

    // Non-streaming reserves a durable conservative billing row BEFORE the
    // upstream call so a crash after the GPU work but before we read usage is
    // still finalized — by us with actuals below, or by the reaper with the
    // fallback. Streaming reserves after its own success check further down.
    let non_streaming_pending = if !is_streaming && state.billing_outbox_relay_handle().is_some() {
        Some(
            billing_outbox::enqueue_stream_pending(
                state.db(),
                api_key.tenant_id,
                Some(api_key.model_id),
                estimated_prompt_tokens,
                max_tokens,
                token_estimator::estimate_inference_cost(estimated_prompt_tokens, max_tokens),
                serde_json::json!({"api_key_id": api_key.key_id.to_string()}),
            )
            .await?,
        )
    } else {
        None
    };

    let vllm_resp = match backend
        .circuit_breaker()
        .execute(|| async {
            backend
                .apply_auth(http_client.post(&inference_url))
                .json(&inference_request)
                .send()
                .await
                .map_err(|e| {
                    AppError::Internal(anyhow::anyhow!("Cannot reach inference service: {e}"))
                })
        })
        .await
    {
        Ok(resp) => resp,
        Err(e) => {
            // No billable work occurred — drop the reservation so the reaper
            // does not later charge the fallback for a failed call.
            if let Some(row_id) = non_streaming_pending {
                billing_outbox::cancel_stream_pending(state.db(), row_id).await;
            }
            return Err(e);
        }
    };

    if !vllm_resp.status().is_success() {
        let status = vllm_resp.status();
        let body_text = vllm_resp.text().await.unwrap_or_default();
        tracing::error!(status = %status, body = %body_text, "Inference request failed");
        if let Some(row_id) = non_streaming_pending {
            billing_outbox::cancel_stream_pending(state.db(), row_id).await;
        }
        return Err(AppError::Internal(anyhow::anyhow!(
            "Inference request failed: {status}"
        )));
    }

    if is_streaming {
        // SSE streaming: forward the byte stream and extract usage from final chunk
        let tenant_id = api_key.tenant_id;
        let model_id = api_key.model_id;
        let key_id = api_key.key_id;

        let vllm_stream = vllm_resp.bytes_stream();

        // Tee the stream: forward bytes to client AND capture usage from final chunk
        let (usage_tx, mut usage_rx) = tokio::sync::mpsc::channel::<(i64, i64)>(1);

        // Flywheel capture: accumulate assistant deltas as chunks pass through.
        // The usage chunk is the last data chunk, so by the time the finalizer
        // below wakes up the accumulator holds the complete response.
        let capture_acc = sample_id.map(|_| Arc::new(Mutex::new(String::new())));
        let acc_for_stream = capture_acc.clone();

        let forwarded_stream =
            vllm_stream.map(move |chunk_result: Result<bytes::Bytes, reqwest::Error>| {
                match chunk_result {
                    Ok(bytes) => {
                        // Scan for usage in the final SSE chunk
                        // OpenAI-compat SSE: data: {"usage":{"prompt_tokens":N,"completion_tokens":N}}
                        let text = String::from_utf8_lossy(&bytes);
                        if let Some(acc) = &acc_for_stream
                            && let Ok(mut guard) = acc.lock()
                        {
                            append_stream_content(&mut guard, &text);
                        }
                        for line in text.lines() {
                            if let Some(data) = line.strip_prefix("data: ")
                                && let Ok(parsed) = serde_json::from_str::<serde_json::Value>(data)
                                && let Some(usage) = parsed.get("usage")
                            {
                                let tokens_in = usage["prompt_tokens"].as_i64().unwrap_or(0);
                                let tokens_out = usage["completion_tokens"].as_i64().unwrap_or(0);
                                if tokens_in > 0 || tokens_out > 0 {
                                    let _ = usage_tx.try_send((tokens_in, tokens_out));
                                }
                            }
                        }
                        Ok::<_, std::io::Error>(bytes)
                    }
                    Err(e) => Err(std::io::Error::other(e.to_string())),
                }
            });

        let stream_metadata = serde_json::json!({
            "api_key_id": key_id.to_string(),
            "stream": true,
        });
        let pending_billing_row = if state.billing_outbox_relay_handle().is_some() {
            Some(
                billing_outbox::enqueue_stream_pending(
                    state.db(),
                    tenant_id,
                    Some(model_id),
                    estimated_prompt_tokens,
                    max_tokens,
                    token_estimator::estimate_inference_cost(estimated_prompt_tokens, max_tokens),
                    stream_metadata.clone(),
                )
                .await?,
            )
        } else {
            None
        };

        // Spawn a finalizer for the pending streaming outbox row. If the process
        // dies mid-stream, the row remains pending and the relay's stale-pending
        // reaper finalizes the conservative fallback charge durably.
        let batcher_state = state.clone();
        let db = state.db().clone();
        let capped_max_tokens = max_tokens;
        let capture_repo = sample_id.map(|_| state.inference_sample_repo_arc());
        tokio::spawn(async move {
            let (tokens_in, tokens_out, completed) = match usage_rx.recv().await {
                Some((tokens_in, tokens_out)) => (tokens_in, tokens_out, true),
                None => {
                    // Client disconnected before usage chunk — bill conservatively
                    tracing::warn!(
                        model_id = %model_id,
                        "Client disconnected before usage chunk; billing estimate"
                    );
                    (estimated_prompt_tokens, capped_max_tokens, false)
                }
            };

            // Capture only completed responses — a partial answer cut off by a
            // client disconnect is not a usable training example.
            if completed
                && let (Some(id), Some(repo), Some(msgs), Some(acc)) =
                    (sample_id, capture_repo, capture_messages, capture_acc)
            {
                let response_text = acc.lock().map(|g| g.clone()).unwrap_or_default();
                if !response_text.is_empty() {
                    InferenceSampleService::capture_best_effort(
                        &*repo,
                        tenant_id,
                        id,
                        model_id,
                        Some(key_id),
                        msgs,
                        &response_text,
                    )
                    .await;
                }
            }

            let cost = token_estimator::estimate_inference_cost(tokens_in, tokens_out);
            if let Some(row_id) = pending_billing_row {
                if let Err(e) = billing_outbox::finalize_stream_pending(
                    &db,
                    row_id,
                    tokens_in,
                    tokens_out,
                    cost,
                    stream_metadata,
                )
                .await
                {
                    tracing::error!(error = %e, row_id = %row_id, "Failed to finalize stream billing row");
                }
            } else {
                batcher_state
                    .record_billing_event_best_effort(
                        tenant_id,
                        "inference",
                        Some(model_id),
                        tokens_in,
                        tokens_out,
                        0,
                        cost,
                        serde_json::json!({"api_key_id": key_id.to_string(), "stream": true}),
                    )
                    .await;
            }
        });

        let body = Body::from_stream(forwarded_stream);

        let mut builder = Response::builder()
            .header(CONTENT_TYPE, "text/event-stream")
            .header(CACHE_CONTROL, "no-cache");
        if let Some(id) = sample_id {
            builder = builder.header(SAMPLE_ID_HEADER, id.to_string());
        }
        let resp = builder.body(body).map_err(|e| {
            AppError::Internal(anyhow::anyhow!("Failed to build SSE response: {e}"))
        })?;
        Ok(resp.into_response())
    } else {
        // Non-streaming: parse JSON response and bill
        let response: serde_json::Value = vllm_resp.json().await.map_err(|e| {
            AppError::Internal(anyhow::anyhow!("Failed to parse inference response: {e}"))
        })?;

        let tokens_in = response["usage"]["prompt_tokens"].as_i64().unwrap_or(0);
        let tokens_out = response["usage"]["completion_tokens"].as_i64().unwrap_or(0);

        match non_streaming_billing_action(
            non_streaming_pending.is_some(),
            tokens_in,
            tokens_out,
            estimated_prompt_tokens,
            max_tokens,
        ) {
            BillingAction::FinalizeReservation(bill_in, bill_out) => {
                if let Some(row_id) = non_streaming_pending {
                    // Finalize the reservation with actuals. A finalize failure
                    // must NOT fail the request — the reaper delivers the
                    // conservative fallback still on the row.
                    let cost = token_estimator::estimate_inference_cost(bill_in, bill_out);
                    if let Err(e) = billing_outbox::finalize_stream_pending(
                        state.db(),
                        row_id,
                        bill_in,
                        bill_out,
                        cost,
                        serde_json::json!({"api_key_id": api_key.key_id.to_string()}),
                    )
                    .await
                    {
                        tracing::error!(error = %e, row_id = %row_id, "Failed to finalize non-streaming billing row");
                    }
                }
            }
            BillingAction::SingleWrite(bill_in, bill_out) => {
                state
                    .record_billing_event_required(
                        api_key.tenant_id,
                        "inference",
                        Some(api_key.model_id),
                        bill_in,
                        bill_out,
                        0,
                        token_estimator::estimate_inference_cost(bill_in, bill_out),
                        serde_json::json!({"api_key_id": api_key.key_id.to_string()}),
                    )
                    .await?;
            }
            BillingAction::NoWrite | BillingAction::CancelReservation => {}
        }

        if let (Some(id), Some(msgs)) = (sample_id, capture_messages)
            && let Some(completion) = extract_completion_text(&response)
        {
            InferenceSampleService::capture_best_effort(
                state.inference_sample_repo(),
                api_key.tenant_id,
                id,
                api_key.model_id,
                Some(api_key.key_id),
                msgs,
                &completion,
            )
            .await;
        }

        let mut resp = Json(response).into_response();
        if let Some(id) = sample_id
            && let Ok(value) = axum::http::HeaderValue::from_str(&id.to_string())
        {
            resp.headers_mut().insert(SAMPLE_ID_HEADER, value);
        }
        Ok(resp)
    }
}

/// A single item in a batch request.
#[derive(Debug, Deserialize, ToSchema)]
pub struct BatchRequestItem {
    /// Client-supplied identifier for correlating responses.
    pub custom_id: String,
    pub messages: Vec<ChatMessage>,
    #[serde(default = "default_temperature")]
    pub temperature: f64,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: i64,
    #[serde(default)]
    pub top_p: Option<f64>,
}

/// Batch chat completion request.
#[derive(Debug, Deserialize, ToSchema)]
pub struct BatchChatCompletionRequest {
    pub requests: Vec<BatchRequestItem>,
}

/// Result of a single item in the batch response.
#[derive(Debug, Serialize, ToSchema)]
pub struct BatchResponseItem {
    pub custom_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Batch chat completion response.
#[derive(Debug, Serialize, ToSchema)]
pub struct BatchChatCompletionResponse {
    pub results: Vec<BatchResponseItem>,
    pub usage: BatchUsageSummary,
}

/// Aggregated usage across all batch items.
#[derive(Debug, Serialize, ToSchema)]
pub struct BatchUsageSummary {
    pub total_prompt_tokens: i64,
    pub total_completion_tokens: i64,
    pub total_tokens: i64,
    pub successful: usize,
    pub failed: usize,
}

/// POST /v1/chat/completions/batch
///
/// Process multiple chat completion requests in a single API call.
/// Items are processed concurrently with bounded parallelism.
/// Streaming is not supported for batch requests.
#[utoipa::path(
    post,
    path = "/v1/chat/completions/batch",
    tag = "Inference",
    request_body = BatchChatCompletionRequest,
    responses(
        (status = 200, description = "Batch completion response", body = BatchChatCompletionResponse),
        (status = 400, body = crate::error::ErrorEnvelope),
        (status = 401, body = crate::error::ErrorEnvelope),
    ),
    security(("api_key" = []))
)]
pub async fn batch_chat_completions(
    State(state): State<AppState>,
    api_key: ApiKeyAuth,
    Json(body): Json<BatchChatCompletionRequest>,
) -> AppResult<Json<BatchChatCompletionResponse>> {
    if body.requests.is_empty() {
        return Err(AppError::BadRequest {
            message: "Batch must contain at least one request".to_string(),
        });
    }

    if body.requests.len() > MAX_BATCH_SIZE {
        return Err(AppError::BadRequest {
            message: format!("Batch size exceeds maximum of {MAX_BATCH_SIZE}"),
        });
    }

    // Verify model is actively deployed (once for the entire batch)
    let model = state
        .model_repo()
        .get_by_id(api_key.tenant_id, api_key.model_id)
        .await?
        .ok_or(AppError::NotFound {
            message: "Model not found".to_string(),
        })?;

    if model.deployment_status != DeploymentStatus::Active.to_string() {
        return Err(AppError::BadRequest {
            message: format!(
                "Model is not deployed (status: {}). Deploy the model first.",
                model.deployment_status
            ),
        });
    }

    // Reject before doing billable work once the tenant is over its monthly cap.
    PlanService::check_spend_cap(
        state.tenant_repo(),
        state.billing_event_repo(),
        api_key.tenant_id,
    )
    .await?;

    let adapter_name = model.deployment_config["adapter_ref"]
        .as_str()
        .ok_or(AppError::Internal(anyhow::anyhow!(
            "Model deployment config missing adapter_ref"
        )))?
        .to_string();

    let batch_backend = resolve_backend_for_model(&state, &model).await?;

    let batch_url = batch_backend.chat_completions_url();
    let http_client = state.inference_http_client().clone();
    let max_tokens_limit = state.config().inference_max_tokens;
    let default_system_prompt = model.deployment_config["system_prompt"]
        .as_str()
        .unwrap_or("")
        .to_string();

    let batch_size = body.requests.len();

    // Conservative batch reservation: summed prompt-token estimates plus capped
    // max_tokens across all items, committed durably BEFORE any upstream work so
    // a crash mid-batch is finalized by the reaper.
    let batch_est_prompt: i64 = body
        .requests
        .iter()
        .map(|it| {
            token_estimator::estimate_tokens_from_messages(
                it.messages.iter().map(|m| m.content.as_str()),
            )
        })
        .sum();
    let batch_est_completion: i64 = body
        .requests
        .iter()
        .map(|it| it.max_tokens.min(max_tokens_limit))
        .sum();

    let batch_pending = if state.billing_outbox_relay_handle().is_some() {
        Some(
            billing_outbox::enqueue_stream_pending(
                state.db(),
                api_key.tenant_id,
                Some(api_key.model_id),
                batch_est_prompt,
                batch_est_completion,
                token_estimator::estimate_inference_cost(batch_est_prompt, batch_est_completion),
                serde_json::json!({
                    "api_key_id": api_key.key_id.to_string(),
                    "batch": true,
                    "batch_size": batch_size,
                }),
            )
            .await?,
        )
    } else {
        None
    };

    // Process batch items concurrently with bounded parallelism
    let results: Vec<BatchResponseItem> = futures::stream::iter(body.requests)
        .map(|item| {
            let adapter = adapter_name.clone();
            let url = batch_url.clone();
            let client = http_client.clone();
            let cb = batch_backend.circuit_breaker().clone();
            let backend = batch_backend.clone();
            let system_prompt = default_system_prompt.clone();

            async move {
                let max_tokens = item.max_tokens.min(max_tokens_limit);
                let messages = with_default_system_prompt(&item.messages, &system_prompt);
                let request_body = backend.build_inference_body(
                    &adapter,
                    &serde_json::json!(messages),
                    item.temperature,
                    max_tokens,
                    item.top_p.unwrap_or(1.0),
                    false,
                );

                let resp = cb
                    .execute(|| async {
                        backend
                            .apply_auth(client.post(&url))
                            .json(&request_body)
                            .send()
                            .await
                            .map_err(|e| {
                                AppError::Internal(anyhow::anyhow!(
                                    "Cannot reach inference service: {e}"
                                ))
                            })
                    })
                    .await;

                match resp {
                    Ok(r) if r.status().is_success() => match r.json::<serde_json::Value>().await {
                        Ok(json) => BatchResponseItem {
                            custom_id: item.custom_id,
                            response: Some(json),
                            error: None,
                        },
                        Err(e) => {
                            tracing::warn!(custom_id = %item.custom_id, error = %e, "Batch item response parse failed");
                            BatchResponseItem {
                                custom_id: item.custom_id,
                                response: None,
                                error: Some("Inference service returned an invalid response".into()),
                            }
                        }
                    },
                    Ok(r) => {
                        let status = r.status();
                        tracing::warn!(custom_id = %item.custom_id, status = %status, "Batch item inference failed");
                        BatchResponseItem {
                            custom_id: item.custom_id,
                            response: None,
                            error: Some(format!("Inference failed with status {status}")),
                        }
                    }
                    Err(e) => {
                        tracing::warn!(custom_id = %item.custom_id, error = %e, "Batch item request error");
                        BatchResponseItem {
                            custom_id: item.custom_id,
                            response: None,
                            error: Some("Inference service unavailable".into()),
                        }
                    }
                }
            }
        })
        .buffer_unordered(BATCH_CONCURRENCY)
        .collect()
        .await;

    // Aggregate usage and bill
    let mut total_prompt = 0i64;
    let mut total_completion = 0i64;
    let mut successful = 0usize;
    let mut failed = 0usize;

    for item in &results {
        if let Some(ref resp) = item.response {
            successful += 1;
            let tokens_in = resp["usage"]["prompt_tokens"].as_i64().unwrap_or(0);
            let tokens_out = resp["usage"]["completion_tokens"].as_i64().unwrap_or(0);
            total_prompt += tokens_in;
            total_completion += tokens_out;
        } else {
            failed += 1;
        }
    }

    let batch_metadata = serde_json::json!({
        "api_key_id": api_key.key_id.to_string(),
        "batch": true,
        "batch_size": results.len(),
    });

    match batch_billing_action(batch_pending.is_some(), total_prompt, total_completion) {
        BillingAction::FinalizeReservation(bill_in, bill_out) => {
            if let Some(row_id) = batch_pending {
                // Finalize failure must not fail the request — the reaper
                // delivers the conservative fallback still on the row.
                let cost = token_estimator::estimate_inference_cost(bill_in, bill_out);
                if let Err(e) = billing_outbox::finalize_stream_pending(
                    state.db(),
                    row_id,
                    bill_in,
                    bill_out,
                    cost,
                    batch_metadata,
                )
                .await
                {
                    tracing::error!(error = %e, row_id = %row_id, "Failed to finalize batch billing row");
                }
            }
        }
        BillingAction::CancelReservation => {
            // Every item failed — no billable work, so drop the reservation.
            if let Some(row_id) = batch_pending {
                billing_outbox::cancel_stream_pending(state.db(), row_id).await;
            }
        }
        BillingAction::SingleWrite(bill_in, bill_out) => {
            state
                .record_billing_event_required(
                    api_key.tenant_id,
                    "inference",
                    Some(api_key.model_id),
                    bill_in,
                    bill_out,
                    0,
                    token_estimator::estimate_inference_cost(bill_in, bill_out),
                    batch_metadata,
                )
                .await?;
        }
        BillingAction::NoWrite => {}
    }

    Ok(Json(BatchChatCompletionResponse {
        results,
        usage: BatchUsageSummary {
            total_prompt_tokens: total_prompt,
            total_completion_tokens: total_completion,
            total_tokens: total_prompt + total_completion,
            successful,
            failed,
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(role: &str, content: &str) -> ChatMessage {
        ChatMessage {
            role: role.to_string(),
            content: content.to_string(),
        }
    }

    #[test]
    fn injects_default_when_caller_sends_no_system_message() {
        let msgs = vec![msg("user", "hi")];
        let out = with_default_system_prompt(&msgs, "You are a legal assistant.");
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].role, "system");
        assert_eq!(out[0].content, "You are a legal assistant.");
        assert_eq!(out[1].role, "user");
    }

    #[test]
    fn caller_supplied_system_message_wins() {
        let msgs = vec![msg("system", "caller persona"), msg("user", "hi")];
        let out = with_default_system_prompt(&msgs, "trained persona");
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].content, "caller persona");
    }

    #[test]
    fn empty_default_is_pass_through() {
        let msgs = vec![msg("user", "hi")];
        let out = with_default_system_prompt(&msgs, "");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].role, "user");
    }

    // ── Non-streaming billing decision ──

    #[test]
    fn non_streaming_relay_finalizes_with_actuals() {
        // Relay on + real usage: finalize the reservation with the actuals.
        let action = non_streaming_billing_action(true, 120, 40, 10, 512);
        assert_eq!(action, BillingAction::FinalizeReservation(120, 40));
    }

    #[test]
    fn non_streaming_relay_finalizes_with_estimate_when_usage_missing() {
        // Relay on + backend omitted usage: GPU work still happened, so
        // finalize with the conservative pre-call estimate, never zero.
        let action = non_streaming_billing_action(true, 0, 0, 10, 512);
        assert_eq!(action, BillingAction::FinalizeReservation(10, 512));
    }

    #[test]
    fn non_streaming_no_relay_keeps_single_write() {
        // No relay + usage: preserve the prior single durable write behavior.
        let action = non_streaming_billing_action(false, 120, 40, 10, 512);
        assert_eq!(action, BillingAction::SingleWrite(120, 40));
    }

    #[test]
    fn non_streaming_no_relay_no_usage_writes_nothing() {
        let action = non_streaming_billing_action(false, 0, 0, 10, 512);
        assert_eq!(action, BillingAction::NoWrite);
    }

    // ── Batch billing decision ──

    #[test]
    fn batch_relay_finalizes_with_aggregate() {
        let action = batch_billing_action(true, 500, 200);
        assert_eq!(action, BillingAction::FinalizeReservation(500, 200));
    }

    #[test]
    fn batch_relay_cancels_when_all_items_failed() {
        // No aggregate usage means no billable work — cancel, don't overcharge.
        let action = batch_billing_action(true, 0, 0);
        assert_eq!(action, BillingAction::CancelReservation);
    }

    #[test]
    fn batch_no_relay_keeps_single_write() {
        let action = batch_billing_action(false, 500, 200);
        assert_eq!(action, BillingAction::SingleWrite(500, 200));
    }

    #[test]
    fn batch_no_relay_no_usage_writes_nothing() {
        let action = batch_billing_action(false, 0, 0);
        assert_eq!(action, BillingAction::NoWrite);
    }
}
