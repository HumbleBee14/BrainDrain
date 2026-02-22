use platform_db::models::Project;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppResult;
use crate::repositories::traits::{BoxFuture, ProjectRepository};

/// PostgreSQL implementation of the project repository.
///
/// All queries require `tenant_id` — multi-tenancy is enforced at this layer.
/// No query ever returns data outside the caller's tenant boundary.
pub struct PgProjectRepo {
    db: PgPool,
}

impl PgProjectRepo {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }
}

impl ProjectRepository for PgProjectRepo {
    fn create(
        &self,
        tenant_id: Uuid,
        name: &str,
        description: Option<&str>,
        task_type: Option<&str>,
    ) -> BoxFuture<'_, AppResult<Project>> {
        let name = name.to_string();
        let description = description.map(|s| s.to_string());
        let task_type = task_type.map(|s| s.to_string());
        Box::pin(async move {
            let project = sqlx::query_as::<_, Project>(
                r#"
                INSERT INTO projects (tenant_id, name, description, task_type)
                VALUES ($1, $2, $3, $4)
                RETURNING *
                "#,
            )
            .bind(tenant_id)
            .bind(&name)
            .bind(description.as_deref())
            .bind(task_type.as_deref())
            .fetch_one(&self.db)
            .await?;

            Ok(project)
        })
    }

    fn create_with_limit(
        &self,
        tenant_id: Uuid,
        name: &str,
        description: Option<&str>,
        task_type: Option<&str>,
        max_count: i64,
    ) -> BoxFuture<'_, AppResult<Option<Project>>> {
        let name = name.to_string();
        let description = description.map(|s| s.to_string());
        let task_type = task_type.map(|s| s.to_string());
        Box::pin(async move {
            let project = sqlx::query_as::<_, Project>(
                r#"
                INSERT INTO projects (tenant_id, name, description, task_type)
                SELECT $1, $2, $3, $4
                WHERE (SELECT COUNT(*) FROM projects WHERE tenant_id = $1 AND deleted_at IS NULL) < $5
                RETURNING *
                "#,
            )
            .bind(tenant_id)
            .bind(&name)
            .bind(description.as_deref())
            .bind(task_type.as_deref())
            .bind(max_count)
            .fetch_optional(&self.db)
            .await?;

            Ok(project)
        })
    }

    fn get_by_id(
        &self,
        tenant_id: Uuid,
        project_id: Uuid,
    ) -> BoxFuture<'_, AppResult<Option<Project>>> {
        Box::pin(async move {
            let project = sqlx::query_as::<_, Project>(
                r#"
                SELECT * FROM projects
                WHERE id = $1 AND tenant_id = $2 AND deleted_at IS NULL
                "#,
            )
            .bind(project_id)
            .bind(tenant_id)
            .fetch_optional(&self.db)
            .await?;

            Ok(project)
        })
    }

    fn list(
        &self,
        tenant_id: Uuid,
        offset: i64,
        limit: i64,
    ) -> BoxFuture<'_, AppResult<Vec<Project>>> {
        Box::pin(async move {
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
            .fetch_all(&self.db)
            .await?;

            Ok(projects)
        })
    }

    fn count(&self, tenant_id: Uuid) -> BoxFuture<'_, AppResult<i64>> {
        Box::pin(async move {
            let count = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM projects WHERE tenant_id = $1 AND deleted_at IS NULL",
            )
            .bind(tenant_id)
            .fetch_one(&self.db)
            .await?;

            Ok(count)
        })
    }

    fn update(
        &self,
        tenant_id: Uuid,
        project_id: Uuid,
        name: Option<&str>,
        description: Option<&str>,
        task_type: Option<&str>,
    ) -> BoxFuture<'_, AppResult<Option<Project>>> {
        let name = name.map(|s| s.to_string());
        let description = description.map(|s| s.to_string());
        let task_type = task_type.map(|s| s.to_string());
        Box::pin(async move {
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
            .bind(name.as_deref())
            .bind(description.as_deref())
            .bind(task_type.as_deref())
            .fetch_optional(&self.db)
            .await?;

            Ok(project)
        })
    }

    fn delete(&self, tenant_id: Uuid, project_id: Uuid) -> BoxFuture<'_, AppResult<bool>> {
        Box::pin(async move {
            let result = sqlx::query(
                r#"
                UPDATE projects
                SET deleted_at = now()
                WHERE id = $1 AND tenant_id = $2 AND deleted_at IS NULL
                "#,
            )
            .bind(project_id)
            .bind(tenant_id)
            .execute(&self.db)
            .await?;

            Ok(result.rows_affected() > 0)
        })
    }
}
