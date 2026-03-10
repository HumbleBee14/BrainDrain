use std::convert::Infallible;
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, Sse};
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
use crate::temporal::TraceContext;

/// Export routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/models/{model_id}/exports",
            post(create_export).get(list_exports),
        )
        .route("/exports/{export_id}/download", get(download_export))
        .route(
            "/models/{model_id}/exports/stream",
            get(stream_export_status),
        )
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
    headers: HeaderMap,
    Path(model_id): Path<Uuid>,
    Json(body): Json<ExportRequest>,
) -> AppResult<(StatusCode, Json<ExportResponse>)> {
    require_role(&user, TeamRole::Member)?;
    let trace_ctx = TraceContext::from_headers(&headers);
    let result = ExportService::create(
        state.export_repo(),
        state.model_repo(),
        state.orchestrator(),
        user.tenant_id,
        model_id,
        &body.quant_type,
        trace_ctx,
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

/// GET /api/v1/models/:model_id/exports/stream
///
/// SSE endpoint that pushes export status changes for a model's exports.
pub async fn stream_export_status(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(model_id): Path<Uuid>,
) -> AppResult<Sse<impl futures::Stream<Item = Result<Event, Infallible>>>> {
    let initial = ExportService::list(state.export_repo(), user.tenant_id, model_id).await?;
    let tenant_id = user.tenant_id;

    let stream = async_stream::stream! {
        let mut last_json = serde_json::to_string(&initial).unwrap_or_default();

        if let Ok(json) = serde_json::to_string(&initial) {
            yield Ok(Event::default().data(json).event("status"));
        }

        let all_terminal = |exports: &[ExportResponse]| -> bool {
            exports.iter().all(|e| matches!(e.status.as_str(), "completed" | "failed"))
        };

        if all_terminal(&initial) {
            return;
        }

        loop {
            tokio::time::sleep(Duration::from_secs(3)).await;

            match ExportService::list(state.export_repo(), tenant_id, model_id).await {
                Ok(exports) => {
                    let json = serde_json::to_string(&exports).unwrap_or_default();
                    if json != last_json {
                        last_json = json.clone();
                        yield Ok(Event::default().data(json).event("status"));
                    } else {
                        yield Ok(Event::default().comment("heartbeat"));
                    }
                    if all_terminal(&exports) {
                        return;
                    }
                }
                Err(_) => {
                    yield Ok(Event::default().comment("heartbeat"));
                }
            }
        }
    };

    Ok(Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    ))
}
