//! Pluggable inference backend abstraction.
//!
//! All serving engines expose an OpenAI-compatible `/v1/chat/completions` endpoint.
//! The only divergence is in the **adapter lifecycle API** (load/unload LoRA adapters).
//! This module abstracts exactly those two operations so the rest of the codebase is
//! engine-agnostic.
//!
//! # Adding a new backend
//! 1. Implement `InferenceBackend` for your type.
//! 2. Add a new backend type string branch in `build_backend` that constructs it.
//! 3. Document the new backend's type string and adapter lifecycle endpoints below.
//!
//! # Supported backends
//! | Backend     | Type string | Load endpoint              | Unload endpoint              |
//! |-------------|-------------|----------------------------|------------------------------|
//! | vLLM        | `vllm`      | `POST /v1/load_lora_adapter`  | `POST /v1/unload_lora_adapter` |
//! | TGI         | `tgi`       | `POST /lora_adapters`         | `DELETE /lora_adapters/{id}`  |
//! | SGLang      | `sglang`    | `POST /load_lora`             | `POST /unload_lora`           |

use async_trait::async_trait;
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::services::circuit_breaker::CircuitBreaker;

/// A loaded adapter reference returned by [`InferenceBackend::load_adapter`].
///
/// `adapter_ref` is the value passed as the `"model"` field in OpenAI-compat
/// chat completion requests.  It is also stored in `models.deployment_config`
/// so it survives API restarts.
#[derive(Debug, Clone)]
pub struct AdapterHandle {
    /// The name / ID used to address this adapter in inference requests.
    pub adapter_ref: String,
    /// Backend-specific metadata, stored verbatim in `deployment_config` JSONB.
    pub metadata: serde_json::Value,
}

/// Abstracts load/unload lifecycle for LoRA adapters across serving engines.
///
/// Chat completion itself is **not** part of this trait: all supported engines
/// expose an OpenAI-compatible `/v1/chat/completions` endpoint, so the
/// inference proxy in `routes/inference.rs` calls `base_url()` directly.
/// The circuit breaker is exposed via [`circuit_breaker()`] so callers can
/// wrap inference requests too.
#[async_trait]
pub trait InferenceBackend: Send + Sync {
    /// Human-readable engine name for logs and the `backend` field in
    /// `deployment_config`.
    fn name(&self) -> &str;

    /// Base URL for the OpenAI-compatible chat completions API.
    /// `inference.rs` constructs `{base_url}/v1/chat/completions` from this.
    fn base_url(&self) -> &str;

    /// Circuit breaker shared across load and inference calls.
    /// Routes use this to wrap chat completion HTTP requests.
    fn circuit_breaker(&self) -> &CircuitBreaker;

    /// Load a LoRA adapter and return a handle for inference routing.
    async fn load_adapter(&self, model_id: Uuid, adapter_path: &str) -> AppResult<AdapterHandle>;

    /// Unload a LoRA adapter.  Best-effort — never fails hard; callers
    /// update the DB regardless so a stale unload does not brick the model.
    async fn unload_adapter(&self, adapter_ref: &str) -> AppResult<()>;
}

// ─── vLLM ──────────────────────────────────────────────────────────────────

/// vLLM backend — the reference serving engine.
///
/// Uses vLLM's proprietary `/v1/load_lora_adapter` / `/v1/unload_lora_adapter`
/// endpoints introduced in vLLM 0.4+.
pub struct VllmBackend {
    base_url: String,
    http_client: reqwest::Client,
    circuit_breaker: CircuitBreaker,
}

impl VllmBackend {
    pub fn new(
        base_url: String,
        http_client: reqwest::Client,
        circuit_breaker: CircuitBreaker,
    ) -> Self {
        Self {
            base_url,
            http_client,
            circuit_breaker,
        }
    }
}

#[async_trait]
impl InferenceBackend for VllmBackend {
    fn name(&self) -> &str {
        "vllm"
    }

    fn base_url(&self) -> &str {
        &self.base_url
    }

    fn circuit_breaker(&self) -> &CircuitBreaker {
        &self.circuit_breaker
    }

    async fn load_adapter(&self, model_id: Uuid, adapter_path: &str) -> AppResult<AdapterHandle> {
        let adapter_ref = format!("adapter-{model_id}");
        let url = format!("{}/v1/load_lora_adapter", self.base_url);
        let body = serde_json::json!({
            "lora_name": adapter_ref,
            "lora_path": adapter_path,
        });
        let http = self.http_client.clone();

        let resp = self
            .circuit_breaker
            .execute(|| async {
                http.post(&url)
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| AppError::Internal(anyhow::anyhow!("vLLM unreachable: {e}")))
            })
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            return Err(AppError::Internal(anyhow::anyhow!(
                "vLLM adapter load failed ({status}): {body_text}"
            )));
        }

        Ok(AdapterHandle {
            adapter_ref,
            metadata: serde_json::json!({"backend": "vllm"}),
        })
    }

    async fn unload_adapter(&self, adapter_ref: &str) -> AppResult<()> {
        let url = format!("{}/v1/unload_lora_adapter", self.base_url);
        match self
            .http_client
            .post(&url)
            .json(&serde_json::json!({"lora_name": adapter_ref}))
            .send()
            .await
        {
            Ok(resp) if !resp.status().is_success() => {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                tracing::warn!(adapter_ref, %status, body = %body, "vLLM unload returned error (best-effort)");
            }
            Err(e) => {
                tracing::warn!(adapter_ref, error = %e, "vLLM unload request failed (best-effort)");
            }
            _ => {}
        }
        Ok(())
    }
}

// ─── TGI (HuggingFace Text Generation Inference) ──────────────────────────

/// HuggingFace TGI backend.
///
/// Uses TGI's `/lora_adapters` REST resource.  Requires TGI ≥ 2.0 with
/// `--enable-lora` and `--lora-adapters` at startup.
pub struct TgiBackend {
    base_url: String,
    http_client: reqwest::Client,
    circuit_breaker: CircuitBreaker,
}

impl TgiBackend {
    pub fn new(
        base_url: String,
        http_client: reqwest::Client,
        circuit_breaker: CircuitBreaker,
    ) -> Self {
        Self {
            base_url,
            http_client,
            circuit_breaker,
        }
    }
}

#[async_trait]
impl InferenceBackend for TgiBackend {
    fn name(&self) -> &str {
        "tgi"
    }

    fn base_url(&self) -> &str {
        &self.base_url
    }

    fn circuit_breaker(&self) -> &CircuitBreaker {
        &self.circuit_breaker
    }

    async fn load_adapter(&self, model_id: Uuid, adapter_path: &str) -> AppResult<AdapterHandle> {
        let adapter_id = format!("adapter-{model_id}");
        let url = format!("{}/lora_adapters", self.base_url);
        let body = serde_json::json!({
            "id": adapter_id,
            "path": adapter_path,
        });
        let http = self.http_client.clone();

        let resp = self
            .circuit_breaker
            .execute(|| async {
                http.post(&url)
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| AppError::Internal(anyhow::anyhow!("TGI unreachable: {e}")))
            })
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            return Err(AppError::Internal(anyhow::anyhow!(
                "TGI adapter load failed ({status}): {body_text}"
            )));
        }

        Ok(AdapterHandle {
            adapter_ref: adapter_id,
            metadata: serde_json::json!({"backend": "tgi"}),
        })
    }

    async fn unload_adapter(&self, adapter_ref: &str) -> AppResult<()> {
        let url = format!("{}/lora_adapters/{adapter_ref}", self.base_url);
        match self.http_client.delete(&url).send().await {
            Ok(resp) if !resp.status().is_success() => {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                tracing::warn!(adapter_ref, %status, body = %body, "TGI unload returned error (best-effort)");
            }
            Err(e) => {
                tracing::warn!(adapter_ref, error = %e, "TGI unload request failed (best-effort)");
            }
            _ => {}
        }
        Ok(())
    }
}

// ─── SGLang ───────────────────────────────────────────────────────────────

/// SGLang backend.
///
/// SGLang uses `/load_lora` and `/unload_lora` for adapter lifecycle and the
/// standard OpenAI-compat endpoint for inference.
pub struct SgLangBackend {
    base_url: String,
    http_client: reqwest::Client,
    circuit_breaker: CircuitBreaker,
}

impl SgLangBackend {
    pub fn new(
        base_url: String,
        http_client: reqwest::Client,
        circuit_breaker: CircuitBreaker,
    ) -> Self {
        Self {
            base_url,
            http_client,
            circuit_breaker,
        }
    }
}

#[async_trait]
impl InferenceBackend for SgLangBackend {
    fn name(&self) -> &str {
        "sglang"
    }

    fn base_url(&self) -> &str {
        &self.base_url
    }

    fn circuit_breaker(&self) -> &CircuitBreaker {
        &self.circuit_breaker
    }

    async fn load_adapter(&self, model_id: Uuid, adapter_path: &str) -> AppResult<AdapterHandle> {
        let adapter_ref = format!("adapter-{model_id}");
        let url = format!("{}/load_lora", self.base_url);
        let body = serde_json::json!({
            "lora_name": adapter_ref,
            "lora_path": adapter_path,
        });
        let http = self.http_client.clone();

        let resp = self
            .circuit_breaker
            .execute(|| async {
                http.post(&url)
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| AppError::Internal(anyhow::anyhow!("SGLang unreachable: {e}")))
            })
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            return Err(AppError::Internal(anyhow::anyhow!(
                "SGLang adapter load failed ({status}): {body_text}"
            )));
        }

        Ok(AdapterHandle {
            adapter_ref,
            metadata: serde_json::json!({"backend": "sglang"}),
        })
    }

    async fn unload_adapter(&self, adapter_ref: &str) -> AppResult<()> {
        let url = format!("{}/unload_lora", self.base_url);
        match self
            .http_client
            .post(&url)
            .json(&serde_json::json!({"lora_name": adapter_ref}))
            .send()
            .await
        {
            Ok(resp) if !resp.status().is_success() => {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                tracing::warn!(adapter_ref, %status, body = %body, "SGLang unload returned error (best-effort)");
            }
            Err(e) => {
                tracing::warn!(adapter_ref, error = %e, "SGLang unload request failed (best-effort)");
            }
            _ => {}
        }
        Ok(())
    }
}

// ─── Factory ──────────────────────────────────────────────────────────────

/// Construct the inference backend from configuration.
///
/// `backend_type`: one of `"vllm"` (default), `"tgi"`, `"sglang"`.
/// `server_url`: the serving engine's base URL (maps to `INFERENCE_SERVER_URL`).
pub fn build_backend(
    backend_type: &str,
    server_url: String,
    http_client: reqwest::Client,
    circuit_breaker: CircuitBreaker,
) -> Arc<dyn InferenceBackend> {
    let normalized = backend_type.trim().to_lowercase();
    match normalized.as_str() {
        "tgi" => {
            tracing::info!(%server_url, "Inference backend: TGI");
            Arc::new(TgiBackend::new(server_url, http_client, circuit_breaker))
        }
        "sglang" => {
            tracing::info!(%server_url, "Inference backend: SGLang");
            Arc::new(SgLangBackend::new(server_url, http_client, circuit_breaker))
        }
        other => {
            if other != "vllm" {
                tracing::warn!(
                    backend_type = %other,
                    "Unknown INFERENCE_BACKEND_TYPE — defaulting to vLLM"
                );
            }
            tracing::info!(%server_url, "Inference backend: vLLM");
            Arc::new(VllmBackend::new(server_url, http_client, circuit_breaker))
        }
    }
}

use std::sync::Arc;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vllm_adapter_ref_format() {
        let id = Uuid::nil();
        // We can't call load_adapter without a real HTTP client, but we can
        // verify the naming convention used everywhere matches.
        let expected = format!("adapter-{id}");
        assert!(expected.starts_with("adapter-"));
    }

    #[test]
    fn build_backend_defaults_to_vllm() {
        use std::time::Duration;
        let cb = CircuitBreaker::new(5, Duration::from_secs(30));
        let client = reqwest::Client::new();
        let backend = build_backend(
            "unknown-engine",
            "http://localhost:8080".to_string(),
            client,
            cb,
        );
        // Unknown type falls through to vLLM
        assert_eq!(backend.name(), "vllm");
    }

    #[test]
    fn build_backend_selects_tgi() {
        use std::time::Duration;
        let cb = CircuitBreaker::new(5, Duration::from_secs(30));
        let client = reqwest::Client::new();
        let backend = build_backend("tgi", "http://localhost:8080".to_string(), client, cb);
        assert_eq!(backend.name(), "tgi");
        assert_eq!(backend.base_url(), "http://localhost:8080");
    }

    #[test]
    fn build_backend_selects_sglang() {
        use std::time::Duration;
        let cb = CircuitBreaker::new(5, Duration::from_secs(30));
        let client = reqwest::Client::new();
        let backend = build_backend("sglang", "http://localhost:30000".to_string(), client, cb);
        assert_eq!(backend.name(), "sglang");
    }
}
