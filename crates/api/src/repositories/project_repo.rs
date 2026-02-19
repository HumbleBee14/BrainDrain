use platform_db::models::Project;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;

/// Repository for project database operations.
///
/// All queries require `tenant_id` — multi-tenancy is enforced at this layer.
/// No query ever returns data outside the caller's tenant boundary.
pub struct ProjectRepo;

impl ProjectRepo {
    /// Create a new project.
    pub async fn create(
        db: &PgPool,
        tenant_id: Uuid,
        name: &str,
        description: Option<&str>,
        task_type: Option<&str>,
    ) -> Result<Project, AppError> {
        let project = sqlx::query_as::<_, Project>(
            r#"
            INSERT INTO projects (tenant_id, name, description, task_type)
            VALUES ($1, $2, $3, $4)
            RETURNING *
            "#,
        )
        .bind(tenant_id)
        .bind(name)
        .bind(description)
        .bind(task_type)
        .fetch_one(db)
        .await?;

        Ok(project)
    }

    /// Get a project by ID within a tenant.
    pub async fn get_by_id(
        db: &PgPool,
        tenant_id: Uuid,
        project_id: Uuid,
    ) -> Result<Option<Project>, AppError> {
        let project = sqlx::query_as::<_, Project>(
            r#"
            SELECT * FROM projects
            WHERE id = $1 AND tenant_id = $2 AND deleted_at IS NULL
            "#,
        )
        .bind(project_id)
        .bind(tenant_id)
        .fetch_optional(db)
        .await?;

        Ok(project)
    }

    /// List projects for a tenant with pagination.
    pub async fn list(
        db: &PgPool,
        tenant_id: Uuid,
        offset: i64,
        limit: i64,
    ) -> Result<Vec<Project>, AppError> {
        let projects = sqlx::query_as::<_, Project>(
            r#"
            SELECT * FROM projects
            WHERE tenant_id = $1 AND deleted_at IS NULL
            ORDER BY created_at DESC
            LIMIT $2 OFFSET $3
            "#,
        )
        .bind(tenant_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(db)
        .await?;

        Ok(projects)
    }

    /// Count projects for a tenant.
    pub async fn count(db: &PgPool, tenant_id: Uuid) -> Result<i64, AppError> {
        let count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM projects WHERE tenant_id = $1 AND deleted_at IS NULL",
        )
        .bind(tenant_id)
        .fetch_one(db)
        .await?;

        Ok(count)
    }

    /// Update a project's name and/or description.
    pub async fn update(
        db: &PgPool,
        tenant_id: Uuid,
        project_id: Uuid,
        name: Option<&str>,
        description: Option<&str>,
        task_type: Option<&str>,
    ) -> Result<Option<Project>, AppError> {
        let project = sqlx::query_as::<_, Project>(
            r#"
            UPDATE projects
            SET
                name = COALESCE($3, name),
                description = COALESCE($4, description),
                task_type = COALESCE($5, task_type)
            WHERE id = $1 AND tenant_id = $2 AND deleted_at IS NULL
            RETURNING *
            "#,
        )
        .bind(project_id)
        .bind(tenant_id)
        .bind(name)
        .bind(description)
        .bind(task_type)
        .fetch_optional(db)
        .await?;

        Ok(project)
    }

    /// Soft-delete a project.
    pub async fn delete(
        db: &PgPool,
        tenant_id: Uuid,
        project_id: Uuid,
    ) -> Result<bool, AppError> {
        let result = sqlx::query(
            r#"
            UPDATE projects
            SET deleted_at = now()
            WHERE id = $1 AND tenant_id = $2 AND deleted_at IS NULL
            "#,
        )
        .bind(project_id)
        .bind(tenant_id)
        .execute(db)
        .await?;

        Ok(result.rows_affected() > 0)
    }
}
