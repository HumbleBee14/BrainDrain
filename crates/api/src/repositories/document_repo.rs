use platform_db::models::Document;
use platform_shared::enums::DocumentStatus;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppResult;
use crate::repositories::traits::{BoxFuture, DocumentRepository};

/// PostgreSQL implementation of the document repository.
///
/// All queries require `tenant_id` — multi-tenancy enforced at this layer.
pub struct PgDocumentRepo {
    db: PgPool,
}

impl PgDocumentRepo {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }
}

impl DocumentRepository for PgDocumentRepo {
    fn create(
        &self,
        tenant_id: Uuid,
        project_id: Uuid,
        filename: &str,
        file_size: i64,
        mime_type: &str,
        storage_path: &str,
    ) -> BoxFuture<'_, AppResult<Document>> {
        let filename = filename.to_string();
        let mime_type = mime_type.to_string();
        let storage_path = storage_path.to_string();
        Box::pin(async move {
            let doc = sqlx::query_as::<_, Document>(
                r#"
                INSERT INTO documents (tenant_id, project_id, filename, file_size, mime_type, storage_path)
                VALUES ($1, $2, $3, $4, $5, $6)
                RETURNING *
                "#,
            )
            .bind(tenant_id)
            .bind(project_id)
            .bind(&filename)
            .bind(file_size)
            .bind(&mime_type)
            .bind(&storage_path)
            .fetch_one(&self.db)
            .await?;

            Ok(doc)
        })
    }

    fn list_by_project(
        &self,
        tenant_id: Uuid,
        project_id: Uuid,
        offset: i64,
        limit: i64,
    ) -> BoxFuture<'_, AppResult<Vec<Document>>> {
        Box::pin(async move {
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
            .fetch_all(&self.db)
            .await?;

            Ok(docs)
        })
    }

    fn get_by_id(
        &self,
        tenant_id: Uuid,
        document_id: Uuid,
    ) -> BoxFuture<'_, AppResult<Option<Document>>> {
        Box::pin(async move {
            let doc = sqlx::query_as::<_, Document>(
                "SELECT * FROM documents WHERE id = $1 AND tenant_id = $2",
            )
            .bind(document_id)
            .bind(tenant_id)
            .fetch_optional(&self.db)
            .await?;

            Ok(doc)
        })
    }

    fn list_by_status(
        &self,
        tenant_id: Uuid,
        project_id: Uuid,
        status: DocumentStatus,
    ) -> BoxFuture<'_, AppResult<Vec<Document>>> {
        Box::pin(async move {
            let docs = sqlx::query_as::<_, Document>(
                r#"
                SELECT * FROM documents
                WHERE project_id = $1 AND tenant_id = $2 AND status = $3
                ORDER BY created_at DESC
                "#,
            )
            .bind(project_id)
            .bind(tenant_id)
            .bind(status.to_string())
            .fetch_all(&self.db)
            .await?;

            Ok(docs)
        })
    }

    fn count_by_status(
        &self,
        tenant_id: Uuid,
        project_id: Uuid,
        status: DocumentStatus,
    ) -> BoxFuture<'_, AppResult<i64>> {
        Box::pin(async move {
            let count = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM documents WHERE project_id = $1 AND tenant_id = $2 AND status = $3",
            )
            .bind(project_id)
            .bind(tenant_id)
            .bind(status.to_string())
            .fetch_one(&self.db)
            .await?;

            Ok(count)
        })
    }

    fn update_status(
        &self,
        tenant_id: Uuid,
        document_id: Uuid,
        status: DocumentStatus,
        error_message: Option<&str>,
    ) -> BoxFuture<'_, AppResult<bool>> {
        let error_message = error_message.map(|s| s.to_string());
        Box::pin(async move {
            let result = sqlx::query(
                r#"
                UPDATE documents
                SET status = $3, error_message = $4
                WHERE id = $1 AND tenant_id = $2
                "#,
            )
            .bind(document_id)
            .bind(tenant_id)
            .bind(status.to_string())
            .bind(error_message.as_deref())
            .execute(&self.db)
            .await?;

            Ok(result.rows_affected() > 0)
        })
    }

    fn count_by_project(
        &self,
        tenant_id: Uuid,
        project_id: Uuid,
    ) -> BoxFuture<'_, AppResult<i64>> {
        Box::pin(async move {
            let count = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM documents WHERE project_id = $1 AND tenant_id = $2",
            )
            .bind(project_id)
            .bind(tenant_id)
            .fetch_one(&self.db)
            .await?;

            Ok(count)
        })
    }
}
