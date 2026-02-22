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

/// List audit logs with optional filtering.
#[utoipa::path(
    get,
    path = "/api/v1/audit-logs",
    tag = "Audit Logs",
    params(
        ("offset" = Option<i64>, Query, description = "Pagination offset"),
        ("limit" = Option<i64>, Query, description = "Pagination limit"),
        ("resource_type" = Option<String>, Query, description = "Filter by resource type"),
        ("resource_id" = Option<uuid::Uuid>, Query, description = "Filter by resource ID"),
    ),
    responses(
        (status = 200, description = "Audit log entries", body = inline(PaginatedResponse<AuditLogResponse>)),
    ),
    security(("jwt" = []))
)]
pub async fn list_audit_logs(
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
