use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Base trait for all pipeline events.
/// Every event is serializable and carries tenant context for multi-tenancy.
pub trait PipelineEvent: Serialize {
    fn event_type(&self) -> &'static str;
    fn tenant_id(&self) -> Uuid;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentUploadedEvent {
    pub tenant_id: Uuid,
    pub project_id: Uuid,
    pub document_id: Uuid,
    pub filename: String,
    pub mime_type: String,
    pub timestamp: DateTime<Utc>,
}

impl PipelineEvent for DocumentUploadedEvent {
    fn event_type(&self) -> &'static str {
        "document.uploaded"
    }
    fn tenant_id(&self) -> Uuid {
        self.tenant_id
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentParsedEvent {
    pub tenant_id: Uuid,
    pub project_id: Uuid,
    pub document_id: Uuid,
    pub parse_quality: f64,
    pub page_count: i32,
    pub timestamp: DateTime<Utc>,
}

impl PipelineEvent for DocumentParsedEvent {
    fn event_type(&self) -> &'static str {
        "document.parsed"
    }
    fn tenant_id(&self) -> Uuid {
        self.tenant_id
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatasetReadyEvent {
    pub tenant_id: Uuid,
    pub project_id: Uuid,
    pub dataset_id: Uuid,
    pub pair_count: i32,
    pub timestamp: DateTime<Utc>,
}

impl PipelineEvent for DatasetReadyEvent {
    fn event_type(&self) -> &'static str {
        "dataset.ready"
    }
    fn tenant_id(&self) -> Uuid {
        self.tenant_id
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrainingStartedEvent {
    pub tenant_id: Uuid,
    pub project_id: Uuid,
    pub training_job_id: Uuid,
    pub base_model: String,
    pub timestamp: DateTime<Utc>,
}

impl PipelineEvent for TrainingStartedEvent {
    fn event_type(&self) -> &'static str {
        "training.started"
    }
    fn tenant_id(&self) -> Uuid {
        self.tenant_id
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrainingCompletedEvent {
    pub tenant_id: Uuid,
    pub training_job_id: Uuid,
    pub model_id: Uuid,
    pub timestamp: DateTime<Utc>,
}

impl PipelineEvent for TrainingCompletedEvent {
    fn event_type(&self) -> &'static str {
        "training.completed"
    }
    fn tenant_id(&self) -> Uuid {
        self.tenant_id
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvaluationCompletedEvent {
    pub tenant_id: Uuid,
    pub model_id: Uuid,
    pub evaluation_id: Uuid,
    pub overall_score: f64,
    pub timestamp: DateTime<Utc>,
}

impl PipelineEvent for EvaluationCompletedEvent {
    fn event_type(&self) -> &'static str {
        "evaluation.completed"
    }
    fn tenant_id(&self) -> Uuid {
        self.tenant_id
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelDeployedEvent {
    pub tenant_id: Uuid,
    pub model_id: Uuid,
    pub deployment_type: String,
    pub timestamp: DateTime<Utc>,
}

impl PipelineEvent for ModelDeployedEvent {
    fn event_type(&self) -> &'static str {
        "model.deployed"
    }
    fn tenant_id(&self) -> Uuid {
        self.tenant_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_types_are_dotted_format() {
        let tid = Uuid::new_v4();
        let now = Utc::now();

        let uploaded = DocumentUploadedEvent {
            tenant_id: tid,
            project_id: Uuid::new_v4(),
            document_id: Uuid::new_v4(),
            filename: "test.pdf".to_string(),
            mime_type: "application/pdf".to_string(),
            timestamp: now,
        };
        assert_eq!(uploaded.event_type(), "document.uploaded");
        assert_eq!(uploaded.tenant_id(), tid);

        let completed = TrainingCompletedEvent {
            tenant_id: tid,
            training_job_id: Uuid::new_v4(),
            model_id: Uuid::new_v4(),
            timestamp: now,
        };
        assert_eq!(completed.event_type(), "training.completed");
    }

    #[test]
    fn events_serialize_to_json() {
        let event = DocumentUploadedEvent {
            tenant_id: Uuid::nil(),
            project_id: Uuid::nil(),
            document_id: Uuid::nil(),
            filename: "test.pdf".to_string(),
            mime_type: "application/pdf".to_string(),
            timestamp: Utc::now(),
        };

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"filename\":\"test.pdf\""));
        assert!(json.contains("\"mime_type\":\"application/pdf\""));
    }

    #[test]
    fn events_roundtrip_json() {
        let event = DatasetReadyEvent {
            tenant_id: Uuid::new_v4(),
            project_id: Uuid::new_v4(),
            dataset_id: Uuid::new_v4(),
            pair_count: 1500,
            timestamp: Utc::now(),
        };

        let json = serde_json::to_string(&event).unwrap();
        let back: DatasetReadyEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(event.pair_count, back.pair_count);
        assert_eq!(event.tenant_id, back.tenant_id);
    }
}
