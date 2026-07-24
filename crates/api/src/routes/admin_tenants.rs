use axum::extract::{Path, State};
use axum::routing::delete;
use axum::{Json, Router};
use uuid::Uuid;

use crate::app_state::AppState;
use crate::auth::AuthenticatedUser;
use crate::dto::admin::TenantErasureSummary;
use crate::error::AppResult;
use crate::rbac::require_platform_admin;
use crate::services::audit_logger::AuditLogger;
use crate::services::tenant_erasure_service::TenantErasureService;

pub fn router() -> Router<AppState> {
    Router::new().route("/admin/tenants/{tenant_id}", delete(erase_tenant))
}

#[utoipa::path(
    delete,
    path = "/api/v1/admin/tenants/{tenant_id}",
    tag = "Admin",
    responses((status = 200, body = TenantErasureSummary)),
    security(("jwt" = []))
)]
pub async fn erase_tenant(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Path(tenant_id): Path<Uuid>,
) -> AppResult<Json<TenantErasureSummary>> {
    require_platform_admin(&user, state.config())?;

    let summary =
        TenantErasureService::erase_tenant(state.tenant_repo(), state.storage(), tenant_id).await?;

    AuditLogger::log(
        state.audit_log_repo(),
        &user,
        "erase_tenant",
        "tenant",
        Some(tenant_id),
        serde_json::json!({ "objects_deleted": summary.objects_deleted }),
    )
    .await;

    Ok(Json(summary))
}
