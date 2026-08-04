use serde::{Deserialize, Serialize};
use strum::{Display, EnumString};
use ts_rs::TS;
use utoipa::ToSchema;

/// Status of a document through the ingestion pipeline.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, EnumString, TS, ToSchema,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[ts(export)]
pub enum DocumentStatus {
    Uploaded,
    Parsing,
    Parsed,
    Failed,
}

/// Status of a dataset through the refinement pipeline.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, EnumString, TS, ToSchema,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[ts(export)]
pub enum DatasetStatus {
    Generating,
    ReviewPending,
    Approved,
    Archived,
    Failed,
}

/// Status of a training job through its lifecycle.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, EnumString, TS, ToSchema,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[ts(export)]
pub enum TrainingJobStatus {
    Pending,
    CostApproval,
    Provisioning,
    Training,
    Completed,
    Failed,
    Cancelled,
}

/// Training method used for fine-tuning.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, EnumString, TS, ToSchema,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[ts(export)]
pub enum TrainingMethod {
    Qlora,
    Lora,
    Full,
}

/// Training mode determines the pipeline stages.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, EnumString, TS, ToSchema,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[ts(export)]
pub enum TrainingMode {
    /// SFT only — fastest iteration
    Quick,
    /// SFT → DPO — production quality
    Aligned,
    /// SFT → GRPO — reasoning optimized
    Reasoning,
    /// Multiple train-eval-improve iterations
    Iterative,
    /// SFT on teacher-generated data, evaluated for teacher parity
    Distill,
}

/// How much of the teacher a distill run learns from.
///
/// Orthogonal to [`TrainingMethod`] (which is about how the student's weights
/// are updated) and to [`TrainingMode`] (which stays `distill` either way):
/// `Text` trains on the teacher's written answers, `Logit` additionally trains
/// on its per-token confidence.
/// `Text` is the default everywhere: higher fidelity costs GPU time, so it is
/// only ever entered deliberately.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Display,
    EnumString,
    TS,
    ToSchema,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[ts(export)]
pub enum DistillMethod {
    #[default]
    Text,
    Logit,
}

/// Weight precision a hosted teacher is loaded at, trading accuracy for GPU
/// memory and throughput.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Display,
    EnumString,
    TS,
    ToSchema,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[ts(export)]
pub enum TeacherPrecision {
    #[default]
    Fp8,
    Int4,
    Bf16,
}

/// Deployment status for a trained model.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, EnumString, TS, ToSchema,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[ts(export)]
pub enum DeploymentStatus {
    Undeployed,
    Deploying,
    Active,
    Inactive,
}

/// Serving instance health from the control plane's point of view.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Display,
    EnumString,
    TS,
    ToSchema,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[ts(export)]
pub enum InferenceInstanceHealthStatus {
    #[default]
    Unknown,
    Healthy,
    Unhealthy,
}

/// Operational lifecycle state for an inference instance.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Display,
    EnumString,
    TS,
    ToSchema,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[ts(export)]
pub enum InferenceInstanceLifecycleState {
    #[default]
    Ready,
    Draining,
    Retired,
}

/// Evaluation job status.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, EnumString, TS, ToSchema,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[ts(export)]
pub enum EvaluationStatus {
    Running,
    Completed,
    Failed,
}

/// Pipeline stage identifier.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, EnumString, TS, ToSchema,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[ts(export)]
pub enum PipelineStage {
    Ingest,
    Refine,
    Train,
    Evaluate,
    Deploy,
}

/// Project task type — what the fine-tuned model should do.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, EnumString, TS, ToSchema,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[ts(export)]
pub enum TaskType {
    /// Answer questions about domain knowledge
    QuestionAnswering,
    /// Follow specific instructions or writing style
    InstructionFollowing,
    /// Analyze and reason about complex problems
    Reasoning,
    /// Custom user-defined task
    Custom,
}

/// User's subscription plan.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, EnumString, TS, ToSchema,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[ts(export)]
pub enum Plan {
    Starter,
    Growth,
    Pro,
    Enterprise,
}

/// Billing operation types.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, EnumString, TS, ToSchema,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[ts(export)]
pub enum BillingOperation {
    Parse,
    Synthesize,
    Train,
    Evaluate,
    Inference,
    Export,
}

/// GPU class for training provisioning.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, EnumString, TS, ToSchema,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[ts(export)]
pub enum GpuClass {
    T4,
    A10g,
    L40s,
    A10040gb,
    A10080gb,
    H100,
}

/// Project status.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, EnumString, TS, ToSchema,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[ts(export)]
pub enum ProjectStatus {
    Created,
    Ingesting,
    Refining,
    Training,
    Evaluating,
    Deployed,
    Archived,
}

/// Team member role for RBAC.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    Display,
    EnumString,
    TS,
    ToSchema,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[ts(export)]
pub enum TeamRole {
    Viewer,
    Member,
    Admin,
    Owner,
}

/// Status of a team invitation.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, EnumString, TS, ToSchema,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[ts(export)]
pub enum InvitationStatus {
    Pending,
    Accepted,
    Expired,
    Revoked,
}

/// Status of a data guide through the guided synthesis pipeline.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, EnumString, TS, ToSchema,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[ts(export)]
pub enum DataGuideStatus {
    Draft,
    GeneratingFacets,
    FacetsReady,
    GeneratingPreview,
    Ready,
    Generating,
    Completed,
    Failed,
}

/// User rating for a generated preview sample.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, EnumString, TS, ToSchema,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[ts(export)]
pub enum SampleRating {
    Realistic,
    NeedsWork,
}

/// End-user / reviewer feedback on a captured production inference response.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, EnumString, TS, ToSchema,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[ts(export)]
pub enum FeedbackRating {
    Positive,
    Negative,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn enum_serializes_to_snake_case() {
        let status = DocumentStatus::Parsing;
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, "\"parsing\"");

        let job = TrainingJobStatus::CostApproval;
        let json = serde_json::to_string(&job).unwrap();
        assert_eq!(json, "\"cost_approval\"");
    }

    #[test]
    fn enum_deserializes_from_snake_case() {
        let status: DocumentStatus = serde_json::from_str("\"uploaded\"").unwrap();
        assert_eq!(status, DocumentStatus::Uploaded);

        let method: TrainingMethod = serde_json::from_str("\"qlora\"").unwrap();
        assert_eq!(method, TrainingMethod::Qlora);
    }

    #[test]
    fn enum_display_is_snake_case() {
        assert_eq!(DocumentStatus::Parsed.to_string(), "parsed");
        assert_eq!(TrainingMode::Quick.to_string(), "quick");
        assert_eq!(GpuClass::A10080gb.to_string(), "a10080gb");
        assert_eq!(PipelineStage::Evaluate.to_string(), "evaluate");
    }

    #[test]
    fn enum_from_str_works() {
        assert_eq!(
            DocumentStatus::from_str("uploaded").unwrap(),
            DocumentStatus::Uploaded
        );
        assert_eq!(
            TrainingMethod::from_str("lora").unwrap(),
            TrainingMethod::Lora
        );
    }

    #[test]
    fn enum_from_str_rejects_invalid() {
        assert!(DocumentStatus::from_str("nonexistent").is_err());
        assert!(TrainingMethod::from_str("").is_err());
    }

    #[test]
    fn enum_roundtrip_json() {
        // Serialize then deserialize should give back the same value
        for status in [
            ProjectStatus::Created,
            ProjectStatus::Training,
            ProjectStatus::Archived,
        ] {
            let json = serde_json::to_string(&status).unwrap();
            let back: ProjectStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(status, back);
        }
    }

    #[test]
    fn team_role_ordering() {
        assert!(TeamRole::Viewer < TeamRole::Member);
        assert!(TeamRole::Member < TeamRole::Admin);
        assert!(TeamRole::Admin < TeamRole::Owner);
    }

    #[test]
    fn team_role_roundtrip() {
        for role in [
            TeamRole::Viewer,
            TeamRole::Member,
            TeamRole::Admin,
            TeamRole::Owner,
        ] {
            let json = serde_json::to_string(&role).unwrap();
            let back: TeamRole = serde_json::from_str(&json).unwrap();
            assert_eq!(role, back);
        }
    }

    #[test]
    fn invitation_status_roundtrip() {
        for status in [
            InvitationStatus::Pending,
            InvitationStatus::Accepted,
            InvitationStatus::Expired,
            InvitationStatus::Revoked,
        ] {
            let json = serde_json::to_string(&status).unwrap();
            let back: InvitationStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(status, back);
        }
    }

    #[test]
    fn instance_health_status_roundtrip() {
        for status in [
            InferenceInstanceHealthStatus::Unknown,
            InferenceInstanceHealthStatus::Healthy,
            InferenceInstanceHealthStatus::Unhealthy,
        ] {
            let json = serde_json::to_string(&status).unwrap();
            let back: InferenceInstanceHealthStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(status, back);
        }
    }

    #[test]
    fn instance_health_status_default_is_unknown() {
        assert_eq!(
            InferenceInstanceHealthStatus::default(),
            InferenceInstanceHealthStatus::Unknown
        );
    }

    #[test]
    fn instance_lifecycle_roundtrip() {
        for state in [
            InferenceInstanceLifecycleState::Ready,
            InferenceInstanceLifecycleState::Draining,
            InferenceInstanceLifecycleState::Retired,
        ] {
            let json = serde_json::to_string(&state).unwrap();
            let back: InferenceInstanceLifecycleState = serde_json::from_str(&json).unwrap();
            assert_eq!(state, back);
        }
    }

    #[test]
    fn instance_lifecycle_default_is_ready() {
        assert_eq!(
            InferenceInstanceLifecycleState::default(),
            InferenceInstanceLifecycleState::Ready
        );
    }

    #[test]
    fn instance_health_display_is_snake_case() {
        assert_eq!(
            InferenceInstanceHealthStatus::Healthy.to_string(),
            "healthy"
        );
        assert_eq!(
            InferenceInstanceHealthStatus::Unhealthy.to_string(),
            "unhealthy"
        );
        assert_eq!(
            InferenceInstanceHealthStatus::Unknown.to_string(),
            "unknown"
        );
    }

    #[test]
    fn instance_lifecycle_display_is_snake_case() {
        assert_eq!(InferenceInstanceLifecycleState::Ready.to_string(), "ready");
        assert_eq!(
            InferenceInstanceLifecycleState::Draining.to_string(),
            "draining"
        );
        assert_eq!(
            InferenceInstanceLifecycleState::Retired.to_string(),
            "retired"
        );
    }

    #[test]
    fn data_guide_status_roundtrips_snake_case() {
        assert_eq!(DataGuideStatus::FacetsReady.to_string(), "facets_ready");
        assert_eq!(
            "generating_preview".parse::<DataGuideStatus>().unwrap(),
            DataGuideStatus::GeneratingPreview
        );
        assert_eq!(SampleRating::NeedsWork.to_string(), "needs_work");
    }
}
