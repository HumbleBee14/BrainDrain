use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};

use crate::app_state::AppState;
use crate::auth::AuthenticatedUser;
use crate::dto::audit_log::{AuditLogFilterParams, AuditLogResponse};
use crate::dto::common::PaginatedResponse;
use crate::error::AppResult;

/// Audit log routes.
pub fn router() -> Router<AppState> {
    Router::new().route("/audit-logs", get(list_audit_logs))
}

/// GET /api/v1/audit-logs
///
/// Supports optional filtering by `resource_type` and `resource_id`.
async fn list_audit_logs(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Query(params): Query<AuditLogFilterParams>,
) -> AppResult<Json<PaginatedResponse<AuditLogResponse>>> {
    let limit = params.limit.min(100);
    let offset = params.offset;
    let repo = state.audit_log_repo();

    let (logs, total) = match (&params.resource_type, params.resource_id) {
        (Some(rt), Some(rid)) => tokio::try_join!(
            repo.list_by_resource(user.tenant_id, rt, rid, offset, limit),
            repo.count_by_resource(user.tenant_id, rt, rid),
        )?,
        _ => tokio::try_join!(
            repo.list_by_tenant(user.tenant_id, offset, limit),
            repo.count_by_tenant(user.tenant_id),
        )?,
    };

    Ok(Json(PaginatedResponse {
        data: logs.into_iter().map(Into::into).collect(),
        total,
        offset,
        limit,
    }))
}
