// Auto-generated barrel file for ts-rs types.
// Re-run `cargo test --workspace` to regenerate individual type files.
// Only this index.ts is manually maintained — all other files are generated.

// Enums (from crates/shared/src/enums.rs)
export type { BillingOperation } from "./BillingOperation";
export type { DatasetStatus } from "./DatasetStatus";
export type { DeploymentStatus } from "./DeploymentStatus";
export type { DocumentStatus } from "./DocumentStatus";
export type { EvaluationStatus } from "./EvaluationStatus";
export type { GpuClass } from "./GpuClass";
export type { PipelineStage } from "./PipelineStage";
export type { Plan } from "./Plan";
export type { ProjectStatus } from "./ProjectStatus";
export type { TaskType } from "./TaskType";
export type { TrainingJobStatus } from "./TrainingJobStatus";
export type { TrainingMethod } from "./TrainingMethod";
export type { TrainingMode } from "./TrainingMode";

// Typed structs (from crates/shared/src/types.rs)
export type { ABComparisonScores } from "./ABComparisonScores";
export type { DeploymentConfig } from "./DeploymentConfig";
export type { DomainScores } from "./DomainScores";
export type { EvaluationScores } from "./EvaluationScores";
export type { GeneralScores } from "./GeneralScores";
export type { Hyperparams } from "./Hyperparams";
export type { SafetyScores } from "./SafetyScores";
export type { TrainingMetrics } from "./TrainingMetrics";

// API responses (from crates/api/src/dto/)
export type { ApiKeyResponse } from "./ApiKeyResponse";
export type { BillingEventResponse } from "./BillingEventResponse";
export type { CostEstimateResponse } from "./CostEstimateResponse";
export type { CreateApiKeyRequest } from "./CreateApiKeyRequest";
export type { CreateApiKeyResponse } from "./CreateApiKeyResponse";
export type { CreateEvaluationRequest } from "./CreateEvaluationRequest";
export type { CreateProjectRequest } from "./CreateProjectRequest";
export type { CreateTrainingJobRequest } from "./CreateTrainingJobRequest";
export type { DatasetResponse } from "./DatasetResponse";
export type { DatasetStatusCounts } from "./DatasetStatusCounts";
export type { DeploymentStatusResponse } from "./DeploymentStatusResponse";
export type { DocumentResponse } from "./DocumentResponse";
export type { DocumentStatusCounts } from "./DocumentStatusCounts";
export type { EvaluationResponse } from "./EvaluationResponse";
export type { EvaluationStatusCounts } from "./EvaluationStatusCounts";
export type { ModelResponse } from "./ModelResponse";
export type { ModelStatusCounts } from "./ModelStatusCounts";
export type { ProjectPipelineStatus } from "./ProjectPipelineStatus";
export type { ProjectResponse } from "./ProjectResponse";
export type { TrainingJobResponse } from "./TrainingJobResponse";
export type { TrainingJobStatusCounts } from "./TrainingJobStatusCounts";
export type { TriggerParseResponse } from "./TriggerParseResponse";
export type { TriggerRefineRequest } from "./TriggerRefineRequest";
export type { TriggerRefineResponse } from "./TriggerRefineResponse";
export type { TriggerTrainResponse } from "./TriggerTrainResponse";
export type { UpdateProjectRequest } from "./UpdateProjectRequest";
export type { UploadResponse } from "./UploadResponse";

// Settings (from crates/api/src/dto/tenant_settings.rs)
export type { LlmSettingsResponse } from "./LlmSettingsResponse";
export type { LlmTestResponse } from "./LlmTestResponse";
export type { UpdateLlmSettingsRequest } from "./UpdateLlmSettingsRequest";
export type { AdminConfigResponse } from "./AdminConfigResponse";
export type { UpdateAdminConfigRequest } from "./UpdateAdminConfigRequest";

// Inference instances (from crates/api/src/dto/inference_instance.rs)
export type { InferenceInstanceResponse } from "./InferenceInstanceResponse";
export type { CreateInferenceInstanceRequest } from "./CreateInferenceInstanceRequest";
export type { UpdateInferenceInstanceLifecycleRequest } from "./UpdateInferenceInstanceLifecycleRequest";
export type { InferenceInstanceHealthStatus } from "./InferenceInstanceHealthStatus";
export type { InferenceInstanceLifecycleState } from "./InferenceInstanceLifecycleState";

// Model versioning (from crates/api/src/dto/model.rs)
export type { RollbackModelRequest } from "./RollbackModelRequest";

// Data Studio (from crates/api/src/dto/datagen.rs)
export type { DataGuideResponse } from "./DataGuideResponse";
export type { DataGuideStatus } from "./DataGuideStatus";
export type { Facet } from "./Facet";
export type { PreviewSample } from "./PreviewSample";
export type { SampleRating } from "./SampleRating";
export type { SampleRatingItem } from "./SampleRatingItem";
export type { CreateDataGuideRequest } from "./CreateDataGuideRequest";
export type { GenerateFacetsRequest } from "./GenerateFacetsRequest";
export type { UpdateFacetsRequest } from "./UpdateFacetsRequest";
export type { GeneratePreviewRequest } from "./GeneratePreviewRequest";
export type { RateSamplesRequest } from "./RateSamplesRequest";
export type { UpdateGuidanceRequest } from "./UpdateGuidanceRequest";
export type { GenerateDatasetRequest } from "./GenerateDatasetRequest";

// Generic wrapper — manually defined (ts-rs doesn't export generics directly)
export interface PaginatedResponse<T> {
  data: T[];
  total: number;
  offset: number;
  limit: number;
}
