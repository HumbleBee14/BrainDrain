use serde::{Deserialize, Serialize};
use strum::{Display, EnumString};
use ts_rs::TS;

/// Status of a document through the ingestion pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, EnumString, TS)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[ts(export)]
pub enum DocumentStatus {
    Uploaded,
    Scanning,
    Parsing,
    Parsed,
    Failed,
}

/// Status of a dataset through the refinement pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, EnumString, TS)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[ts(export)]
pub enum DatasetStatus {
    Generating,
    ReviewPending,
    Approved,
    Archived,
}

/// Status of a training job through its lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, EnumString, TS)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, EnumString, TS)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[ts(export)]
pub enum TrainingMethod {
    Qlora,
    Lora,
    Full,
}

/// Training mode determines the pipeline stages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, EnumString, TS)]
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
}

/// Deployment status for a trained model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, EnumString, TS)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[ts(export)]
pub enum DeploymentStatus {
    Undeployed,
    Deploying,
    Active,
    Inactive,
}

/// Evaluation job status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, EnumString, TS)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[ts(export)]
pub enum EvaluationStatus {
    Running,
    Completed,
    Failed,
}

/// Pipeline stage identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, EnumString, TS)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, EnumString, TS)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, EnumString, TS)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, EnumString, TS)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, EnumString, TS)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, EnumString, TS)]
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
}
