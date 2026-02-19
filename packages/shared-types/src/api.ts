/**
 * API response types matching the Rust DTOs from crates/api/src/dto/.
 *
 * Keep in sync with the Rust definitions.
 */

import type {
  DocumentStatus,
  ProjectStatus,
  TaskType,
  TrainingJobStatus,
  TrainingMethod,
  TrainingMode,
  DeploymentStatus,
  EvaluationStatus,
  DatasetStatus,
} from "./enums";

// ── Generic envelope ──

export interface ApiErrorResponse {
  error: {
    code: string;
    message: string;
    request_id?: string;
  };
}

export interface PaginatedResponse<T> {
  data: T[];
  total: number;
  offset: number;
  limit: number;
}

// ── Projects ──

export interface ProjectResponse {
  id: string;
  name: string;
  description: string | null;
  task_type: TaskType | null;
  status: ProjectStatus;
  created_at: string;
  updated_at: string;
}

export interface CreateProjectRequest {
  name: string;
  description?: string;
  task_type?: TaskType;
}

export interface UpdateProjectRequest {
  name?: string;
  description?: string;
  task_type?: TaskType;
}

// ── Documents ──

export interface DocumentResponse {
  id: string;
  project_id: string;
  filename: string;
  file_size: number;
  mime_type: string;
  status: DocumentStatus;
  parse_quality: number | null;
  page_count: number | null;
  language: string | null;
  domain: string | null;
  error_message: string | null;
  created_at: string;
  updated_at: string;
}

export interface UploadResponse {
  id: string;
  filename: string;
  file_size: number;
  status: DocumentStatus;
}

// ── Datasets ──

export interface DatasetResponse {
  id: string;
  project_id: string;
  name: string;
  format: string;
  status: DatasetStatus;
  pair_count: number | null;
  created_at: string;
  updated_at: string;
}

// ── Training Jobs ──

export interface TrainingJobResponse {
  id: string;
  project_id: string;
  dataset_id: string;
  base_model: string;
  method: TrainingMethod;
  mode: TrainingMode;
  hyperparams: Record<string, unknown>;
  gpu_class: string | null;
  status: TrainingJobStatus;
  cost_estimate: number | null;
  actual_cost: number | null;
  metrics: Record<string, unknown>;
  started_at: string | null;
  completed_at: string | null;
  error_message: string | null;
  created_at: string;
  updated_at: string;
}

// ── Models ──

export interface ModelResponse {
  id: string;
  project_id: string;
  training_job_id: string;
  name: string;
  base_model: string;
  deployment_status: DeploymentStatus;
  eval_scores: Record<string, unknown>;
  version: number;
  created_at: string;
  updated_at: string;
}

// ── Evaluations ──

export interface EvaluationResponse {
  id: string;
  model_id: string;
  status: EvaluationStatus;
  scores: Record<string, unknown>;
  report: Record<string, unknown>;
  started_at: string | null;
  completed_at: string | null;
  created_at: string;
  updated_at: string;
}
