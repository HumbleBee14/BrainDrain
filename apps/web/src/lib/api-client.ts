import type {
  ProjectResponse,
  DocumentResponse,
  DatasetResponse,
  TrainingJobResponse,
  ModelResponse,
  EvaluationResponse,
  CreateProjectRequest,
  CreateTrainingJobRequest,
  CreateEvaluationRequest,
  CreateApiKeyRequest,
  CreateApiKeyResponse,
  ApiKeyResponse,
  DeploymentStatusResponse,
  UploadResponse,
  ProjectPipelineStatus,
  TriggerParseResponse,
  TriggerRefineResponse,
  PaginatedResponse,
} from "./generated";

// ── Re-export generated types with frontend-friendly aliases ──
// All types below are auto-generated from Rust DTOs via ts-rs.
// Run `cargo test --workspace` to regenerate.

export type {
  // Enums
  ProjectStatus,
  DocumentStatus,
  DatasetStatus,
  TrainingJobStatus,
  TrainingMethod,
  TrainingMode,
  EvaluationStatus,
  DeploymentStatus,
  // Typed structs
  EvaluationScores,
  Hyperparams,
  TrainingMetrics,
  // Response types (pass-through)
  ApiKeyResponse,
  CreateApiKeyResponse,
  DeploymentStatusResponse,
  UploadResponse,
  ProjectPipelineStatus,
  TriggerParseResponse,
  TriggerRefineResponse,
  PaginatedResponse,
} from "./generated";

// Response types with frontend-friendly aliases
export type { ProjectResponse as Project } from "./generated";
export type { DocumentResponse as Document } from "./generated";
export type { DatasetResponse as Dataset } from "./generated";
export type { TrainingJobResponse as TrainingJob } from "./generated";
export type { ModelResponse as Model } from "./generated";
export type { EvaluationResponse as Evaluation } from "./generated";

// Request types with frontend-friendly aliases
export type { CreateProjectRequest as CreateProjectInput } from "./generated";
export type { CreateTrainingJobRequest as CreateTrainingJobInput } from "./generated";
export type { CreateEvaluationRequest as CreateEvaluationInput } from "./generated";
export type { CreateApiKeyRequest as CreateApiKeyInput } from "./generated";

// Backward-compatible alias
export type { DeploymentStatus as ModelDeploymentStatus } from "./generated";

// ── Frontend-only types (not in Rust DTOs) ──

export interface TrainingMetricsEntry {
  step: number;
  epoch: number;
  loss: number;
  learning_rate: number;
  grad_norm: number;
  phase: string;
  timestamp: string;
}

export interface ParsedContentResponse {
  url: string;
}

// ── API client infrastructure ──

const API_URL = process.env.NEXT_PUBLIC_API_URL || "http://localhost:8000";

const DEFAULT_TIMEOUT_MS = 30_000;
const MAX_RETRIES = 3;
const BASE_BACKOFF_MS = 500;

interface ApiError {
  error: {
    code: string;
    message: string;
  };
}

export class ApiClientError extends Error {
  code: string;
  status: number;

  constructor(status: number, body: ApiError) {
    super(body.error.message);
    this.code = body.error.code;
    this.status = status;
  }
}

function isRetryable(error: unknown): boolean {
  if (error instanceof ApiClientError) {
    return error.status >= 500;
  }
  return (
    error instanceof TypeError ||
    (error instanceof DOMException && error.name === "AbortError")
  );
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

/**
 * Wraps fetch with a timeout (via AbortController) and retry logic
 * for transient failures (network errors, 5xx responses).
 *
 * Retries up to MAX_RETRIES times with exponential backoff.
 * Non-retryable errors (4xx, known client errors) are thrown immediately.
 */
async function fetchWithRetry(
  url: string,
  init?: RequestInit,
  timeoutMs: number = DEFAULT_TIMEOUT_MS,
): Promise<Response> {
  let lastError: unknown;

  for (let attempt = 0; attempt <= MAX_RETRIES; attempt++) {
    const controller = new AbortController();
    const timeoutId = setTimeout(() => controller.abort(), timeoutMs);

    if (init?.signal) {
      init.signal.addEventListener("abort", () => controller.abort(), {
        once: true,
      });
    }

    try {
      const res = await fetch(url, { ...init, signal: controller.signal });

      if (res.status >= 500 && attempt < MAX_RETRIES) {
        lastError = new ApiClientError(res.status, {
          error: { code: "server_error", message: `Server returned ${res.status}` },
        });
        await sleep(BASE_BACKOFF_MS * 2 ** attempt);
        continue;
      }

      return res;
    } catch (error) {
      lastError = error;

      if (!isRetryable(error) || attempt >= MAX_RETRIES) {
        throw error;
      }

      await sleep(BASE_BACKOFF_MS * 2 ** attempt);
    } finally {
      clearTimeout(timeoutId);
    }
  }

  throw lastError;
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

  const res = await fetchWithRetry(`${API_URL}${path}`, {
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

// ── API methods ──

async function uploadRequest(
  path: string,
  token: string,
  formData: FormData,
): Promise<UploadResponse[]> {
  const res = await fetchWithRetry(`${API_URL}${path}`, {
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
      request<PaginatedResponse<ProjectResponse>>(
        `/api/v1/projects?offset=${offset}&limit=${limit}`,
        { token }
      ),

    get: (token: string, id: string) =>
      request<ProjectResponse>(`/api/v1/projects/${id}`, { token }),

    create: (token: string, data: CreateProjectRequest) =>
      request<ProjectResponse>("/api/v1/projects", {
        token,
        method: "POST",
        body: JSON.stringify(data),
      }),

    delete: (token: string, id: string) =>
      request<void>(`/api/v1/projects/${id}`, { token, method: "DELETE" }),
  },

  documents: {
    list: (token: string, projectId: string, offset = 0, limit = 50) =>
      request<PaginatedResponse<DocumentResponse>>(
        `/api/v1/projects/${projectId}/documents?offset=${offset}&limit=${limit}`,
        { token }
      ),

    get: (token: string, id: string) =>
      request<DocumentResponse>(`/api/v1/documents/${id}`, { token }),

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
      request<PaginatedResponse<DatasetResponse>>(
        `/api/v1/projects/${projectId}/datasets?offset=${offset}&limit=${limit}`,
        { token }
      ),

    get: (token: string, id: string) =>
      request<DatasetResponse>(`/api/v1/datasets/${id}`, { token }),

    preview: (token: string, id: string, maxRows = 20) =>
      request<Record<string, unknown>[]>(
        `/api/v1/datasets/${id}/preview?max_rows=${maxRows}`,
        { token }
      ),
  },

  trainingJobs: {
    list: (token: string, projectId: string, offset = 0, limit = 20) =>
      request<PaginatedResponse<TrainingJobResponse>>(
        `/api/v1/projects/${projectId}/training-jobs?offset=${offset}&limit=${limit}`,
        { token }
      ),

    get: (token: string, id: string) =>
      request<TrainingJobResponse>(`/api/v1/training-jobs/${id}`, { token }),

    create: (token: string, projectId: string, data: CreateTrainingJobRequest) =>
      request<TrainingJobResponse>(`/api/v1/projects/${projectId}/training-jobs`, {
        token,
        method: "POST",
        body: JSON.stringify(data),
      }),

    cancel: (token: string, id: string) =>
      request<TrainingJobResponse>(`/api/v1/training-jobs/${id}/cancel`, {
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
      request<PaginatedResponse<ModelResponse>>(
        `/api/v1/projects/${projectId}/models?offset=${offset}&limit=${limit}`,
        { token }
      ),

    get: (token: string, id: string) =>
      request<ModelResponse>(`/api/v1/models/${id}`, { token }),
  },

  evaluations: {
    list: (token: string, modelId: string, offset = 0, limit = 20) =>
      request<PaginatedResponse<EvaluationResponse>>(
        `/api/v1/models/${modelId}/evaluations?offset=${offset}&limit=${limit}`,
        { token }
      ),

    get: (token: string, id: string) =>
      request<EvaluationResponse>(`/api/v1/evaluations/${id}`, { token }),

    create: (token: string, modelId: string, data: CreateEvaluationRequest) =>
      request<EvaluationResponse>(`/api/v1/models/${modelId}/evaluations`, {
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

    create: (token: string, modelId: string, data: CreateApiKeyRequest) =>
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
      request<ModelResponse>(`/api/v1/models/${modelId}/deploy`, {
        token,
        method: "POST",
        body: JSON.stringify({}),
      }),

    undeploy: (token: string, modelId: string) =>
      request<ModelResponse>(`/api/v1/models/${modelId}/undeploy`, {
        token,
        method: "POST",
        body: JSON.stringify({}),
      }),

    status: (token: string, modelId: string) =>
      request<DeploymentStatusResponse>(
        `/api/v1/models/${modelId}/deployment`,
        { token }
      ),
  },
};
