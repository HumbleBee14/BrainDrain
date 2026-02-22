use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use uuid::Uuid;

use platform_shared::enums::TeamRole;

use crate::app_state::AppState;
use crate::auth::AuthenticatedUser;
use crate::dto::export::{ExportDownloadResponse, ExportRequest, ExportResponse};
use crate::error::AppResult;
use crate::rbac::require_role;
use crate::services::audit_logger::AuditLogger;
use crate::services::export_service::ExportService;

/// Export routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/models/{model_id}/exports",
            post(create_export).get(list_exports),
        )
        .route("/exports/{export_id}/download", get(download_export))
}

/// POST /api/v1/models/:model_id/exports
#[utoipa::path(
    post,
    path = "/api/v1/models/{model_id}/exports",
    tag = "Exports",
    params(
        ("model_id" = Uuid, Path, description = "Model ID")
    ),
    request_body = ExportRequest,
    responses(
        (status = 201, description = "Export created", body = ExportResponse),
        (status = 401, description = "Unauthorized"),
    ),
    security(("jwt" = []))
)]
pub async fn create_export(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(model_id): Path<Uuid>,
    Json(body): Json<ExportRequest>,
) -> AppResult<(StatusCode, Json<ExportResponse>)> {
    require_role(&user, TeamRole::Member)?;
    let result = ExportService::create(
        state.export_repo(),
        state.model_repo(),
        state.orchestrator(),
        user.tenant_id,
        model_id,
        &body.quant_type,
    )
    .await?;

    AuditLogger::log(
        state.audit_log_repo(),
        &user,
        "create",
        "export",
        result.id.parse().ok(),
        serde_json::json!({"model_id": model_id.to_string(), "quant_type": body.quant_type}),
    )
    .await;

    Ok((StatusCode::CREATED, Json(result)))
}

/// GET /api/v1/models/:model_id/exports
#[utoipa::path(
    get,
    path = "/api/v1/models/{model_id}/exports",
    tag = "Exports",
    params(
        ("model_id" = Uuid, Path, description = "Model ID")
    ),
    responses(
        (status = 200, description = "List of exports", body = Vec<ExportResponse>),
        (status = 401, description = "Unauthorized"),
    ),
    security(("jwt" = []))
)]
pub async fn list_exports(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(model_id): Path<Uuid>,
) -> AppResult<Json<Vec<ExportResponse>>> {
    let exports = ExportService::list(state.export_repo(), user.tenant_id, model_id).await?;
    Ok(Json(exports))
}

/// GET /api/v1/exports/:export_id/download
#[utoipa::path(
    get,
    path = "/api/v1/exports/{export_id}/download",
    tag = "Exports",
    params(
        ("export_id" = Uuid, Path, description = "Export ID")
    ),
    responses(
        (status = 200, description = "Export download URL", body = ExportDownloadResponse),
        (status = 401, description = "Unauthorized"),
    ),
    security(("jwt" = []))
)]
pub async fn download_export(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(export_id): Path<Uuid>,
) -> AppResult<Json<ExportDownloadResponse>> {
    let (download_url, file_size_bytes, filename) = ExportService::download_url(
        state.export_repo(),
        state.storage(),
        user.tenant_id,
        export_id,
    )
    .await?;

    Ok(Json(ExportDownloadResponse {
        download_url,
        file_size_bytes,
        filename,
    }))
}
