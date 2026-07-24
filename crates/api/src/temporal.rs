use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Error type for workflow orchestration operations.
#[derive(Debug, thiserror::Error)]
pub enum OrchestratorError {
    #[error("Orchestrator HTTP request failed: {0}")]
    Request(#[from] reqwest::Error),

    #[error("Orchestrator returned error: {status} - {body}")]
    Api { status: u16, body: String },

    #[allow(dead_code)]
    #[error("Orchestrator not configured")]
    NotConfigured,
}

/// Status of a workflow execution.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowStatus {
    pub workflow_id: String,
    pub run_id: String,
    pub status: String,
}

/// Response from starting a workflow.
#[derive(Debug, Serialize)]
pub struct StartWorkflowResponse {
    pub workflow_id: String,
    pub run_id: String,
}

// Convenience type alias for boxed futures (used by the trait methods).
type BoxFuture<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

/// Optional trace context propagated from the API request to workflow execution.
#[derive(Debug, Clone, Default)]
pub struct TraceContext {
    pub request_id: Option<String>,
}

impl TraceContext {
    /// Extract trace context from an HTTP request's headers.
    /// Reads the `x-request-id` header set by the `SetRequestIdLayer`.
    pub fn from_headers(headers: &axum::http::HeaderMap) -> Self {
        let request_id = headers
            .get("x-request-id")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        Self { request_id }
    }
}

/// Trait for workflow orchestration — decouples services from Temporal.
///
/// Implement this for any workflow engine (Temporal, Airflow, Prefect, etc.).
/// Services depend on `&dyn WorkflowOrchestrator`, not concrete implementations.
#[allow(clippy::too_many_arguments, dead_code)]
pub trait WorkflowOrchestrator: Send + Sync {
    fn start_ingest(
        &self,
        tenant_id: Uuid,
        project_id: Uuid,
        document_ids: Vec<Uuid>,
        trace_ctx: TraceContext,
    ) -> BoxFuture<'_, Result<StartWorkflowResponse, OrchestratorError>>;

    fn start_refine(
        &self,
        tenant_id: Uuid,
        project_id: Uuid,
        document_ids: Vec<Uuid>,
        task_type: &str,
        config: serde_json::Value,
        trace_ctx: TraceContext,
    ) -> BoxFuture<'_, Result<StartWorkflowResponse, OrchestratorError>>;

    fn start_generate_facets(
        &self,
        tenant_id: Uuid,
        project_id: Uuid,
        data_guide_id: Uuid,
        task_type: &str,
        guidance: &str,
        num_facets: u32,
        trace_ctx: TraceContext,
    ) -> BoxFuture<'_, Result<StartWorkflowResponse, OrchestratorError>>;

    fn start_generate_preview(
        &self,
        tenant_id: Uuid,
        project_id: Uuid,
        data_guide_id: Uuid,
        task_type: &str,
        guidance: &str,
        facets: serde_json::Value,
        num_samples: u32,
        trace_ctx: TraceContext,
    ) -> BoxFuture<'_, Result<StartWorkflowResponse, OrchestratorError>>;

    fn start_refine_guidance(
        &self,
        tenant_id: Uuid,
        project_id: Uuid,
        data_guide_id: Uuid,
        task_type: &str,
        current_guidance: &str,
        rated: serde_json::Value,
        trace_ctx: TraceContext,
    ) -> BoxFuture<'_, Result<StartWorkflowResponse, OrchestratorError>>;

    #[allow(clippy::too_many_arguments)]
    fn start_generate_dataset(
        &self,
        tenant_id: Uuid,
        project_id: Uuid,
        data_guide_id: Uuid,
        task_type: &str,
        guidance: &str,
        system_prompt: &str,
        facets: serde_json::Value,
        document_ids: Vec<Uuid>,
        trace_ctx: TraceContext,
    ) -> BoxFuture<'_, Result<StartWorkflowResponse, OrchestratorError>>;

    fn start_train(
        &self,
        tenant_id: Uuid,
        training_job_id: Uuid,
        dataset_path: &str,
        base_model: &str,
        method: &str,
        mode: &str,
        hyperparams: serde_json::Value,
        gpu_class: Option<&str>,
        trace_ctx: TraceContext,
    ) -> BoxFuture<'_, Result<StartWorkflowResponse, OrchestratorError>>;

    fn start_evaluate(
        &self,
        tenant_id: Uuid,
        model_id: Uuid,
        evaluation_id: Uuid,
        adapter_path: &str,
        base_model: &str,
        dataset_path: &str,
        judge_model: Option<&str>,
        judge_api_base: Option<&str>,
        trace_ctx: TraceContext,
    ) -> BoxFuture<'_, Result<StartWorkflowResponse, OrchestratorError>>;

    fn start_export(
        &self,
        tenant_id: Uuid,
        model_id: Uuid,
        export_id: Uuid,
        adapter_path: &str,
        base_model: &str,
        quant_type: &str,
        trace_ctx: TraceContext,
    ) -> BoxFuture<'_, Result<StartWorkflowResponse, OrchestratorError>>;

    fn start_full_pipeline(
        &self,
        tenant_id: Uuid,
        project_id: Uuid,
        document_ids: Vec<Uuid>,
        task_type: &str,
        base_model: &str,
        training_config: serde_json::Value,
        trace_ctx: TraceContext,
    ) -> BoxFuture<'_, Result<StartWorkflowResponse, OrchestratorError>>;

    fn get_workflow_status(
        &self,
        workflow_id: &str,
    ) -> BoxFuture<'_, Result<WorkflowStatus, OrchestratorError>>;

    /// Terminate a running workflow. Terminate (not cancel) is immediate — the
    /// worker activity is killed without a chance to run cleanup, so the caller
    /// is responsible for the terminal DB state and billing. A workflow that is
    /// already gone (404) is treated as success.
    fn terminate_workflow(
        &self,
        workflow_id: &str,
        reason: &str,
    ) -> BoxFuture<'_, Result<(), OrchestratorError>>;

    /// Request graceful cancellation of a running workflow. Unlike terminate,
    /// cancellation is *delivered* to the executing activity (on its next
    /// heartbeat), so a long-running activity can stop external work — e.g.
    /// cancel an in-flight remote GPU call so it stops billing immediately —
    /// before the workflow ends. A workflow that is already gone (404) is
    /// treated as success.
    fn cancel_workflow(
        &self,
        workflow_id: &str,
        reason: &str,
    ) -> BoxFuture<'_, Result<(), OrchestratorError>>;
}

/// Temporal implementation of the WorkflowOrchestrator trait.
///
/// Temporal Server (v1.24+) exposes an HTTP API alongside gRPC.
/// This client uses `reqwest` to call it — no gRPC dependency needed.
#[derive(Clone)]
pub struct TemporalClient {
    http: reqwest::Client,
    base_url: String,
    namespace: String,
    task_queue: String,
}

impl TemporalClient {
    /// Create a new Temporal client.
    ///
    /// `host` is the Temporal server address (e.g., "localhost:7233").
    /// The HTTP API is assumed to be on the same host.
    pub fn new(host: &str, namespace: &str, task_queue: &str) -> Self {
        let base_url = if host.starts_with("http") {
            host.to_string()
        } else {
            format!("http://{host}")
        };

        Self {
            http: reqwest::Client::new(),
            base_url,
            namespace: namespace.to_string(),
            task_queue: task_queue.to_string(),
        }
    }

    /// Start a Temporal workflow via the HTTP API on a specific task queue.
    ///
    /// When `trace_ctx` has a `request_id`, it is injected as a Temporal header
    /// so Python workers can extract it for structured logging.
    async fn start_workflow_on_queue(
        &self,
        workflow_type: &str,
        workflow_id: &str,
        args: serde_json::Value,
        task_queue: Option<&str>,
        trace_ctx: &TraceContext,
    ) -> Result<StartWorkflowResponse, OrchestratorError> {
        // Workflow id must be in the path — an id-less /workflows POST is UNIMPLEMENTED.
        let url = format!(
            "{}/api/v1/namespaces/{}/workflows/{}",
            self.base_url, self.namespace, workflow_id
        );

        let queue = task_queue.unwrap_or(&self.task_queue);

        // Build Temporal headers for trace propagation
        let header = if let Some(ref rid) = trace_ctx.request_id {
            serde_json::json!({
                "fields": {
                    "x-request-id": {
                        "metadata": { "encoding": base64_encode("json/plain") },
                        "data": base64_encode(&format!("\"{rid}\"")),
                    }
                }
            })
        } else {
            serde_json::json!({ "fields": {} })
        };

        // Temporal HTTP API payload format
        let payload = serde_json::json!({
            "workflowId": workflow_id,
            "workflowType": { "name": workflow_type },
            "taskQueue": { "name": queue },
            "header": header,
            "input": {
                "payloads": args.as_array().unwrap_or(&vec![]).iter().map(|arg| {
                    serde_json::json!({
                        "metadata": {
                            "encoding": base64_encode("json/plain"),
                        },
                        "data": base64_encode(&arg.to_string()),
                    })
                }).collect::<Vec<_>>(),
            },
        });

        let resp = self.http.post(&url).json(&payload).send().await?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(OrchestratorError::Api {
                status: status.as_u16(),
                body,
            });
        }

        let body: serde_json::Value = resp.json().await?;
        let run_id = body["runId"].as_str().unwrap_or("").to_string();

        Ok(StartWorkflowResponse {
            workflow_id: workflow_id.to_string(),
            run_id,
        })
    }
}

/// Build a workflow id of the form `{prefix}-{id}-{timestamp}`.
fn build_workflow_id(prefix: &str, id: Uuid, timestamp: i64) -> String {
    format!("{prefix}-{id}-{timestamp}")
}

fn build_facets_workflow_id(data_guide_id: Uuid, timestamp: i64) -> String {
    build_workflow_id("facets", data_guide_id, timestamp)
}

fn build_preview_workflow_id(data_guide_id: Uuid, timestamp: i64) -> String {
    build_workflow_id("preview", data_guide_id, timestamp)
}

fn build_refine_guidance_workflow_id(data_guide_id: Uuid, timestamp: i64) -> String {
    build_workflow_id("refine-guidance", data_guide_id, timestamp)
}

fn build_generate_dataset_workflow_id(data_guide_id: Uuid, timestamp: i64) -> String {
    build_workflow_id("generate-dataset", data_guide_id, timestamp)
}

impl WorkflowOrchestrator for TemporalClient {
    fn start_ingest(
        &self,
        tenant_id: Uuid,
        project_id: Uuid,
        document_ids: Vec<Uuid>,
        trace_ctx: TraceContext,
    ) -> BoxFuture<'_, Result<StartWorkflowResponse, OrchestratorError>> {
        Box::pin(async move {
            let workflow_id = format!("ingest-{project_id}-{}", chrono::Utc::now().timestamp());
            let doc_ids: Vec<String> = document_ids.iter().map(|id| id.to_string()).collect();

            self.start_workflow_on_queue(
                "IngestWorkflow",
                &workflow_id,
                serde_json::json!([tenant_id.to_string(), project_id.to_string(), doc_ids]),
                None,
                &trace_ctx,
            )
            .await
        })
    }

    fn start_refine(
        &self,
        tenant_id: Uuid,
        project_id: Uuid,
        document_ids: Vec<Uuid>,
        task_type: &str,
        config: serde_json::Value,
        trace_ctx: TraceContext,
    ) -> BoxFuture<'_, Result<StartWorkflowResponse, OrchestratorError>> {
        let task_type = task_type.to_string();
        Box::pin(async move {
            let workflow_id = format!("refine-{project_id}-{}", chrono::Utc::now().timestamp());
            let doc_ids: Vec<String> = document_ids.iter().map(|id| id.to_string()).collect();

            self.start_workflow_on_queue(
                "RefineWorkflow",
                &workflow_id,
                serde_json::json!([
                    tenant_id.to_string(),
                    project_id.to_string(),
                    doc_ids,
                    task_type,
                    config,
                ]),
                None,
                &trace_ctx,
            )
            .await
        })
    }

    fn start_generate_facets(
        &self,
        tenant_id: Uuid,
        project_id: Uuid,
        data_guide_id: Uuid,
        task_type: &str,
        guidance: &str,
        num_facets: u32,
        trace_ctx: TraceContext,
    ) -> BoxFuture<'_, Result<StartWorkflowResponse, OrchestratorError>> {
        let task_type = task_type.to_string();
        let guidance = guidance.to_string();
        Box::pin(async move {
            let workflow_id =
                build_facets_workflow_id(data_guide_id, chrono::Utc::now().timestamp());

            self.start_workflow_on_queue(
                "GenerateFacetsWorkflow",
                &workflow_id,
                serde_json::json!([
                    tenant_id.to_string(),
                    project_id.to_string(),
                    data_guide_id.to_string(),
                    task_type,
                    guidance,
                    num_facets,
                ]),
                None,
                &trace_ctx,
            )
            .await
        })
    }

    fn start_generate_preview(
        &self,
        tenant_id: Uuid,
        project_id: Uuid,
        data_guide_id: Uuid,
        task_type: &str,
        guidance: &str,
        facets: serde_json::Value,
        num_samples: u32,
        trace_ctx: TraceContext,
    ) -> BoxFuture<'_, Result<StartWorkflowResponse, OrchestratorError>> {
        let task_type = task_type.to_string();
        let guidance = guidance.to_string();
        Box::pin(async move {
            let workflow_id =
                build_preview_workflow_id(data_guide_id, chrono::Utc::now().timestamp());

            self.start_workflow_on_queue(
                "GeneratePreviewWorkflow",
                &workflow_id,
                serde_json::json!([
                    tenant_id.to_string(),
                    project_id.to_string(),
                    data_guide_id.to_string(),
                    task_type,
                    guidance,
                    facets,
                    num_samples,
                ]),
                None,
                &trace_ctx,
            )
            .await
        })
    }

    fn start_refine_guidance(
        &self,
        tenant_id: Uuid,
        project_id: Uuid,
        data_guide_id: Uuid,
        task_type: &str,
        current_guidance: &str,
        rated: serde_json::Value,
        trace_ctx: TraceContext,
    ) -> BoxFuture<'_, Result<StartWorkflowResponse, OrchestratorError>> {
        let task_type = task_type.to_string();
        let current_guidance = current_guidance.to_string();
        Box::pin(async move {
            let workflow_id =
                build_refine_guidance_workflow_id(data_guide_id, chrono::Utc::now().timestamp());

            self.start_workflow_on_queue(
                "RefineGuidanceWorkflow",
                &workflow_id,
                serde_json::json!([
                    tenant_id.to_string(),
                    project_id.to_string(),
                    data_guide_id.to_string(),
                    task_type,
                    current_guidance,
                    rated,
                ]),
                None,
                &trace_ctx,
            )
            .await
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn start_generate_dataset(
        &self,
        tenant_id: Uuid,
        project_id: Uuid,
        data_guide_id: Uuid,
        task_type: &str,
        guidance: &str,
        system_prompt: &str,
        facets: serde_json::Value,
        document_ids: Vec<Uuid>,
        trace_ctx: TraceContext,
    ) -> BoxFuture<'_, Result<StartWorkflowResponse, OrchestratorError>> {
        let task_type = task_type.to_string();
        let guidance = guidance.to_string();
        let system_prompt = system_prompt.to_string();
        let document_ids: Vec<String> = document_ids.iter().map(|id| id.to_string()).collect();
        Box::pin(async move {
            let workflow_id =
                build_generate_dataset_workflow_id(data_guide_id, chrono::Utc::now().timestamp());

            // Positional args — order MUST match GenerateDatasetWorkflow.run.
            // system_prompt is last so pre-existing in-flight workflows using
            // the Python default stay compatible.
            self.start_workflow_on_queue(
                "GenerateDatasetWorkflow",
                &workflow_id,
                serde_json::json!([
                    tenant_id.to_string(),
                    project_id.to_string(),
                    data_guide_id.to_string(),
                    task_type,
                    guidance,
                    facets,
                    document_ids,
                    system_prompt,
                ]),
                None,
                &trace_ctx,
            )
            .await
        })
    }

    fn start_train(
        &self,
        tenant_id: Uuid,
        training_job_id: Uuid,
        dataset_path: &str,
        base_model: &str,
        method: &str,
        mode: &str,
        hyperparams: serde_json::Value,
        gpu_class: Option<&str>,
        trace_ctx: TraceContext,
    ) -> BoxFuture<'_, Result<StartWorkflowResponse, OrchestratorError>> {
        let dataset_path = dataset_path.to_string();
        let base_model = base_model.to_string();
        let method = method.to_string();
        let mode = mode.to_string();
        let gpu_class = gpu_class.map(|s| s.to_string());
        Box::pin(async move {
            let workflow_id = format!("train-{training_job_id}-{}", chrono::Utc::now().timestamp());

            self.start_workflow_on_queue(
                "TrainWorkflow",
                &workflow_id,
                serde_json::json!([
                    tenant_id.to_string(),
                    training_job_id.to_string(),
                    dataset_path,
                    base_model,
                    method,
                    mode,
                    hyperparams,
                    gpu_class,
                ]),
                None, // default queue — GPU activities pin their own
                &trace_ctx,
            )
            .await
        })
    }

    fn start_evaluate(
        &self,
        tenant_id: Uuid,
        model_id: Uuid,
        evaluation_id: Uuid,
        adapter_path: &str,
        base_model: &str,
        dataset_path: &str,
        judge_model: Option<&str>,
        judge_api_base: Option<&str>,
        trace_ctx: TraceContext,
    ) -> BoxFuture<'_, Result<StartWorkflowResponse, OrchestratorError>> {
        let adapter_path = adapter_path.to_string();
        let base_model = base_model.to_string();
        let dataset_path = dataset_path.to_string();
        let judge_model = judge_model.map(|s| s.to_string());
        let judge_api_base = judge_api_base.map(|s| s.to_string());
        Box::pin(async move {
            let workflow_id = format!(
                "evaluate-{evaluation_id}-{}",
                chrono::Utc::now().timestamp()
            );

            self.start_workflow_on_queue(
                "EvaluateWorkflow",
                &workflow_id,
                serde_json::json!([
                    tenant_id.to_string(),
                    model_id.to_string(),
                    evaluation_id.to_string(),
                    adapter_path,
                    base_model,
                    dataset_path,
                    judge_model.as_deref().unwrap_or(""),
                    judge_api_base.as_deref().unwrap_or(""),
                ]),
                None, // default queue — GPU activities pin their own
                &trace_ctx,
            )
            .await
        })
    }

    fn start_export(
        &self,
        tenant_id: Uuid,
        model_id: Uuid,
        export_id: Uuid,
        adapter_path: &str,
        base_model: &str,
        quant_type: &str,
        trace_ctx: TraceContext,
    ) -> BoxFuture<'_, Result<StartWorkflowResponse, OrchestratorError>> {
        let adapter_path = adapter_path.to_string();
        let base_model = base_model.to_string();
        let quant_type = quant_type.to_string();
        Box::pin(async move {
            let workflow_id = format!("export-{export_id}-{}", chrono::Utc::now().timestamp());

            self.start_workflow_on_queue(
                "ExportWorkflow",
                &workflow_id,
                serde_json::json!([
                    tenant_id.to_string(),
                    model_id.to_string(),
                    export_id.to_string(),
                    adapter_path,
                    base_model,
                    quant_type,
                ]),
                None, // default queue — GPU activities pin their own
                &trace_ctx,
            )
            .await
        })
    }

    fn start_full_pipeline(
        &self,
        tenant_id: Uuid,
        project_id: Uuid,
        document_ids: Vec<Uuid>,
        task_type: &str,
        base_model: &str,
        training_config: serde_json::Value,
        trace_ctx: TraceContext,
    ) -> BoxFuture<'_, Result<StartWorkflowResponse, OrchestratorError>> {
        let task_type = task_type.to_string();
        let base_model = base_model.to_string();
        Box::pin(async move {
            let workflow_id = format!(
                "full-pipeline-{project_id}-{}",
                chrono::Utc::now().timestamp()
            );
            let doc_ids: Vec<String> = document_ids.iter().map(|id| id.to_string()).collect();

            self.start_workflow_on_queue(
                "FullPipelineWorkflow",
                &workflow_id,
                serde_json::json!([
                    tenant_id.to_string(),
                    project_id.to_string(),
                    doc_ids,
                    task_type,
                    base_model,
                    training_config,
                ]),
                None,
                &trace_ctx,
            )
            .await
        })
    }

    fn get_workflow_status(
        &self,
        workflow_id: &str,
    ) -> BoxFuture<'_, Result<WorkflowStatus, OrchestratorError>> {
        let workflow_id = workflow_id.to_string();
        Box::pin(async move {
            let url = format!(
                "{}/api/v1/namespaces/{}/workflows/{}",
                self.base_url, self.namespace, workflow_id
            );

            let resp = self.http.get(&url).send().await?;

            let status = resp.status();
            if !status.is_success() {
                let body = resp.text().await.unwrap_or_default();
                return Err(OrchestratorError::Api {
                    status: status.as_u16(),
                    body,
                });
            }

            let body: serde_json::Value = resp.json().await?;
            let run_id = body["workflowExecutionInfo"]["execution"]["runId"]
                .as_str()
                .unwrap_or("")
                .to_string();
            let wf_status = body["workflowExecutionInfo"]["status"]
                .as_str()
                .unwrap_or("UNKNOWN")
                .to_string();

            Ok(WorkflowStatus {
                workflow_id,
                run_id,
                status: wf_status,
            })
        })
    }

    fn terminate_workflow(
        &self,
        workflow_id: &str,
        reason: &str,
    ) -> BoxFuture<'_, Result<(), OrchestratorError>> {
        let workflow_id = workflow_id.to_string();
        let reason = reason.to_string();
        Box::pin(async move {
            let url = format!(
                "{}/api/v1/namespaces/{}/workflows/{}/terminate",
                self.base_url, self.namespace, workflow_id
            );

            let resp = self
                .http
                .post(&url)
                .json(&serde_json::json!({ "reason": reason }))
                .send()
                .await?;

            let status = resp.status();
            // Already gone → nothing to terminate; treat as success.
            if status == reqwest::StatusCode::NOT_FOUND {
                return Ok(());
            }
            if !status.is_success() {
                let body = resp.text().await.unwrap_or_default();
                return Err(OrchestratorError::Api {
                    status: status.as_u16(),
                    body,
                });
            }

            Ok(())
        })
    }

    fn cancel_workflow(
        &self,
        workflow_id: &str,
        reason: &str,
    ) -> BoxFuture<'_, Result<(), OrchestratorError>> {
        let workflow_id = workflow_id.to_string();
        let reason = reason.to_string();
        Box::pin(async move {
            let url = format!(
                "{}/api/v1/namespaces/{}/workflows/{}/cancel",
                self.base_url, self.namespace, workflow_id
            );

            let resp = self
                .http
                .post(&url)
                .json(&serde_json::json!({ "reason": reason }))
                .send()
                .await?;

            let status = resp.status();
            // Already gone → nothing to cancel; treat as success.
            if status == reqwest::StatusCode::NOT_FOUND {
                return Ok(());
            }
            if !status.is_success() {
                let body = resp.text().await.unwrap_or_default();
                return Err(OrchestratorError::Api {
                    status: status.as_u16(),
                    body,
                });
            }

            Ok(())
        })
    }
}

/// Base64 encode a string (Temporal HTTP API requires base64-encoded payloads).
fn base64_encode(input: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(input.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facets_workflow_id_format() {
        let id = build_facets_workflow_id(uuid::Uuid::nil(), 123);
        assert!(id.starts_with("facets-"));
    }
}
