//! Packages a trained model's LoRA adapter into a single archive.
//!
//! An adapter is a directory of files (`adapter_config.json`,
//! `adapter_model.safetensors`, tokenizer files) stored under one object-storage
//! prefix. Presigned URLs address a single object, so serving the adapter as one
//! download requires assembling an archive.

use std::io::{Cursor, Write};

use platform_storage::ObjectStorage;
use uuid::Uuid;
use zip::write::SimpleFileOptions;

use crate::error::{AppError, AppResult};
use crate::repositories::traits::ModelRepository;

/// A packaged adapter ready to send to the client.
pub struct AdapterArchive {
    pub filename: String,
    pub bytes: Vec<u8>,
}

pub struct AdapterDownloadService;

impl AdapterDownloadService {
    /// Build a zip of every object under the model's adapter prefix.
    pub async fn build_archive(
        model_repo: &dyn ModelRepository,
        storage: &impl ObjectStorage,
        tenant_id: Uuid,
        model_id: Uuid,
        max_bytes: i64,
    ) -> AppResult<AdapterArchive> {
        let model = model_repo
            .get_by_id(tenant_id, model_id)
            .await?
            .ok_or(AppError::NotFound {
                message: "Model not found".to_string(),
            })?;

        let adapter_path = model.adapter_path.ok_or(AppError::BadRequest {
            message: "Model has no adapter — training may not be complete".to_string(),
        })?;

        let prefix = format!("{}/", adapter_path.trim_end_matches('/'));
        let objects = storage.list_prefix(&prefix).await.map_err(|e| {
            AppError::Internal(anyhow::anyhow!("Failed to list adapter files: {e}"))
        })?;

        let files: Vec<_> = objects
            .into_iter()
            .filter(|object| !object.key.ends_with('/'))
            .collect();

        if files.is_empty() {
            return Err(AppError::NotFound {
                message: "Adapter files are no longer present in storage".to_string(),
            });
        }

        let total: i64 = files.iter().map(|object| object.size).sum();
        if total > max_bytes {
            return Err(AppError::BadRequest {
                message: format!(
                    "Adapter is too large to package ({total} bytes, limit {max_bytes}). \
                     Use the export flow instead."
                ),
            });
        }

        let mut entries = Vec::with_capacity(files.len());
        for object in &files {
            let data = storage.get(&object.key).await.map_err(|e| {
                AppError::Internal(anyhow::anyhow!(
                    "Failed to read adapter file {}: {e}",
                    object.key
                ))
            })?;
            let name = object
                .key
                .strip_prefix(&prefix)
                .unwrap_or(&object.key)
                .to_string();
            entries.push((name, data));
        }

        let bytes = tokio::task::spawn_blocking(move || build_zip(entries))
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("Archive task failed: {e}")))?
            .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to build archive: {e}")))?;

        Ok(AdapterArchive {
            filename: archive_filename(&model.name, model.version),
            bytes,
        })
    }
}

/// Safetensors are already dense, so entries are stored uncompressed.
fn build_zip(entries: Vec<(String, bytes::Bytes)>) -> std::io::Result<Vec<u8>> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);

    for (name, data) in entries {
        writer.start_file(name, options)?;
        writer.write_all(&data)?;
    }

    Ok(writer.finish()?.into_inner())
}

/// Slug the model name so the filename is safe for any client filesystem.
fn archive_filename(model_name: &str, version: i32) -> String {
    let slug: String = model_name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let slug = slug.trim_matches('-');
    let stem = if slug.is_empty() { "adapter" } else { slug };
    format!("{stem}-v{version}-adapter.zip")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filename_slugs_unsafe_characters() {
        assert_eq!(
            archive_filename("Support Bot / v2!", 3),
            "support-bot---v2-v3-adapter.zip"
        );
    }

    #[test]
    fn filename_falls_back_when_name_has_no_usable_characters() {
        assert_eq!(archive_filename("///", 1), "adapter-v1-adapter.zip");
    }

    #[test]
    fn zip_contains_every_entry_uncompressed() {
        let entries = vec![
            (
                "adapter_config.json".to_string(),
                bytes::Bytes::from_static(b"{}"),
            ),
            (
                "adapter_model.safetensors".to_string(),
                bytes::Bytes::from_static(b"weights"),
            ),
        ];

        let archive = build_zip(entries).expect("zip builds");
        let mut reader = zip::ZipArchive::new(Cursor::new(archive)).expect("archive is readable");

        assert_eq!(reader.len(), 2);
        let names: Vec<String> = reader.file_names().map(str::to_string).collect();
        assert!(names.contains(&"adapter_config.json".to_string()));
        assert!(names.contains(&"adapter_model.safetensors".to_string()));

        let mut entry = reader.by_name("adapter_model.safetensors").unwrap();
        assert_eq!(entry.compression(), zip::CompressionMethod::Stored);
        let mut contents = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut contents).unwrap();
        assert_eq!(contents, b"weights");
    }
}
