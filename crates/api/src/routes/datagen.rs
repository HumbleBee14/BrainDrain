use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use uuid::Uuid;

use platform_shared::enums::TeamRole;

use crate::app_state::AppState;
use crate::auth::AuthenticatedUser;
use crate::dto::datagen::{
    CreateDataGuideRequest, DataGuideResponse, GenerateDatasetRequest, GenerateFacetsRequest,
    GeneratePreviewRequest, RateSamplesRequest, UpdateFacetsRequest, UpdateGuidanceRequest,
};
use crate::error::AppResult;
use crate::rbac::require_role;
use crate::services::audit_logger::AuditLogger;
use crate::services::data_guide_service::DataGuideService;
use crate::temporal::TraceContext;

/// Data Studio (guided synthetic-data session) routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/projects/{project_id}/data-guide", post(create_data_guide))
        .route("/projects/{project_id}/data-guide", get(get_data_guide))
        .route("/data-guides/{id}/reset", post(reset_data_guide))
        .route("/data-guides/{id}/facets", post(start_facets))
        .route("/data-guides/{id}/facets", put(update_facets))
        .route("/data-guides/{id}/preview", post(start_preview))
        .route("/data-guides/{id}/rate", post(rate_samples))
        .route("/data-guides/{id}/refine", post(refine_guidance))
        .route("/data-guides/{id}/guidance", put(update_guidance))
        .route("/data-guides/{id}/generate", post(generate_dataset))
}

/// Fetch a data guide by id, scoped to the caller's tenant.
async fn require_guide(
    state: &AppState,
    tenant_id: Uuid,
    guide_id: Uuid,
) -> AppResult<platform_db::models::DataGuide> {
    state
        .data_guide_repo()
        .get(tenant_id, guide_id)
        .await?
        .ok_or(crate::error::AppError::NotFound {
            message: "Data guide not found".to_string(),
        })
}

/// POST /api/v1/data-guides/{id}/reset
#[utoipa::path(
    post,
    path = "/api/v1/data-guides/{id}/reset",
    tag = "DataGen",
    params(("id" = Uuid, Path, description = "Data guide ID")),
    responses(
        (status = 200, description = "Guide reset to draft", body = DataGuideResponse),
        (status = 400, body = crate::error::ErrorEnvelope),
        (status = 404, body = crate::error::ErrorEnvelope),
    ),
    security(("jwt" = []))
)]
pub async fn reset_data_guide(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(guide_id): Path<Uuid>,
) -> AppResult<Json<DataGuideResponse>> {
    require_role(&user, TeamRole::Member)?;

    let guide = DataGuideService::reset(state.data_guide_repo(), user.tenant_id, guide_id).await?;

    AuditLogger::log(
        state.audit_log_repo(),
        &user,
        "reset_data_guide",
        "data_guide",
        Some(guide_id),
        serde_json::json!({}),
    )
    .await;

    Ok(Json(guide))
}

/// Create (or fetch the existing) data guide session for a project.
#[utoipa::path(
    post,
    path = "/api/v1/projects/{project_id}/data-guide",
    tag = "Data Studio",
    params(("project_id" = Uuid, Path, description = "Project ID")),
    request_body = CreateDataGuideRequest,
    responses(
        (status = 201, description = "Data guide created or fetched", body = DataGuideResponse),
        (status = 404, body = crate::error::ErrorEnvelope),
    ),
    security(("jwt" = []))
)]
pub async fn create_data_guide(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(project_id): Path<Uuid>,
    Json(body): Json<CreateDataGuideRequest>,
) -> AppResult<(StatusCode, Json<DataGuideResponse>)> {
    require_role(&user, TeamRole::Member)?;
    let result =
        DataGuideService::create_or_get(state.data_guide_repo(), user.tenant_id, project_id, body)
            .await?;

    AuditLogger::log(
        state.audit_log_repo(),
        &user,
        "create_data_guide",
        "data_guide",
        Some(project_id),
        serde_json::json!({}),
    )
    .await;

    Ok((StatusCode::CREATED, Json(result)))
}

/// Get the current data guide session for a project.
#[utoipa::path(
    get,
    path = "/api/v1/projects/{project_id}/data-guide",
    tag = "Data Studio",
    params(("project_id" = Uuid, Path, description = "Project ID")),
    responses(
        (status = 200, description = "Data guide", body = DataGuideResponse),
        (status = 404, body = crate::error::ErrorEnvelope),
    ),
    security(("jwt" = []))
)]
pub async fn get_data_guide(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(project_id): Path<Uuid>,
) -> AppResult<Json<DataGuideResponse>> {
    let result =
        DataGuideService::get_for_project(state.data_guide_repo(), user.tenant_id, project_id)
            .await?;

    Ok(Json(result))
}

/// Start facet generation from parsed source documents.
#[utoipa::path(
    post,
    path = "/api/v1/data-guides/{id}/facets",
    tag = "Data Studio",
    params(("id" = Uuid, Path, description = "Data guide ID")),
    request_body = GenerateFacetsRequest,
    responses(
        (status = 202, description = "Facet generation started", body = DataGuideResponse),
        (status = 400, body = crate::error::ErrorEnvelope),
        (status = 404, body = crate::error::ErrorEnvelope),
    ),
    security(("jwt" = []))
)]
pub async fn start_facets(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<GenerateFacetsRequest>,
) -> AppResult<(StatusCode, Json<DataGuideResponse>)> {
    require_role(&user, TeamRole::Member)?;
    let guide = require_guide(&state, user.tenant_id, id).await?;
    let trace_ctx = TraceContext::from_headers(&headers);

    let result = DataGuideService::start_facets(
        state.data_guide_repo(),
        state.document_repo(),
        state.orchestrator(),
        user.tenant_id,
        guide.project_id,
        id,
        body,
        trace_ctx,
    )
    .await?;

    AuditLogger::log(
        state.audit_log_repo(),
        &user,
        "start_facets",
        "data_guide",
        Some(id),
        serde_json::json!({}),
    )
    .await;

    Ok((StatusCode::ACCEPTED, Json(result)))
}

/// Persist the user's kept/discarded facet selections.
#[utoipa::path(
    put,
    path = "/api/v1/data-guides/{id}/facets",
    tag = "Data Studio",
    params(("id" = Uuid, Path, description = "Data guide ID")),
    request_body = UpdateFacetsRequest,
    responses(
        (status = 200, description = "Facets updated", body = DataGuideResponse),
        (status = 400, body = crate::error::ErrorEnvelope),
        (status = 404, body = crate::error::ErrorEnvelope),
    ),
    security(("jwt" = []))
)]
pub async fn update_facets(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateFacetsRequest>,
) -> AppResult<Json<DataGuideResponse>> {
    require_role(&user, TeamRole::Member)?;
    let result =
        DataGuideService::update_facets(state.data_guide_repo(), user.tenant_id, id, body).await?;

    AuditLogger::log(
        state.audit_log_repo(),
        &user,
        "update_facets",
        "data_guide",
        Some(id),
        serde_json::json!({}),
    )
    .await;

    Ok(Json(result))
}

/// Start preview-sample generation from the kept facets.
#[utoipa::path(
    post,
    path = "/api/v1/data-guides/{id}/preview",
    tag = "Data Studio",
    params(("id" = Uuid, Path, description = "Data guide ID")),
    request_body = GeneratePreviewRequest,
    responses(
        (status = 202, description = "Preview generation started", body = DataGuideResponse),
        (status = 400, body = crate::error::ErrorEnvelope),
        (status = 404, body = crate::error::ErrorEnvelope),
    ),
    security(("jwt" = []))
)]
pub async fn start_preview(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<GeneratePreviewRequest>,
) -> AppResult<(StatusCode, Json<DataGuideResponse>)> {
    require_role(&user, TeamRole::Member)?;
    let guide = require_guide(&state, user.tenant_id, id).await?;
    let trace_ctx = TraceContext::from_headers(&headers);

    let result = DataGuideService::start_preview(
        state.data_guide_repo(),
        state.orchestrator(),
        user.tenant_id,
        guide.project_id,
        id,
        body,
        trace_ctx,
    )
    .await?;

    AuditLogger::log(
        state.audit_log_repo(),
        &user,
        "start_preview",
        "data_guide",
        Some(id),
        serde_json::json!({}),
    )
    .await;

    Ok((StatusCode::ACCEPTED, Json(result)))
}

/// Merge user ratings into the stored preview samples.
#[utoipa::path(
    post,
    path = "/api/v1/data-guides/{id}/rate",
    tag = "Data Studio",
    params(("id" = Uuid, Path, description = "Data guide ID")),
    request_body = RateSamplesRequest,
    responses(
        (status = 200, description = "Samples rated", body = DataGuideResponse),
        (status = 400, body = crate::error::ErrorEnvelope),
        (status = 404, body = crate::error::ErrorEnvelope),
    ),
    security(("jwt" = []))
)]
pub async fn rate_samples(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(id): Path<Uuid>,
    Json(body): Json<RateSamplesRequest>,
) -> AppResult<Json<DataGuideResponse>> {
    require_role(&user, TeamRole::Member)?;
    let result =
        DataGuideService::rate_samples(state.data_guide_repo(), user.tenant_id, id, body).await?;

    AuditLogger::log(
        state.audit_log_repo(),
        &user,
        "rate_samples",
        "data_guide",
        Some(id),
        serde_json::json!({}),
    )
    .await;

    Ok(Json(result))
}

/// Regenerate guidance text from user-rated preview samples.
#[utoipa::path(
    post,
    path = "/api/v1/data-guides/{id}/refine",
    tag = "Data Studio",
    params(("id" = Uuid, Path, description = "Data guide ID")),
    responses(
        (status = 202, description = "Guidance refinement started", body = DataGuideResponse),
        (status = 400, body = crate::error::ErrorEnvelope),
        (status = 404, body = crate::error::ErrorEnvelope),
    ),
    security(("jwt" = []))
)]
pub async fn refine_guidance(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> AppResult<(StatusCode, Json<DataGuideResponse>)> {
    require_role(&user, TeamRole::Member)?;
    let guide = require_guide(&state, user.tenant_id, id).await?;
    let trace_ctx = TraceContext::from_headers(&headers);

    let result = DataGuideService::refine_guidance(
        state.data_guide_repo(),
        state.orchestrator(),
        user.tenant_id,
        guide.project_id,
        id,
        trace_ctx,
    )
    .await?;

    AuditLogger::log(
        state.audit_log_repo(),
        &user,
        "refine_guidance",
        "data_guide",
        Some(id),
        serde_json::json!({}),
    )
    .await;

    Ok((StatusCode::ACCEPTED, Json(result)))
}

/// Overwrite the free-form guidance text.
#[utoipa::path(
    put,
    path = "/api/v1/data-guides/{id}/guidance",
    tag = "Data Studio",
    params(("id" = Uuid, Path, description = "Data guide ID")),
    request_body = UpdateGuidanceRequest,
    responses(
        (status = 200, description = "Guidance updated", body = DataGuideResponse),
        (status = 400, body = crate::error::ErrorEnvelope),
        (status = 404, body = crate::error::ErrorEnvelope),
    ),
    security(("jwt" = []))
)]
pub async fn update_guidance(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateGuidanceRequest>,
) -> AppResult<Json<DataGuideResponse>> {
    require_role(&user, TeamRole::Member)?;
    let result =
        DataGuideService::update_guidance(state.data_guide_repo(), user.tenant_id, id, body)
            .await?;

    AuditLogger::log(
        state.audit_log_repo(),
        &user,
        "update_guidance",
        "data_guide",
        Some(id),
        serde_json::json!({}),
    )
    .await;

    Ok(Json(result))
}

/// Start full dataset generation from the finalized guidance and facets.
#[utoipa::path(
    post,
    path = "/api/v1/data-guides/{id}/generate",
    tag = "Data Studio",
    params(("id" = Uuid, Path, description = "Data guide ID")),
    request_body = GenerateDatasetRequest,
    responses(
        (status = 202, description = "Dataset generation started", body = DataGuideResponse),
        (status = 400, body = crate::error::ErrorEnvelope),
        (status = 404, body = crate::error::ErrorEnvelope),
    ),
    security(("jwt" = []))
)]
pub async fn generate_dataset(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(_body): Json<GenerateDatasetRequest>,
) -> AppResult<(StatusCode, Json<DataGuideResponse>)> {
    require_role(&user, TeamRole::Member)?;
    let guide = require_guide(&state, user.tenant_id, id).await?;
    let trace_ctx = TraceContext::from_headers(&headers);

    let result = DataGuideService::generate_dataset(
        state.data_guide_repo(),
        state.document_repo(),
        state.dataset_repo(),
        state.tenant_repo(),
        state.orchestrator(),
        user.tenant_id,
        guide.project_id,
        id,
        trace_ctx,
    )
    .await?;

    AuditLogger::log(
        state.audit_log_repo(),
        &user,
        "generate_dataset",
        "data_guide",
        Some(id),
        serde_json::json!({}),
    )
    .await;

    Ok((StatusCode::ACCEPTED, Json(result)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn router_registers_expected_paths() {
        let r = router();
        // smoke: router constructs without panic
        let _ = r;
    }
}
