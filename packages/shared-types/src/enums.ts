/**
 * Mirrors Rust enum definitions from crates/shared/src/enums.rs
 *
 * Keep in sync — any changes to the Rust enums must be reflected here.
 */

export const DocumentStatus = {
  Uploaded: "uploaded",
  Parsing: "parsing",
  Parsed: "parsed",
  Failed: "failed",
} as const;
export type DocumentStatus = (typeof DocumentStatus)[keyof typeof DocumentStatus];

export const DatasetStatus = {
  Building: "building",
  Ready: "ready",
  Failed: "failed",
} as const;
export type DatasetStatus = (typeof DatasetStatus)[keyof typeof DatasetStatus];

export const TrainingJobStatus = {
  Queued: "queued",
  Preparing: "preparing",
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
  Sft: "sft",
  Dpo: "dpo",
  Orpo: "orpo",
} as const;
export type TrainingMode = (typeof TrainingMode)[keyof typeof TrainingMode];

export const DeploymentStatus = {
  NotDeployed: "not_deployed",
  Deploying: "deploying",
  Active: "active",
  Failed: "failed",
  Stopped: "stopped",
} as const;
export type DeploymentStatus = (typeof DeploymentStatus)[keyof typeof DeploymentStatus];

export const EvaluationStatus = {
  Pending: "pending",
  Running: "running",
  Completed: "completed",
  Failed: "failed",
} as const;
export type EvaluationStatus = (typeof EvaluationStatus)[keyof typeof EvaluationStatus];

export const PipelineStage = {
  Upload: "upload",
  Parse: "parse",
  Refine: "refine",
  Train: "train",
  Evaluate: "evaluate",
  Deploy: "deploy",
} as const;
export type PipelineStage = (typeof PipelineStage)[keyof typeof PipelineStage];

export const TaskType = {
  Chat: "chat",
  Instruct: "instruct",
  Classify: "classify",
  Extract: "extract",
  Summarize: "summarize",
  Code: "code",
  Custom: "custom",
} as const;
export type TaskType = (typeof TaskType)[keyof typeof TaskType];

export const ProjectStatus = {
  Active: "active",
  Archived: "archived",
} as const;
export type ProjectStatus = (typeof ProjectStatus)[keyof typeof ProjectStatus];
