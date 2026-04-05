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
use crate::services::token_estimator;

/// Maximum number of items in a single batch request.
const MAX_BATCH_SIZE: usize = 50;

/// Concurrent inference requests per batch.
const BATCH_CONCURRENCY: usize = 5;

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

fn default_temperature() -> f64 {
    0.7
}

fn default_max_tokens() -> i64 {
    512
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

    let adapter_name = model.deployment_config["adapter_ref"]
        .as_str()
        .ok_or(AppError::Internal(anyhow::anyhow!(
            "Model deployment config missing adapter_ref"
        )))?
        .to_string();

    // Validate the model was deployed on the same backend we're currently running.
    let backend = state.inference_backend();
    if let Some(deployed_backend) = model.deployment_config["backend"].as_str()
        && deployed_backend != backend.name()
    {
        return Err(AppError::BadRequest {
            message: format!(
                "Model was deployed on '{}' but current backend is '{}'. Redeploy the model.",
                deployed_backend,
                backend.name()
            ),
        });
    }

    // Cap max_tokens to prevent GPU abuse (configurable via INFERENCE_MAX_TOKENS)
    let max_tokens = body.max_tokens.min(state.config().inference_max_tokens);

    let is_streaming = body.stream.unwrap_or(false);

    // Build the inference request via the backend (handles adapter selection correctly)
    let mut inference_request = backend.build_inference_body(
        &adapter_name,
        &serde_json::json!(body.messages),
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
    let http_client = state.http_client().clone();

    let vllm_resp = backend
        .circuit_breaker()
        .execute(|| async {
            http_client
                .post(&inference_url)
                .json(&inference_request)
                .send()
                .await
                .map_err(|e| {
                    AppError::Internal(anyhow::anyhow!("Cannot reach inference service: {e}"))
                })
        })
        .await?;

    if !vllm_resp.status().is_success() {
        let status = vllm_resp.status();
        let body_text = vllm_resp.text().await.unwrap_or_default();
        tracing::error!(status = %status, body = %body_text, "Inference request failed");
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

        let forwarded_stream =
            vllm_stream.map(move |chunk_result: Result<bytes::Bytes, reqwest::Error>| {
                match chunk_result {
                    Ok(bytes) => {
                        // Scan for usage in the final SSE chunk
                        // OpenAI-compat SSE: data: {"usage":{"prompt_tokens":N,"completion_tokens":N}}
                        let text = String::from_utf8_lossy(&bytes);
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

        // Estimate prompt tokens from request for fallback billing.
        let estimated_prompt_tokens = token_estimator::estimate_tokens_from_messages(
            body.messages.iter().map(|m| m.content.as_str()),
        );

        // Spawn billing task that waits for usage from the stream.
        // NOTE: This is inherently fire-and-forget — the token count is only
        // known after streaming completes. If the process crashes during
        // streaming, this billing event is lost. This is an accepted tradeoff:
        // the alternative (pre-billing max_tokens then crediting back) adds
        // significant complexity. For streaming, billing is best-effort.
        // Non-streaming and batch inference bill durably via the outbox.
        let batcher_state = state.clone();
        let capped_max_tokens = max_tokens;
        tokio::spawn(async move {
            let (tokens_in, tokens_out) = match usage_rx.recv().await {
                Some(usage) => usage,
                None => {
                    // Client disconnected before usage chunk — bill conservatively
                    tracing::warn!(
                        model_id = %model_id,
                        "Client disconnected before usage chunk; billing estimate"
                    );
                    (estimated_prompt_tokens, capped_max_tokens)
                }
            };

            batcher_state
                .record_billing_event(
                    tenant_id,
                    "inference",
                    Some(model_id),
                    tokens_in,
                    tokens_out,
                    0,
                    token_estimator::estimate_inference_cost(tokens_in, tokens_out),
                    serde_json::json!({"api_key_id": key_id.to_string(), "stream": true}),
                )
                .await;
        });

        let body = Body::from_stream(forwarded_stream);

        let resp = Response::builder()
            .header(CONTENT_TYPE, "text/event-stream")
            .header(CACHE_CONTROL, "no-cache")
            .body(body)
            .map_err(|e| {
                AppError::Internal(anyhow::anyhow!("Failed to build SSE response: {e}"))
            })?;
        Ok(resp.into_response())
    } else {
        // Non-streaming: parse JSON response and bill
        let response: serde_json::Value = vllm_resp.json().await.map_err(|e| {
            AppError::Internal(anyhow::anyhow!("Failed to parse inference response: {e}"))
        })?;

        // Extract token usage for billing via batcher (not fire-and-forget spawn)
        let tokens_in = response["usage"]["prompt_tokens"].as_i64().unwrap_or(0);
        let tokens_out = response["usage"]["completion_tokens"].as_i64().unwrap_or(0);

        if tokens_in > 0 || tokens_out > 0 {
            state
                .record_billing_event(
                    api_key.tenant_id,
                    "inference",
                    Some(api_key.model_id),
                    tokens_in,
                    tokens_out,
                    0,
                    token_estimator::estimate_inference_cost(tokens_in, tokens_out),
                    serde_json::json!({"api_key_id": api_key.key_id.to_string()}),
                )
                .await;
        }

        Ok(Json(response).into_response())
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

    let adapter_name = model.deployment_config["adapter_ref"]
        .as_str()
        .ok_or(AppError::Internal(anyhow::anyhow!(
            "Model deployment config missing adapter_ref"
        )))?
        .to_string();

    let batch_backend = state.inference_backend();

    // Validate backend matches what the model was deployed on
    if let Some(deployed_backend) = model.deployment_config["backend"].as_str()
        && deployed_backend != batch_backend.name()
    {
        return Err(AppError::BadRequest {
            message: format!(
                "Model was deployed on '{}' but current backend is '{}'. Redeploy the model.",
                deployed_backend,
                batch_backend.name()
            ),
        });
    }

    let batch_url = batch_backend.chat_completions_url();
    let http_client = state.http_client().clone();
    let max_tokens_limit = state.config().inference_max_tokens;

    // Process batch items concurrently with bounded parallelism
    let results: Vec<BatchResponseItem> = futures::stream::iter(body.requests)
        .map(|item| {
            let adapter = adapter_name.clone();
            let url = batch_url.clone();
            let client = http_client.clone();
            let cb = batch_backend.circuit_breaker();

            async move {
                let max_tokens = item.max_tokens.min(max_tokens_limit);
                let request_body = batch_backend.build_inference_body(
                    &adapter,
                    &serde_json::json!(item.messages),
                    item.temperature,
                    max_tokens,
                    item.top_p.unwrap_or(1.0),
                    false,
                );

                let resp = cb
                    .execute(|| async {
                        client
                            .post(&url)
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

    // Bill aggregated tokens
    if total_prompt > 0 || total_completion > 0 {
        state
            .record_billing_event(
                api_key.tenant_id,
                "inference",
                Some(api_key.model_id),
                total_prompt,
                total_completion,
                0,
                token_estimator::estimate_inference_cost(total_prompt, total_completion),
                serde_json::json!({
                    "api_key_id": api_key.key_id.to_string(),
                    "batch": true,
                    "batch_size": results.len(),
                }),
            )
            .await;
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
