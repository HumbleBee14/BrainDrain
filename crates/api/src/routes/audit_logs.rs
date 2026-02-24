use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use platform_shared::enums::TeamRole;

use crate::app_state::AppState;
use crate::auth::AuthenticatedUser;
use crate::dto::audit_log::{AuditLogFilterParams, AuditLogResponse};
use crate::dto::common::PaginatedResponse;
use crate::error::AppResult;
use crate::rbac::require_role;

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
        ("action" = Option<String>, Query, description = "Filter by action"),
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
    require_role(&user, TeamRole::Admin)?;

    let limit = params.limit.clamp(1, 100);
    let offset = params.offset.max(0);
    let repo = state.audit_log_repo();

    let (logs, total) = match (&params.resource_type, params.resource_id) {
        // Exact resource filter takes priority
        (Some(rt), Some(rid)) => tokio::try_join!(
            repo.list_by_resource(user.tenant_id, rt, rid, offset, limit),
            repo.count_by_resource(user.tenant_id, rt, rid),
        )?,
        // Use filtered query when action or resource_type provided
        _ if params.action.is_some() || params.resource_type.is_some() => tokio::try_join!(
            repo.list_filtered(
                user.tenant_id,
                params.action.as_deref(),
                params.resource_type.as_deref(),
                offset,
                limit,
            ),
            repo.count_filtered(
                user.tenant_id,
                params.action.as_deref(),
                params.resource_type.as_deref(),
            ),
        )?,
        // Unfiltered
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
