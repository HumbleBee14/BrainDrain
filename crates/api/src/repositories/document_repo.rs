use platform_db::models::Document;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;

/// Repository for document database operations.
///
/// All queries require `tenant_id` — multi-tenancy enforced at this layer.
pub struct DocumentRepo;

impl DocumentRepo {
    /// Create a new document record after upload.
    pub async fn create(
        db: &PgPool,
        tenant_id: Uuid,
        project_id: Uuid,
        filename: &str,
        file_size: i64,
        mime_type: &str,
        storage_path: &str,
    ) -> Result<Document, AppError> {
        let doc = sqlx::query_as::<_, Document>(
            r#"
            INSERT INTO documents (tenant_id, project_id, filename, file_size, mime_type, storage_path)
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING *
            "#,
        )
        .bind(tenant_id)
        .bind(project_id)
        .bind(filename)
        .bind(file_size)
        .bind(mime_type)
        .bind(storage_path)
        .fetch_one(db)
        .await?;

        Ok(doc)
    }

    /// List documents for a project within a tenant.
    pub async fn list_by_project(
        db: &PgPool,
        tenant_id: Uuid,
        project_id: Uuid,
        offset: i64,
        limit: i64,
    ) -> Result<Vec<Document>, AppError> {
        let docs = sqlx::query_as::<_, Document>(
            r#"
            SELECT * FROM documents
            WHERE project_id = $1 AND tenant_id = $2
            ORDER BY created_at DESC
            LIMIT $3 OFFSET $4
            "#,
        )
        .bind(project_id)
        .bind(tenant_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(db)
        .await?;

        Ok(docs)
    }

    /// Get a single document by ID.
    pub async fn get_by_id(
        db: &PgPool,
        tenant_id: Uuid,
        document_id: Uuid,
    ) -> Result<Option<Document>, AppError> {
        let doc = sqlx::query_as::<_, Document>(
            "SELECT * FROM documents WHERE id = $1 AND tenant_id = $2",
        )
        .bind(document_id)
        .bind(tenant_id)
        .fetch_optional(db)
        .await?;

        Ok(doc)
    }

    /// List documents by status for a project.
    pub async fn list_by_status(
        db: &PgPool,
        tenant_id: Uuid,
        project_id: Uuid,
        status: &str,
    ) -> Result<Vec<Document>, AppError> {
        let docs = sqlx::query_as::<_, Document>(
            r#"
            SELECT * FROM documents
            WHERE project_id = $1 AND tenant_id = $2 AND status = $3
            ORDER BY created_at DESC
            "#,
        )
        .bind(project_id)
        .bind(tenant_id)
        .bind(status)
        .fetch_all(db)
        .await?;

        Ok(docs)
    }

    /// Count documents by status for a project.
    pub async fn count_by_status(
        db: &PgPool,
        tenant_id: Uuid,
        project_id: Uuid,
        status: &str,
    ) -> Result<i64, AppError> {
        let count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM documents WHERE project_id = $1 AND tenant_id = $2 AND status = $3",
        )
        .bind(project_id)
        .bind(tenant_id)
        .bind(status)
        .fetch_one(db)
        .await?;

        Ok(count)
    }

    /// Update document status (used by pipeline workflows).
    #[allow(dead_code)]
    pub async fn update_status(
        db: &PgPool,
        tenant_id: Uuid,
        document_id: Uuid,
        status: &str,
        error_message: Option<&str>,
    ) -> Result<bool, AppError> {
        let result = sqlx::query(
            r#"
            UPDATE documents
            SET status = $3, error_message = $4
            WHERE id = $1 AND tenant_id = $2
            "#,
        )
        .bind(document_id)
        .bind(tenant_id)
        .bind(status)
        .bind(error_message)
        .execute(db)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Count documents for a project.
    pub async fn count_by_project(
        db: &PgPool,
        tenant_id: Uuid,
        project_id: Uuid,
    ) -> Result<i64, AppError> {
        let count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM documents WHERE project_id = $1 AND tenant_id = $2",
        )
        .bind(project_id)
        .bind(tenant_id)
        .fetch_one(db)
        .await?;

        Ok(count)
    }
}
