const API_URL = process.env.NEXT_PUBLIC_API_URL || "http://localhost:8000";

interface ApiError {
  error: {
    code: string;
    message: string;
  };
}

class ApiClientError extends Error {
  code: string;
  status: number;

  constructor(status: number, body: ApiError) {
    super(body.error.message);
    this.code = body.error.code;
    this.status = status;
  }
}

/**
 * Typed API client for the Platform Rust backend.
 *
 * Automatically includes the Clerk session token in requests.
 * All methods return typed responses matching the Rust DTOs.
 */
async function request<T>(
  path: string,
  options?: RequestInit & { token?: string }
): Promise<T> {
  const { token, ...fetchOptions } = options || {};

  const headers: Record<string, string> = {
    ...(fetchOptions.headers as Record<string, string>),
  };

  if (token) {
    headers["Authorization"] = `Bearer ${token}`;
  }

  if (fetchOptions.body && typeof fetchOptions.body === "string") {
    headers["Content-Type"] = "application/json";
  }

  const res = await fetch(`${API_URL}${path}`, {
    ...fetchOptions,
    headers,
  });

  if (!res.ok) {
    const body = (await res.json()) as ApiError;
    throw new ApiClientError(res.status, body);
  }

  if (res.status === 204) {
    return undefined as T;
  }

  return res.json();
}

// ── Project types ──

export interface Project {
  id: string;
  name: string;
  description: string | null;
  task_type: string | null;
  status: string;
  created_at: string;
  updated_at: string;
}

export interface PaginatedResponse<T> {
  data: T[];
  total: number;
  offset: number;
  limit: number;
}

export interface CreateProjectInput {
  name: string;
  description?: string;
  task_type?: string;
}

// ── Document types ──

export interface Document {
  id: string;
  project_id: string;
  filename: string;
  file_size: number;
  mime_type: string;
  status: string;
  parse_quality: number | null;
  page_count: number | null;
  language: string | null;
  domain: string | null;
  created_at: string;
  updated_at: string;
}

export interface UploadResponse {
  id: string;
  filename: string;
  file_size: number;
  status: string;
}

// ── Dataset types ──

export interface Dataset {
  id: string;
  project_id: string;
  name: string;
  format: string;
  status: string;
  pair_count: number | null;
  stats: Record<string, unknown>;
  created_at: string;
  updated_at: string;
}

// ── Training job types ──

export interface TrainingJob {
  id: string;
  project_id: string;
  dataset_id: string;
  base_model: string;
  method: string;
  mode: string;
  hyperparams: Record<string, unknown>;
  gpu_class: string | null;
  status: string;
  cost_estimate: number | null;
  actual_cost: number | null;
  metrics: Record<string, unknown>;
  started_at: string | null;
  completed_at: string | null;
  error_message: string | null;
  created_at: string;
  updated_at: string;
}

export interface CreateTrainingJobInput {
  dataset_id: string;
  base_model: string;
  method?: string;
  mode?: string;
  hyperparams?: Record<string, unknown>;
  gpu_class?: string;
}

// ── Model types ──

export interface Model {
  id: string;
  project_id: string;
  training_job_id: string;
  name: string;
  base_model: string;
  deployment_status: string;
  eval_scores: Record<string, unknown>;
  version: number;
  created_at: string;
  updated_at: string;
}

// ── Training metrics ──

export interface TrainingMetricsEntry {
  step: number;
  epoch: number;
  loss: number;
  learning_rate: number;
  grad_norm: number;
  phase: string;
  timestamp: string;
}

// ── Pipeline types ──

export interface TriggerParseResponse {
  workflow_id: string;
  document_count: number;
}

export interface TriggerRefineResponse {
  workflow_id: string;
  document_count: number;
}

export interface ProjectPipelineStatus {
  project_id: string;
  documents: {
    total: number;
    uploaded: number;
    parsing: number;
    parsed: number;
    failed: number;
  };
  datasets: {
    total: number;
    generating: number;
    review_pending: number;
    approved: number;
  };
  training_jobs: {
    total: number;
    pending: number;
    training: number;
    completed: number;
    failed: number;
  };
  models: {
    total: number;
    undeployed: number;
    active: number;
  };
  evaluations: {
    total: number;
    running: number;
    completed: number;
    failed: number;
  };
}

export interface ParsedContentResponse {
  url: string;
}

// ── Evaluation types ──

export interface Evaluation {
  id: string;
  model_id: string;
  status: string;
  scores: EvaluationScores | null;
  report: Record<string, unknown>;
  started_at: string | null;
  completed_at: string | null;
  created_at: string;
  updated_at: string;
}

export interface EvaluationScores {
  domain: {
    accuracy: number;
    completeness: number;
    faithfulness: number;
    mean: number;
  };
  general: {
    base_score: number;
    finetuned_score: number;
    delta_pct: number;
    forgetting_alert: boolean;
    categories: Record<string, { base: number; finetuned: number }>;
  };
  ab_comparison: {
    win_rate: number;
    confidence_low: number;
    confidence_high: number;
    total_comparisons: number;
  };
  safety: {
    refusal_rate: number;
    base_refusal_rate: number;
    degraded: boolean;
    categories: Record<string, { refusal_rate: number }>;
  };
  overall: number;
}

export interface CreateEvaluationInput {
  judge_model?: string;
  judge_api_base?: string;
}

// ── API Key types ──

export interface ApiKeyResponse {
  id: string;
  model_id: string;
  name: string;
  key_prefix: string;
  rate_limit: number;
  is_active: boolean;
  last_used_at: string | null;
  expires_at: string | null;
  created_at: string;
}

export interface CreateApiKeyResponse {
  id: string;
  name: string;
  key: string;
  key_prefix: string;
  rate_limit: number;
  expires_at: string | null;
  created_at: string;
}

export interface CreateApiKeyInput {
  name: string;
  rate_limit?: number;
  expires_in_days?: number;
}

// ── Deployment types ──

export interface DeploymentStatus {
  model_id: string;
  deployment_status: string;
  deployment_config: Record<string, unknown>;
  base_model: string;
  adapter_path: string | null;
}

// ── API methods ──

async function uploadRequest(
  path: string,
  token: string,
  formData: FormData,
): Promise<UploadResponse[]> {
  const res = await fetch(`${API_URL}${path}`, {
    method: "POST",
    headers: { Authorization: `Bearer ${token}` },
    body: formData,
  });

  if (!res.ok) {
    const body = (await res.json()) as ApiError;
    throw new ApiClientError(res.status, body);
  }

  return res.json();
}

export const api = {
  projects: {
    list: (token: string, offset = 0, limit = 20) =>
      request<PaginatedResponse<Project>>(
        `/api/v1/projects?offset=${offset}&limit=${limit}`,
        { token }
      ),

    get: (token: string, id: string) =>
      request<Project>(`/api/v1/projects/${id}`, { token }),

    create: (token: string, data: CreateProjectInput) =>
      request<Project>("/api/v1/projects", {
        token,
        method: "POST",
        body: JSON.stringify(data),
      }),

    delete: (token: string, id: string) =>
      request<void>(`/api/v1/projects/${id}`, { token, method: "DELETE" }),
  },

  documents: {
    list: (token: string, projectId: string, offset = 0, limit = 50) =>
      request<PaginatedResponse<Document>>(
        `/api/v1/projects/${projectId}/documents?offset=${offset}&limit=${limit}`,
        { token }
      ),

    get: (token: string, id: string) =>
      request<Document>(`/api/v1/documents/${id}`, { token }),

    upload: (token: string, projectId: string, files: File[]) => {
      const formData = new FormData();
      for (const file of files) {
        formData.append("files", file);
      }
      return uploadRequest(
        `/api/v1/projects/${projectId}/documents`,
        token,
        formData,
      );
    },

    getParsed: (token: string, id: string) =>
      request<ParsedContentResponse>(`/api/v1/documents/${id}/parsed`, { token }),
  },

  pipeline: {
    triggerParse: (token: string, projectId: string) =>
      request<TriggerParseResponse>(`/api/v1/projects/${projectId}/parse`, {
        token,
        method: "POST",
        body: JSON.stringify({}),
      }),

    triggerRefine: (
      token: string,
      projectId: string,
      taskType?: string,
      config?: Record<string, unknown>,
    ) =>
      request<TriggerRefineResponse>(`/api/v1/projects/${projectId}/refine`, {
        token,
        method: "POST",
        body: JSON.stringify({ task_type: taskType, config: config || {} }),
      }),

    getStatus: (token: string, projectId: string) =>
      request<ProjectPipelineStatus>(
        `/api/v1/projects/${projectId}/status`,
        { token }
      ),
  },

  datasets: {
    list: (token: string, projectId: string, offset = 0, limit = 20) =>
      request<PaginatedResponse<Dataset>>(
        `/api/v1/projects/${projectId}/datasets?offset=${offset}&limit=${limit}`,
        { token }
      ),

    get: (token: string, id: string) =>
      request<Dataset>(`/api/v1/datasets/${id}`, { token }),

    preview: (token: string, id: string, maxRows = 20) =>
      request<Record<string, unknown>[]>(
        `/api/v1/datasets/${id}/preview?max_rows=${maxRows}`,
        { token }
      ),
  },

  trainingJobs: {
    list: (token: string, projectId: string, offset = 0, limit = 20) =>
      request<PaginatedResponse<TrainingJob>>(
        `/api/v1/projects/${projectId}/training-jobs?offset=${offset}&limit=${limit}`,
        { token }
      ),

    get: (token: string, id: string) =>
      request<TrainingJob>(`/api/v1/training-jobs/${id}`, { token }),

    create: (token: string, projectId: string, data: CreateTrainingJobInput) =>
      request<TrainingJob>(`/api/v1/projects/${projectId}/training-jobs`, {
        token,
        method: "POST",
        body: JSON.stringify(data),
      }),

    cancel: (token: string, id: string) =>
      request<TrainingJob>(`/api/v1/training-jobs/${id}/cancel`, {
        token,
        method: "POST",
        body: JSON.stringify({}),
      }),

    getMetrics: (token: string, id: string) =>
      request<Record<string, unknown>>(`/api/v1/training-jobs/${id}/metrics`, {
        token,
      }),
  },

  models: {
    list: (token: string, projectId: string, offset = 0, limit = 20) =>
      request<PaginatedResponse<Model>>(
        `/api/v1/projects/${projectId}/models?offset=${offset}&limit=${limit}`,
        { token }
      ),

    get: (token: string, id: string) =>
      request<Model>(`/api/v1/models/${id}`, { token }),
  },

  evaluations: {
    list: (token: string, modelId: string, offset = 0, limit = 20) =>
      request<PaginatedResponse<Evaluation>>(
        `/api/v1/models/${modelId}/evaluations?offset=${offset}&limit=${limit}`,
        { token }
      ),

    get: (token: string, id: string) =>
      request<Evaluation>(`/api/v1/evaluations/${id}`, { token }),

    create: (token: string, modelId: string, data: CreateEvaluationInput) =>
      request<Evaluation>(`/api/v1/models/${modelId}/evaluations`, {
        token,
        method: "POST",
        body: JSON.stringify(data),
      }),
  },

  apiKeys: {
    list: (token: string, modelId: string) =>
      request<ApiKeyResponse[]>(
        `/api/v1/models/${modelId}/api-keys`,
        { token }
      ),

    create: (token: string, modelId: string, data: CreateApiKeyInput) =>
      request<CreateApiKeyResponse>(`/api/v1/models/${modelId}/api-keys`, {
        token,
        method: "POST",
        body: JSON.stringify(data),
      }),

    revoke: (token: string, id: string) =>
      request<ApiKeyResponse>(`/api/v1/api-keys/${id}/revoke`, {
        token,
        method: "POST",
        body: JSON.stringify({}),
      }),
  },

  deployments: {
    deploy: (token: string, modelId: string) =>
      request<Model>(`/api/v1/models/${modelId}/deploy`, {
        token,
        method: "POST",
        body: JSON.stringify({}),
      }),

    undeploy: (token: string, modelId: string) =>
      request<Model>(`/api/v1/models/${modelId}/undeploy`, {
        token,
        method: "POST",
        body: JSON.stringify({}),
      }),

    status: (token: string, modelId: string) =>
      request<DeploymentStatus>(
        `/api/v1/models/${modelId}/deployment`,
        { token }
      ),
  },
};
