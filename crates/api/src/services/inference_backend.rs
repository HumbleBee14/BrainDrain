//! Pluggable inference backend abstraction.
//!
//! All serving engines expose an OpenAI-compatible `/v1/chat/completions` endpoint.
//! The only divergence is in the **adapter lifecycle API** (load/unload LoRA adapters)
//! and how the adapter is selected at inference time.
//!
//! # Adding a new backend
//! 1. Implement `InferenceBackend` for your type.
//! 2. Add a new backend type string branch in `build_backend` that constructs it.
//! 3. Document the new backend's type string and adapter lifecycle endpoints below.
//!
//! # Supported backends
//! | Backend | Type string | Dynamic LoRA? | Adapter selection |
//! |---------|-------------|---------------|-------------------|
//! | vLLM    | `vllm`      | Yes (S3 resolver, lazy) | `model` field = S3 key prefix |
//! | TGI     | `tgi`       | No (startup)  | `parameters.adapter_id` field |
//! | SGLang  | `sglang`    | Yes (REST, local path)  | `model` field in request body |
//!
//! vLLM fetches adapters from object storage via the `s3_lora_resolver` plugin
//! (see `infra/serving/`); the control plane never needs the adapter on its own
//! or the GPU box's filesystem. SGLang's dynamic `/load_lora` still expects a
//! path reachable by the SGLang process.

use std::sync::Arc;

use async_trait::async_trait;
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::services::circuit_breaker::CircuitBreaker;

/// A loaded adapter reference returned by [`InferenceBackend::load_adapter`].
#[derive(Debug, Clone)]
pub struct AdapterHandle {
    /// The name / ID used to address this adapter in inference requests.
    /// How this is sent depends on the backend (see `build_inference_request`).
    pub adapter_ref: String,
    /// Backend-specific metadata, stored verbatim in `deployment_config` JSONB.
    pub metadata: serde_json::Value,
}

/// Abstracts adapter lifecycle and inference request building across serving engines.
///
/// Each backend knows:
/// - How to load/unload adapters (or whether it's even supported dynamically)
/// - How to inject the adapter reference into an OpenAI-compat request body
/// - The base URL and circuit breaker for the engine
#[async_trait]
pub trait InferenceBackend: Send + Sync {
    /// Human-readable engine name for logs and the `backend` field in
    /// `deployment_config`.
    fn name(&self) -> &str;

    /// Base URL for the serving engine.
    fn base_url(&self) -> &str;

    /// Circuit breaker shared across load and inference calls.
    fn circuit_breaker(&self) -> &CircuitBreaker;

    /// Load a LoRA adapter and return a handle for inference routing.
    async fn load_adapter(&self, model_id: Uuid, adapter_path: &str) -> AppResult<AdapterHandle>;

    /// Unload a LoRA adapter. Best-effort — never fails hard.
    async fn unload_adapter(&self, adapter_ref: &str) -> AppResult<()>;

    /// Build the inference request body with the adapter reference injected
    /// in the correct backend-specific way.
    ///
    /// vLLM/SGLang: adapter goes in the `model` field.
    /// TGI: adapter goes in `parameters.adapter_id`.
    fn build_inference_body(
        &self,
        adapter_ref: &str,
        messages: &serde_json::Value,
        temperature: f64,
        max_tokens: i64,
        top_p: f64,
        stream: bool,
    ) -> serde_json::Value;

    /// The chat completions URL for this backend.
    fn chat_completions_url(&self) -> String {
        format!("{}/v1/chat/completions", self.base_url())
    }
}

// ─── vLLM ──────────────────────────────────────────────────────────────────

/// vLLM backend — the reference serving engine.
///
/// Adapters live in object storage (S3/MinIO/R2) under their `adapter_path`
/// key prefix. Our vLLM image (`infra/serving/`) runs the `s3_lora_resolver`
/// plugin with `VLLM_ALLOW_RUNTIME_LORA_UPDATING=true`, so vLLM fetches an
/// adapter from the bucket the first time its name appears in the `model`
/// field of a request. The adapter is therefore addressed by its S3 key
/// prefix — no shared filesystem between the control plane and the GPU box.
///
/// - Load: a **warmup** inference (`model = <adapter_ref>`) forces the
///   resolver to fetch + load the adapter now, so deploy fails loudly if it
///   can't be loaded instead of failing on the first user request.
/// - Unload: `POST /v1/unload_lora_adapter` (best-effort).
/// - Inference: adapter selected via `model` field in request body.
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

    async fn load_adapter(&self, _model_id: Uuid, adapter_path: &str) -> AppResult<AdapterHandle> {
        // The adapter is addressed by its S3 key prefix; the resolver plugin
        // fetches it from the bucket on first use. A one-token warmup request
        // forces that fetch + load now so deploy surfaces resolve/load errors
        // eagerly rather than on the first real inference call.
        let adapter_ref = adapter_path.trim_end_matches('/').to_string();
        let url = self.chat_completions_url();
        let warmup = self.build_inference_body(
            &adapter_ref,
            &serde_json::json!([{"role": "user", "content": "ping"}]),
            0.0,
            1,
            1.0,
            false,
        );
        let http = self.http_client.clone();

        let resp = self
            .circuit_breaker
            .execute(|| async {
                http.post(&url)
                    .json(&warmup)
                    .send()
                    .await
                    .map_err(|e| AppError::Internal(anyhow::anyhow!("vLLM unreachable: {e}")))
            })
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            return Err(AppError::Internal(anyhow::anyhow!(
                "vLLM adapter warmup/load failed ({status}): {body_text}"
            )));
        }

        Ok(AdapterHandle {
            adapter_ref,
            metadata: serde_json::json!({"backend": "vllm", "loaded_via": "s3_resolver_warmup"}),
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

    fn build_inference_body(
        &self,
        adapter_ref: &str,
        messages: &serde_json::Value,
        temperature: f64,
        max_tokens: i64,
        top_p: f64,
        stream: bool,
    ) -> serde_json::Value {
        serde_json::json!({
            "model": adapter_ref,
            "messages": messages,
            "temperature": temperature,
            "max_tokens": max_tokens,
            "top_p": top_p,
            "stream": stream,
        })
    }
}

// ─── TGI (HuggingFace Text Generation Inference) ──────────────────────────

/// HuggingFace TGI backend.
///
/// **TGI does not support dynamic LoRA loading via REST.** Adapters must be
/// pre-loaded at startup via the `LORA_ADAPTERS` env var or `--lora-adapters`
/// CLI flag. At inference time, the adapter is selected via the
/// `parameters.adapter_id` field (not the `model` field).
///
/// `load_adapter` registers the adapter ref in deployment config (no-op HTTP)
/// so the platform knows which adapter ID to send at inference time.
/// `unload_adapter` is a no-op — TGI manages adapter lifecycle at the process level.
///
/// The operator must ensure the adapter is loaded in TGI before deploying via
/// the platform. The deploy will succeed (config-only) but inference will fail
/// if TGI doesn't know the adapter.
pub struct TgiBackend {
    base_url: String,
    #[allow(dead_code)] // kept for future dynamic TGI LoRA support
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

    /// TGI doesn't support dynamic loading — this registers the adapter ref
    /// in deployment config only. The operator must pre-load the adapter in TGI.
    async fn load_adapter(&self, model_id: Uuid, adapter_path: &str) -> AppResult<AdapterHandle> {
        let adapter_id = format!("adapter-{model_id}");
        tracing::info!(
            adapter_id,
            adapter_path,
            "TGI backend: registering adapter (TGI requires pre-loading at startup)"
        );

        Ok(AdapterHandle {
            adapter_ref: adapter_id,
            metadata: serde_json::json!({
                "backend": "tgi",
                "note": "TGI requires adapters pre-loaded at startup via LORA_ADAPTERS env var",
                "adapter_path": adapter_path,
            }),
        })
    }

    /// TGI manages adapter lifecycle at the process level — unload is a no-op.
    async fn unload_adapter(&self, adapter_ref: &str) -> AppResult<()> {
        tracing::info!(
            adapter_ref,
            "TGI backend: unload is a no-op (adapters managed at TGI startup)"
        );
        Ok(())
    }

    /// TGI uses `parameters.adapter_id` instead of the `model` field.
    fn build_inference_body(
        &self,
        adapter_ref: &str,
        messages: &serde_json::Value,
        temperature: f64,
        max_tokens: i64,
        top_p: f64,
        stream: bool,
    ) -> serde_json::Value {
        serde_json::json!({
            "model": "tgi",
            "messages": messages,
            "temperature": temperature,
            "max_tokens": max_tokens,
            "top_p": top_p,
            "stream": stream,
            "parameters": {
                "adapter_id": adapter_ref,
            },
        })
    }
}

// ─── SGLang ───────────────────────────────────────────────────────────────

/// SGLang backend.
///
/// Supports dynamic LoRA loading via REST:
/// - Load: `POST /load_lora`
/// - Unload: `POST /unload_lora`
/// - Inference: adapter selected via `model` field (same as vLLM)
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

    fn build_inference_body(
        &self,
        adapter_ref: &str,
        messages: &serde_json::Value,
        temperature: f64,
        max_tokens: i64,
        top_p: f64,
        stream: bool,
    ) -> serde_json::Value {
        serde_json::json!({
            "model": adapter_ref,
            "messages": messages,
            "temperature": temperature,
            "max_tokens": max_tokens,
            "top_p": top_p,
            "stream": stream,
        })
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

/// Construct a backend for a specific registered inference instance.
pub fn build_backend_for_instance(
    backend_type: &str,
    server_url: &str,
    http_client: reqwest::Client,
    circuit_breaker: CircuitBreaker,
) -> Arc<dyn InferenceBackend> {
    build_backend(
        backend_type,
        server_url.to_string(),
        http_client,
        circuit_breaker,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vllm_adapter_ref_is_s3_key_prefix() {
        // The adapter is addressed by its S3 key prefix (trailing slash
        // trimmed) so the resolver can locate it in the bucket.
        let adapter_path = "adapters/tenant-1/model-9/";
        let adapter_ref = adapter_path.trim_end_matches('/').to_string();
        assert_eq!(adapter_ref, "adapters/tenant-1/model-9");
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

    #[test]
    fn build_backend_normalizes_input() {
        use std::time::Duration;
        let cb = CircuitBreaker::new(5, Duration::from_secs(30));
        let client = reqwest::Client::new();
        let backend = build_backend("  TGI  ", "http://localhost:8080".to_string(), client, cb);
        assert_eq!(backend.name(), "tgi");
    }

    #[test]
    fn vllm_inference_body_uses_model_field() {
        use std::time::Duration;
        let cb = CircuitBreaker::new(5, Duration::from_secs(30));
        let backend = VllmBackend::new(
            "http://localhost:8080".to_string(),
            reqwest::Client::new(),
            cb,
        );
        let body = backend.build_inference_body(
            "adapter-123",
            &serde_json::json!([]),
            0.7,
            512,
            1.0,
            false,
        );
        assert_eq!(body["model"], "adapter-123");
        assert!(body.get("parameters").is_none());
    }

    #[test]
    fn tgi_inference_body_uses_adapter_id() {
        use std::time::Duration;
        let cb = CircuitBreaker::new(5, Duration::from_secs(30));
        let backend = TgiBackend::new(
            "http://localhost:8080".to_string(),
            reqwest::Client::new(),
            cb,
        );
        let body = backend.build_inference_body(
            "adapter-123",
            &serde_json::json!([]),
            0.7,
            512,
            1.0,
            false,
        );
        assert_eq!(body["parameters"]["adapter_id"], "adapter-123");
        assert_eq!(body["model"], "tgi");
    }
}
