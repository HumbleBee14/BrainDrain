use platform_db::models::ModelExport;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppResult;
use crate::repositories::traits::{BoxFuture, ExportRepository};

pub struct PgExportRepo {
    db: PgPool,
}

impl PgExportRepo {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }
}

impl ExportRepository for PgExportRepo {
    fn create(
        &self,
        tenant_id: Uuid,
        model_id: Uuid,
        format: &str,
        quant_type: &str,
    ) -> BoxFuture<'_, AppResult<ModelExport>> {
        let format = format.to_string();
        let quant_type = quant_type.to_string();
        Box::pin(async move {
            let export = sqlx::query_as::<_, ModelExport>(
                r#"
                INSERT INTO model_exports (tenant_id, model_id, format, quant_type)
                VALUES ($1, $2, $3, $4)
                RETURNING *
                "#,
            )
            .bind(tenant_id)
            .bind(model_id)
            .bind(&format)
            .bind(&quant_type)
            .fetch_one(&self.db)
            .await?;

            Ok(export)
        })
    }

    fn get_by_id(
        &self,
        tenant_id: Uuid,
        export_id: Uuid,
    ) -> BoxFuture<'_, AppResult<Option<ModelExport>>> {
        Box::pin(async move {
            let export = sqlx::query_as::<_, ModelExport>(
                "SELECT * FROM model_exports WHERE tenant_id = $1 AND id = $2",
            )
            .bind(tenant_id)
            .bind(export_id)
            .fetch_optional(&self.db)
            .await?;

            Ok(export)
        })
    }

    fn list_by_model(
        &self,
        tenant_id: Uuid,
        model_id: Uuid,
    ) -> BoxFuture<'_, AppResult<Vec<ModelExport>>> {
        Box::pin(async move {
            let exports = sqlx::query_as::<_, ModelExport>(
                r#"
                SELECT * FROM model_exports
                WHERE tenant_id = $1 AND model_id = $2
                ORDER BY created_at DESC
                "#,
            )
            .bind(tenant_id)
            .bind(model_id)
            .fetch_all(&self.db)
            .await?;

            Ok(exports)
        })
    }

    fn update_status(
        &self,
        tenant_id: Uuid,
        export_id: Uuid,
        status: &str,
        storage_path: Option<&str>,
        file_size_bytes: Option<i64>,
        error: Option<&str>,
    ) -> BoxFuture<'_, AppResult<bool>> {
        let status = status.to_string();
        let storage_path = storage_path.map(|s| s.to_string());
        let error = error.map(|s| s.to_string());
        Box::pin(async move {
            let result = sqlx::query(
                r#"
                UPDATE model_exports
                SET status = $3,
                    storage_path = COALESCE($4, storage_path),
                    file_size_bytes = COALESCE($5, file_size_bytes),
                    error = $6,
                    completed_at = CASE WHEN $3 IN ('completed', 'failed') THEN now() ELSE completed_at END
                WHERE tenant_id = $1 AND id = $2
                "#,
            )
            .bind(tenant_id)
            .bind(export_id)
            .bind(&status)
            .bind(&storage_path)
            .bind(file_size_bytes)
            .bind(&error)
            .execute(&self.db)
            .await?;

            Ok(result.rows_affected() > 0)
        })
    }
}
