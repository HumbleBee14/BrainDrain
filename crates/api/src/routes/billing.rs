use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};

use crate::app_state::AppState;
use crate::auth::AuthenticatedUser;
use crate::dto::billing::BillingEventResponse;
use crate::dto::common::{PaginatedResponse, PaginationParams};
use crate::error::AppResult;
use crate::repositories::billing_event_repo::BillingEventRepo;

/// Billing routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/billing/events", get(list_billing_events))
        .route("/billing/usage", get(get_usage_summary))
}

/// GET /api/v1/billing/events
async fn list_billing_events(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Query(pagination): Query<PaginationParams>,
) -> AppResult<Json<PaginatedResponse<BillingEventResponse>>> {
    let offset = pagination.offset;
    let limit = pagination.limit.min(100);

    let (events, total) = tokio::try_join!(
        BillingEventRepo::list_by_tenant(state.db(), user.tenant_id, offset, limit),
        BillingEventRepo::count_by_tenant(state.db(), user.tenant_id),
    )?;

    Ok(Json(PaginatedResponse {
        data: events.into_iter().map(Into::into).collect(),
        total,
        offset,
        limit,
    }))
}

/// GET /api/v1/billing/usage
async fn get_usage_summary(
    State(state): State<AppState>,
    user: AuthenticatedUser,
) -> AppResult<Json<TenantUsageSummary>> {
    let (events, total) = tokio::try_join!(
        BillingEventRepo::list_by_tenant(state.db(), user.tenant_id, 0, 1000),
        BillingEventRepo::count_by_tenant(state.db(), user.tenant_id),
    )?;

    let mut total_tokens_in: i64 = 0;
    let mut total_tokens_out: i64 = 0;
    let mut total_cost_usd: f64 = 0.0;

    for event in &events {
        total_tokens_in += event.tokens_in.unwrap_or(0);
        total_tokens_out += event.tokens_out.unwrap_or(0);
        total_cost_usd += event.cost_usd;
    }

    Ok(Json(TenantUsageSummary {
        total_events: total,
        total_tokens_in,
        total_tokens_out,
        total_cost_usd,
    }))
}

#[derive(Debug, serde::Serialize)]
struct TenantUsageSummary {
    total_events: i64,
    total_tokens_in: i64,
    total_tokens_out: i64,
    total_cost_usd: f64,
}
