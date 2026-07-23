use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use uuid::Uuid;

use platform_shared::enums::TeamRole;

use crate::app_state::AppState;
use crate::auth::AuthenticatedUser;
use crate::dto::common::{PaginatedResponse, PaginationParams};
use crate::dto::project::{
    CreateProjectRequest, ProjectResponse, UpdateProjectRequest, UpdateProjectStatusRequest,
};
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
        // GET (pipeline status) and PUT (project state machine) share this
        // path; both live here because two merged routers must not register
        // the same path shape twice (axum panics at startup — see pipeline.rs).
        .route(
            "/projects/{id}/status",
            put(update_project_status).get(crate::routes::pipeline::get_status),
        )
        .route("/projects/{id}", delete(delete_project))
}

#[utoipa::path(
    post,
    path = "/api/v1/projects",
    tag = "Projects",
    request_body = CreateProjectRequest,
    responses(
        (status = 201, description = "Project created", body = ProjectResponse),
        (status = 400, body = crate::error::ErrorEnvelope),
        (status = 401, body = crate::error::ErrorEnvelope),
    ),
    security(("jwt" = []))
)]
pub async fn create_project(
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

#[utoipa::path(
    get,
    path = "/api/v1/projects",
    tag = "Projects",
    params(
        ("offset" = Option<i64>, Query, description = "Pagination offset"),
        ("limit" = Option<i64>, Query, description = "Pagination limit"),
    ),
    responses(
        (status = 200, description = "List of projects", body = inline(PaginatedResponse<ProjectResponse>)),
        (status = 401, body = crate::error::ErrorEnvelope),
    ),
    security(("jwt" = []))
)]
pub async fn list_projects(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Query(params): Query<PaginationParams>,
) -> AppResult<Json<PaginatedResponse<ProjectResponse>>> {
    let result = ProjectService::list(
        state.project_repo(),
        user.tenant_id,
        params.offset(),
        params.limit(),
    )
    .await?;
    Ok(Json(result))
}

#[utoipa::path(
    get,
    path = "/api/v1/projects/{id}",
    tag = "Projects",
    params(("id" = Uuid, Path, description = "Project ID")),
    responses(
        (status = 200, description = "Project details", body = ProjectResponse),
        (status = 404, body = crate::error::ErrorEnvelope),
    ),
    security(("jwt" = []))
)]
pub async fn get_project(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<ProjectResponse>> {
    let project = ProjectService::get(state.project_repo(), user.tenant_id, id).await?;
    Ok(Json(project))
}

#[utoipa::path(
    put,
    path = "/api/v1/projects/{id}",
    tag = "Projects",
    params(("id" = Uuid, Path, description = "Project ID")),
    request_body = UpdateProjectRequest,
    responses(
        (status = 200, description = "Project updated", body = ProjectResponse),
        (status = 404, body = crate::error::ErrorEnvelope),
    ),
    security(("jwt" = []))
)]
pub async fn update_project(
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

/// PUT /api/v1/projects/:id/status
///
/// Update a project's status with state machine validation.
#[utoipa::path(
    put,
    path = "/api/v1/projects/{id}/status",
    tag = "Projects",
    params(("id" = Uuid, Path, description = "Project ID")),
    request_body = UpdateProjectStatusRequest,
    responses(
        (status = 200, description = "Status updated", body = ProjectResponse),
        (status = 400, description = "Invalid transition", body = crate::error::ErrorEnvelope),
        (status = 404, body = crate::error::ErrorEnvelope),
    ),
    security(("jwt" = []))
)]
pub async fn update_project_status(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateProjectStatusRequest>,
) -> AppResult<Json<ProjectResponse>> {
    require_role(&user, TeamRole::Member)?;
    let project =
        ProjectService::update_status(state.project_repo(), user.tenant_id, id, body.status)
            .await?;
    AuditLogger::log(
        state.audit_log_repo(),
        &user,
        "update_status",
        "project",
        Some(id),
        serde_json::json!({"new_status": project.status}),
    )
    .await;
    Ok(Json(project))
}

#[utoipa::path(
    delete,
    path = "/api/v1/projects/{id}",
    tag = "Projects",
    params(("id" = Uuid, Path, description = "Project ID")),
    responses(
        (status = 204, description = "Project deleted"),
        (status = 404, body = crate::error::ErrorEnvelope),
    ),
    security(("jwt" = []))
)]
pub async fn delete_project(
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
