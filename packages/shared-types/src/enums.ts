/**
 * Mirrors Rust enum definitions from crates/shared/src/enums.rs
 *
 * Keep in sync — any changes to the Rust enums must be reflected here.
 */

export const DocumentStatus = {
  Uploaded: "uploaded",
  Scanning: "scanning",
  Parsing: "parsing",
  Parsed: "parsed",
  Failed: "failed",
} as const;
export type DocumentStatus = (typeof DocumentStatus)[keyof typeof DocumentStatus];

export const DatasetStatus = {
  Generating: "generating",
  ReviewPending: "review_pending",
  Approved: "approved",
  Archived: "archived",
} as const;
export type DatasetStatus = (typeof DatasetStatus)[keyof typeof DatasetStatus];

export const TrainingJobStatus = {
  Pending: "pending",
  CostApproval: "cost_approval",
  Provisioning: "provisioning",
  Training: "training",
  Completed: "completed",
  Failed: "failed",
  Cancelled: "cancelled",
} as const;
export type TrainingJobStatus = (typeof TrainingJobStatus)[keyof typeof TrainingJobStatus];

export const TrainingMethod = {
  Qlora: "qlora",
  Lora: "lora",
  Full: "full",
} as const;
export type TrainingMethod = (typeof TrainingMethod)[keyof typeof TrainingMethod];

export const TrainingMode = {
  Quick: "quick",
  Aligned: "aligned",
  Reasoning: "reasoning",
  Iterative: "iterative",
} as const;
export type TrainingMode = (typeof TrainingMode)[keyof typeof TrainingMode];

export const DeploymentStatus = {
  Undeployed: "undeployed",
  Deploying: "deploying",
  Active: "active",
  Inactive: "inactive",
} as const;
export type DeploymentStatus = (typeof DeploymentStatus)[keyof typeof DeploymentStatus];

export const EvaluationStatus = {
  Running: "running",
  Completed: "completed",
  Failed: "failed",
} as const;
export type EvaluationStatus = (typeof EvaluationStatus)[keyof typeof EvaluationStatus];

export const PipelineStage = {
  Ingest: "ingest",
  Refine: "refine",
  Train: "train",
  Evaluate: "evaluate",
  Deploy: "deploy",
} as const;
export type PipelineStage = (typeof PipelineStage)[keyof typeof PipelineStage];

export const TaskType = {
  QuestionAnswering: "question_answering",
  InstructionFollowing: "instruction_following",
  Reasoning: "reasoning",
  Custom: "custom",
} as const;
export type TaskType = (typeof TaskType)[keyof typeof TaskType];

export const ProjectStatus = {
  Created: "created",
  Ingesting: "ingesting",
  Refining: "refining",
  Training: "training",
  Evaluating: "evaluating",
  Deployed: "deployed",
  Archived: "archived",
} as const;
export type ProjectStatus = (typeof ProjectStatus)[keyof typeof ProjectStatus];

export const GpuClass = {
  T4: "t4",
  A10g: "a10g",
  L40s: "l40s",
  A10040gb: "a10040gb",
  A10080gb: "a10080gb",
  H100: "h100",
} as const;
export type GpuClass = (typeof GpuClass)[keyof typeof GpuClass];

export const BillingOperation = {
  Parse: "parse",
  Synthesize: "synthesize",
  Train: "train",
  Evaluate: "evaluate",
  Inference: "inference",
  Export: "export",
} as const;
export type BillingOperation = (typeof BillingOperation)[keyof typeof BillingOperation];

export const Plan = {
  Starter: "starter",
  Growth: "growth",
  Pro: "pro",
  Enterprise: "enterprise",
} as const;
export type Plan = (typeof Plan)[keyof typeof Plan];
