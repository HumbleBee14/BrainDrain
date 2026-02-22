use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use uuid::Uuid;

use platform_shared::enums::TeamRole;

use crate::app_state::AppState;
use crate::auth::AuthenticatedUser;
use crate::dto::common::{PaginatedResponse, PaginationParams};
use crate::dto::project::{CreateProjectRequest, ProjectResponse, UpdateProjectRequest};
use crate::error::AppResult;
use crate::rbac::require_role;
use crate::services::audit_logger::AuditLogger;
use crate::services::plan_service::PlanService;
use crate::services::project_service::ProjectService;

/// Project CRUD routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/projects", post(create_project))
        .route("/projects", get(list_projects))
        .route("/projects/{id}", get(get_project))
        .route("/projects/{id}", put(update_project))
        .route("/projects/{id}", delete(delete_project))
}

async fn create_project(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Json(body): Json<CreateProjectRequest>,
) -> AppResult<(StatusCode, Json<ProjectResponse>)> {
    require_role(&user, TeamRole::Member)?;
    // Atomic limit check: INSERT ... WHERE count < max — no TOCTOU race.
    let limits = PlanService::get_limits(state.tenant_repo(), user.tenant_id).await?;
    let project = ProjectService::create_with_limit(
        state.project_repo(),
        user.tenant_id,
        body,
        limits.max_projects,
    )
    .await?;
    AuditLogger::log(
        state.audit_log_repo(),
        &user,
        "create",
        "project",
        Some(project.id),
        serde_json::json!({"name": project.name}),
    )
    .await;
    Ok((StatusCode::CREATED, Json(project)))
}

async fn list_projects(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Query(params): Query<PaginationParams>,
) -> AppResult<Json<PaginatedResponse<ProjectResponse>>> {
    let result = ProjectService::list(
        state.project_repo(),
        user.tenant_id,
        params.offset,
        params.limit,
    )
    .await?;
    Ok(Json(result))
}

async fn get_project(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<ProjectResponse>> {
    let project = ProjectService::get(state.project_repo(), user.tenant_id, id).await?;
    Ok(Json(project))
}

async fn update_project(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateProjectRequest>,
) -> AppResult<Json<ProjectResponse>> {
    require_role(&user, TeamRole::Member)?;
    let project = ProjectService::update(state.project_repo(), user.tenant_id, id, body).await?;
    AuditLogger::log(
        state.audit_log_repo(),
        &user,
        "update",
        "project",
        Some(id),
        serde_json::json!({}),
    )
    .await;
    Ok(Json(project))
}

async fn delete_project(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> AppResult<StatusCode> {
    require_role(&user, TeamRole::Member)?;
    ProjectService::delete(state.project_repo(), user.tenant_id, id).await?;
    AuditLogger::log(
        state.audit_log_repo(),
        &user,
        "delete",
        "project",
        Some(id),
        serde_json::json!({}),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}
