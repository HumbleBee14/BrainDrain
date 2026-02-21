use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use platform_shared::enums::DeploymentStatus;
use serde::{Deserialize, Serialize};

use crate::app_state::AppState;
use crate::auth_api_key::ApiKeyAuth;
use crate::error::{AppError, AppResult};
use crate::repositories::billing_event_repo::BillingEventRepo;
use crate::repositories::model_repo::ModelRepo;

/// Inference routes — OpenAI-compatible API.
/// These are mounted at `/v1/` (not `/api/v1/`) and use API key auth.
pub fn router() -> Router<AppState> {
    Router::new().route("/v1/chat/completions", post(chat_completions))
}

/// OpenAI-compatible chat completion request (subset of fields we support).
#[derive(Debug, Deserialize)]
struct ChatCompletionRequest {
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

#[derive(Debug, Deserialize, Serialize, Clone)]
struct ChatMessage {
    role: String,
    content: String,
}

fn default_temperature() -> f64 {
    0.7
}

fn default_max_tokens() -> i64 {
    512
}

/// POST /v1/chat/completions
///
/// Proxies the request to the vLLM backend, routing to the correct
/// LoRA adapter based on the API key's associated model.
async fn chat_completions(
    State(state): State<AppState>,
    api_key: ApiKeyAuth,
    Json(body): Json<ChatCompletionRequest>,
) -> AppResult<Json<serde_json::Value>> {
    // Verify model is actively deployed
    let model = ModelRepo::get_by_id(state.db(), api_key.tenant_id, api_key.model_id)
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
        )))?;

    // Build the vLLM request — use the adapter name as the "model" field
    let vllm_request = serde_json::json!({
        "model": adapter_name,
        "messages": body.messages,
        "temperature": body.temperature,
        "max_tokens": body.max_tokens,
        "top_p": body.top_p.unwrap_or(1.0),
        "stream": body.stream.unwrap_or(false),
    });

    let vllm_url = &state.config().vllm_api_url;
    let http = reqwest::Client::new();

    let vllm_resp = http
        .post(format!("{vllm_url}/v1/chat/completions"))
        .json(&vllm_request)
        .send()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Cannot reach vLLM service: {e}")))?;

    if !vllm_resp.status().is_success() {
        let status = vllm_resp.status();
        let body = vllm_resp.text().await.unwrap_or_default();
        tracing::error!(status = %status, body = %body, "vLLM inference failed");
        return Err(AppError::Internal(anyhow::anyhow!(
            "vLLM inference failed: {status}"
        )));
    }

    let response: serde_json::Value = vllm_resp
        .json()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to parse vLLM response: {e}")))?;

    // Extract token usage for billing (fire-and-forget)
    let tokens_in = response["usage"]["prompt_tokens"].as_i64().unwrap_or(0);
    let tokens_out = response["usage"]["completion_tokens"].as_i64().unwrap_or(0);

    if tokens_in > 0 || tokens_out > 0 {
        let db = state.db().clone();
        let tenant_id = api_key.tenant_id;
        let model_id = api_key.model_id;
        let key_id = api_key.key_id;
        tokio::spawn(async move {
            if let Err(e) = BillingEventRepo::create(
                &db,
                tenant_id,
                "inference",
                Some(model_id),
                tokens_in,
                tokens_out,
                0,
                estimate_cost(tokens_in, tokens_out),
                serde_json::json!({"api_key_id": key_id.to_string()}),
            )
            .await
            {
                tracing::error!(
                    tenant_id = %tenant_id,
                    model_id = %model_id,
                    error = %e,
                    "Failed to create billing event"
                );
            }
        });
    }

    Ok(Json(response))
}

/// Simple cost estimation based on token counts.
fn estimate_cost(tokens_in: i64, tokens_out: i64) -> f64 {
    // Approximate pricing: $0.15 per 1M input tokens, $0.60 per 1M output tokens
    let input_cost = tokens_in as f64 * 0.15 / 1_000_000.0;
    let output_cost = tokens_out as f64 * 0.60 / 1_000_000.0;
    input_cost + output_cost
}
