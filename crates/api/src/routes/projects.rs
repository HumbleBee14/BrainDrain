use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use uuid::Uuid;

use crate::app_state::AppState;
use crate::auth::AuthenticatedUser;
use crate::dto::common::{PaginatedResponse, PaginationParams};
use crate::dto::project::{CreateProjectRequest, ProjectResponse, UpdateProjectRequest};
use crate::error::AppResult;
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
    let project = ProjectService::create(state.db(), user.tenant_id, body).await?;
    Ok((StatusCode::CREATED, Json(project)))
}

async fn list_projects(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Query(params): Query<PaginationParams>,
) -> AppResult<Json<PaginatedResponse<ProjectResponse>>> {
    let result =
        ProjectService::list(state.db(), user.tenant_id, params.offset, params.limit).await?;
    Ok(Json(result))
}

async fn get_project(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<ProjectResponse>> {
    let project = ProjectService::get(state.db(), user.tenant_id, id).await?;
    Ok(Json(project))
}

async fn update_project(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateProjectRequest>,
) -> AppResult<Json<ProjectResponse>> {
    let project = ProjectService::update(state.db(), user.tenant_id, id, body).await?;
    Ok(Json(project))
}

async fn delete_project(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> AppResult<StatusCode> {
    ProjectService::delete(state.db(), user.tenant_id, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
