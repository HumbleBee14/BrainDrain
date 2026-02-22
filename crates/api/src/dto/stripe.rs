use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::ToSchema;

/// Current subscription details returned to the frontend.
#[derive(Debug, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct SubscriptionResponse {
    pub id: String,
    pub status: String,
    pub plan: String,
    pub current_period_end: String,
}

/// Request body for creating a Stripe checkout session.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateCheckoutRequest {
    pub plan: String,
    pub success_url: String,
    pub cancel_url: String,
}

/// Checkout session URL returned after creation.
#[derive(Debug, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct CheckoutSessionResponse {
    pub url: String,
}

/// Request body for creating a Stripe customer portal session.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreatePortalRequest {
    pub return_url: String,
}

/// Portal session URL returned after creation.
#[derive(Debug, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct PortalSessionResponse {
    pub url: String,
}
