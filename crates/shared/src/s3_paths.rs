use uuid::Uuid;

/// Build S3 key for a raw uploaded document.
pub fn upload_path(tenant_id: Uuid, project_id: Uuid, file_id: Uuid, ext: &str) -> String {
    format!("uploads/{tenant_id}/{project_id}/{file_id}.{ext}")
}

/// Build S3 key for parsed document output (structured JSON).
pub fn parsed_path(tenant_id: Uuid, project_id: Uuid, doc_id: Uuid) -> String {
    format!("parsed/{tenant_id}/{project_id}/{doc_id}.json")
}

/// Build S3 key for a training dataset (JSONL).
pub fn dataset_path(tenant_id: Uuid, project_id: Uuid, dataset_id: Uuid) -> String {
    format!("datasets/{tenant_id}/{project_id}/{dataset_id}.jsonl")
}

/// Build S3 key prefix for a trained model adapter.
pub fn adapter_prefix(tenant_id: Uuid, model_id: Uuid) -> String {
    format!("adapters/{tenant_id}/{model_id}/")
}

/// Build S3 key for a specific adapter file.
pub fn adapter_file(tenant_id: Uuid, model_id: Uuid, filename: &str) -> String {
    format!("adapters/{tenant_id}/{model_id}/{filename}")
}

/// Build S3 key for training checkpoints.
pub fn checkpoint_prefix(tenant_id: Uuid, training_id: Uuid) -> String {
    format!("checkpoints/{tenant_id}/{training_id}/")
}

/// Build S3 key for exported model files (GGUF, ONNX).
pub fn export_path(tenant_id: Uuid, model_id: Uuid, filename: &str) -> String {
    format!("exports/{tenant_id}/{model_id}/{filename}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_upload_path_format() {
        let tenant = Uuid::nil();
        let project = Uuid::nil();
        let file = Uuid::nil();
        let path = upload_path(tenant, project, file, "pdf");
        assert!(path.starts_with("uploads/"));
        assert!(path.ends_with(".pdf"));
    }

    #[test]
    fn test_adapter_prefix_ends_with_slash() {
        let tenant = Uuid::nil();
        let model = Uuid::nil();
        let prefix = adapter_prefix(tenant, model);
        assert!(prefix.ends_with('/'));
    }
}
