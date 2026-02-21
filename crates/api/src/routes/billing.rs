use axum::extract::{Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};

use platform_shared::enums::TeamRole;

use crate::app_state::AppState;
use crate::auth::AuthenticatedUser;
use crate::dto::billing::BillingEventResponse;
use crate::dto::common::{PaginatedResponse, PaginationParams};
use crate::dto::stripe::{
    CheckoutSessionResponse, CreateCheckoutRequest, CreatePortalRequest, PortalSessionResponse,
    SubscriptionResponse,
};
use crate::error::{AppError, AppResult};
use crate::rbac::require_role;
use crate::services::plan_service::{PlanLimits, PlanService};

/// Billing routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/billing/events", get(list_billing_events))
        .route("/billing/usage", get(get_usage_summary))
        .route("/billing/checkout", post(create_checkout))
        .route("/billing/portal", post(create_portal_session))
        .route("/billing/subscription", get(get_subscription))
        .route("/billing/limits", get(get_plan_limits))
}

/// GET /api/v1/billing/events
async fn list_billing_events(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Query(pagination): Query<PaginationParams>,
) -> AppResult<Json<PaginatedResponse<BillingEventResponse>>> {
    require_role(&user, TeamRole::Admin)?;
    let offset = pagination.offset;
    let limit = pagination.limit.min(100);

    let repo = state.billing_event_repo();
    let (events, total) = tokio::try_join!(
        repo.list_by_tenant(user.tenant_id, offset, limit),
        repo.count_by_tenant(user.tenant_id),
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
    require_role(&user, TeamRole::Admin)?;
    let repo = state.billing_event_repo();
    let (totals, total_events) = tokio::try_join!(
        repo.usage_totals(user.tenant_id),
        repo.count_by_tenant(user.tenant_id),
    )?;

    let (total_cost_usd, total_tokens_in, total_tokens_out) = totals;

    Ok(Json(TenantUsageSummary {
        total_events,
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

/// POST /api/v1/billing/checkout — create a Stripe checkout session.
async fn create_checkout(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Json(body): Json<CreateCheckoutRequest>,
) -> AppResult<Json<CheckoutSessionResponse>> {
    require_role(&user, TeamRole::Owner)?;

    let tenant_repo = state.tenant_repo();
    let tenant = tenant_repo
        .get_by_id(user.tenant_id)
        .await?
        .ok_or(AppError::NotFound {
            message: "Tenant not found".into(),
        })?;

    // Ensure tenant has a Stripe customer ID; create one if needed
    let customer_id = match tenant.stripe_customer_id {
        Some(id) => id,
        None => {
            let billing = state.billing_provider();
            let id = billing
                .create_customer(user.tenant_id, &user.user_id, &tenant.name)
                .await?;
            tenant_repo
                .update_stripe_customer(user.tenant_id, &id)
                .await?;
            id
        }
    };

    let billing = state.billing_provider();
    let url = billing
        .create_checkout_session(
            &customer_id,
            &body.plan,
            &body.success_url,
            &body.cancel_url,
        )
        .await?;

    Ok(Json(CheckoutSessionResponse { url }))
}

/// POST /api/v1/billing/portal — create a Stripe customer portal session.
async fn create_portal_session(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Json(body): Json<CreatePortalRequest>,
) -> AppResult<Json<PortalSessionResponse>> {
    require_role(&user, TeamRole::Owner)?;

    let tenant =
        state
            .tenant_repo()
            .get_by_id(user.tenant_id)
            .await?
            .ok_or(AppError::NotFound {
                message: "Tenant not found".into(),
            })?;

    let customer_id = tenant.stripe_customer_id.ok_or(AppError::BadRequest {
        message: "No billing account configured. Please subscribe first.".into(),
    })?;

    let billing = state.billing_provider();
    let url = billing
        .create_portal_session(&customer_id, &body.return_url)
        .await?;

    Ok(Json(PortalSessionResponse { url }))
}

/// GET /api/v1/billing/subscription — get current subscription info.
async fn get_subscription(
    State(state): State<AppState>,
    user: AuthenticatedUser,
) -> AppResult<Json<SubscriptionResponse>> {
    require_role(&user, TeamRole::Admin)?;

    let tenant =
        state
            .tenant_repo()
            .get_by_id(user.tenant_id)
            .await?
            .ok_or(AppError::NotFound {
                message: "Tenant not found".into(),
            })?;

    let subscription_id = tenant
        .stripe_subscription_id
        .filter(|s| !s.is_empty())
        .ok_or(AppError::NotFound {
            message: "No active subscription".into(),
        })?;

    // Serve from DB (updated by webhooks) instead of live Stripe API call.
    Ok(Json(SubscriptionResponse {
        id: subscription_id,
        status: "active".to_string(), // Webhook keeps this in sync
        plan: tenant.plan.clone(),
        current_period_end: String::new(), // Not stored locally — use Stripe portal for details
    }))
}

/// GET /api/v1/billing/limits — get current plan limits.
async fn get_plan_limits(
    State(state): State<AppState>,
    user: AuthenticatedUser,
) -> AppResult<Json<PlanLimits>> {
    require_role(&user, TeamRole::Viewer)?;

    let limits = PlanService::get_limits(state.tenant_repo(), user.tenant_id).await?;

    Ok(Json(limits))
}
