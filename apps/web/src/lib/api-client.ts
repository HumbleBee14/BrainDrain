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
}

export interface ParsedContentResponse {
  url: string;
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
};
