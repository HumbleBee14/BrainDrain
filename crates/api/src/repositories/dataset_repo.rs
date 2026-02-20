use platform_db::models::Dataset;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;

/// Repository for dataset database operations.
///
/// All queries require `tenant_id` — multi-tenancy enforced at this layer.
pub struct DatasetRepo;

impl DatasetRepo {
    /// List datasets for a project within a tenant.
    pub async fn list_by_project(
        db: &PgPool,
        tenant_id: Uuid,
        project_id: Uuid,
        offset: i64,
        limit: i64,
    ) -> Result<Vec<Dataset>, AppError> {
        let datasets = sqlx::query_as::<_, Dataset>(
            r#"
            SELECT * FROM datasets
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

        Ok(datasets)
    }

    /// Get a single dataset by ID.
    pub async fn get_by_id(
        db: &PgPool,
        tenant_id: Uuid,
        dataset_id: Uuid,
    ) -> Result<Option<Dataset>, AppError> {
        let dataset = sqlx::query_as::<_, Dataset>(
            "SELECT * FROM datasets WHERE id = $1 AND tenant_id = $2",
        )
        .bind(dataset_id)
        .bind(tenant_id)
        .fetch_optional(db)
        .await?;

        Ok(dataset)
    }

    /// Count datasets for a project.
    pub async fn count_by_project(
        db: &PgPool,
        tenant_id: Uuid,
        project_id: Uuid,
    ) -> Result<i64, AppError> {
        let count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM datasets WHERE project_id = $1 AND tenant_id = $2",
        )
        .bind(project_id)
        .bind(tenant_id)
        .fetch_one(db)
        .await?;

        Ok(count)
    }

    /// Count datasets by status for a project.
    pub async fn count_by_status(
        db: &PgPool,
        tenant_id: Uuid,
        project_id: Uuid,
        status: &str,
    ) -> Result<i64, AppError> {
        let count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM datasets WHERE project_id = $1 AND tenant_id = $2 AND status = $3",
        )
        .bind(project_id)
        .bind(tenant_id)
        .bind(status)
        .fetch_one(db)
        .await?;

        Ok(count)
    }
}
