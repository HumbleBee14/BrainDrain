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

// ── API methods ──

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
};
