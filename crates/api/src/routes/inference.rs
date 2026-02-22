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
use crate::services::billing_batcher;
use crate::services::token_estimator;

/// Hard cap on max_tokens to prevent GPU abuse.
const MAX_TOKENS_LIMIT: i64 = 8192;

/// Inference routes — OpenAI-compatible API.
/// These are mounted at `/v1/` (not `/api/v1/`) and use API key auth.
pub fn router() -> Router<AppState> {
    Router::new().route("/v1/chat/completions", post(chat_completions))
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
/// Proxies the request to the vLLM backend, routing to the correct
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

    // Get the vLLM adapter name from deployment config
    let adapter_name = model.deployment_config["vllm_adapter_name"]
        .as_str()
        .ok_or(AppError::Internal(anyhow::anyhow!(
            "Model deployment config missing vllm_adapter_name"
        )))?
        .to_string();

    // Cap max_tokens to prevent GPU abuse
    let max_tokens = body.max_tokens.min(MAX_TOKENS_LIMIT);

    let is_streaming = body.stream.unwrap_or(false);

    // Build the vLLM request — use the adapter name as the "model" field
    let mut vllm_request = serde_json::json!({
        "model": adapter_name,
        "messages": body.messages,
        "temperature": body.temperature,
        "max_tokens": max_tokens,
        "top_p": body.top_p.unwrap_or(1.0),
        "stream": is_streaming,
    });

    // For streaming, request usage in the final chunk
    if is_streaming {
        vllm_request["stream_options"] = serde_json::json!({"include_usage": true});
    }

    let vllm_url = state.config().vllm_api_url.clone();
    let http_client = state.http_client().clone();

    // Execute vLLM request through circuit breaker
    let vllm_resp = state
        .vllm_circuit_breaker()
        .execute(|| async {
            http_client
                .post(format!("{vllm_url}/v1/chat/completions"))
                .json(&vllm_request)
                .send()
                .await
                .map_err(|e| AppError::Internal(anyhow::anyhow!("Cannot reach vLLM service: {e}")))
        })
        .await?;

    if !vllm_resp.status().is_success() {
        let status = vllm_resp.status();
        let body_text = vllm_resp.text().await.unwrap_or_default();
        tracing::error!(status = %status, body = %body_text, "vLLM inference failed");
        return Err(AppError::Internal(anyhow::anyhow!(
            "vLLM inference failed: {status}"
        )));
    }

    if is_streaming {
        // SSE streaming: forward vLLM's byte stream and extract usage from final chunk
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
                        // vLLM sends: data: {"usage":{"prompt_tokens":N,"completion_tokens":N}}
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
        // If the client disconnects before the final chunk, usage_rx.recv()
        // returns None and we bill conservatively using max_tokens to prevent
        // free inference on early disconnect.
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
                .billing_batcher()
                .send(billing_batcher::BillingEvent {
                    tenant_id,
                    operation: "inference".to_string(),
                    resource_id: Some(model_id),
                    tokens_in,
                    tokens_out,
                    gpu_seconds: 0,
                    cost_usd: token_estimator::estimate_inference_cost(tokens_in, tokens_out),
                    metadata: serde_json::json!({"api_key_id": key_id.to_string(), "stream": true}),
                });
        });

        let body = Body::from_stream(forwarded_stream);

        Ok(Response::builder()
            .header(CONTENT_TYPE, "text/event-stream")
            .header(CACHE_CONTROL, "no-cache")
            .body(body)
            .unwrap()
            .into_response())
    } else {
        // Non-streaming: parse JSON response and bill
        let response: serde_json::Value = vllm_resp.json().await.map_err(|e| {
            AppError::Internal(anyhow::anyhow!("Failed to parse vLLM response: {e}"))
        })?;

        // Extract token usage for billing via batcher (not fire-and-forget spawn)
        let tokens_in = response["usage"]["prompt_tokens"].as_i64().unwrap_or(0);
        let tokens_out = response["usage"]["completion_tokens"].as_i64().unwrap_or(0);

        if tokens_in > 0 || tokens_out > 0 {
            state.billing_batcher().send(billing_batcher::BillingEvent {
                tenant_id: api_key.tenant_id,
                operation: "inference".to_string(),
                resource_id: Some(api_key.model_id),
                tokens_in,
                tokens_out,
                gpu_seconds: 0,
                cost_usd: token_estimator::estimate_inference_cost(tokens_in, tokens_out),
                metadata: serde_json::json!({"api_key_id": api_key.key_id.to_string()}),
            });
        }

        Ok(Json(response).into_response())
    }
}
