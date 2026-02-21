use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::app_state::AppState;
use crate::services::plan_service::PlanLimits;

type HmacSha256 = Hmac<Sha256>;

/// Stripe webhook router. Mounted OUTSIDE /api/v1 — no Clerk auth.
pub fn router() -> Router<AppState> {
    Router::new().route("/api/webhooks/stripe", post(handle_stripe_webhook))
}

async fn handle_stripe_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, StatusCode> {
    let signature = headers
        .get("stripe-signature")
        .and_then(|v| v.to_str().ok())
        .ok_or(StatusCode::BAD_REQUEST)?;

    let webhook_secret = state
        .config()
        .stripe_webhook_secret
        .as_deref()
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;

    verify_stripe_signature(&body, signature, webhook_secret)
        .map_err(|_| StatusCode::BAD_REQUEST)?;

    let event: serde_json::Value =
        serde_json::from_slice(&body).map_err(|_| StatusCode::BAD_REQUEST)?;

    let event_type = event["type"].as_str().unwrap_or("");

    match event_type {
        "checkout.session.completed" => handle_checkout_completed(&state, &event).await,
        "customer.subscription.updated" => handle_subscription_updated(&state, &event).await,
        "customer.subscription.deleted" => handle_subscription_deleted(&state, &event).await,
        _ => {
            tracing::debug!(event_type, "Ignoring unhandled Stripe event");
            Ok(StatusCode::OK)
        }
    }
}

/// Verify Stripe webhook signature using HMAC-SHA256.
///
/// Stripe sends a `Stripe-Signature` header with format:
/// `t=<timestamp>,v1=<signature>`
///
/// The signed payload is `<timestamp>.<raw body>`.
fn verify_stripe_signature(payload: &[u8], signature_header: &str, secret: &str) -> Result<(), ()> {
    let mut timestamp = None;
    let mut expected_sig = None;

    for part in signature_header.split(',') {
        let part = part.trim();
        if let Some(t) = part.strip_prefix("t=") {
            timestamp = Some(t);
        } else if let Some(v1) = part.strip_prefix("v1=") {
            expected_sig = Some(v1);
        }
    }

    let timestamp = timestamp.ok_or(())?;
    let expected_sig = expected_sig.ok_or(())?;

    // Build signed payload: "{timestamp}.{body}"
    let mut signed_payload = Vec::with_capacity(timestamp.len() + 1 + payload.len());
    signed_payload.extend_from_slice(timestamp.as_bytes());
    signed_payload.push(b'.');
    signed_payload.extend_from_slice(payload);

    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).map_err(|_| ())?;
    mac.update(&signed_payload);
    let result = mac.finalize().into_bytes();
    let computed = hex::encode(result);

    if computed != expected_sig {
        tracing::warn!("Stripe webhook signature verification failed");
        return Err(());
    }

    Ok(())
}

/// Handle `checkout.session.completed` — link Stripe customer + subscription to tenant.
async fn handle_checkout_completed(
    state: &AppState,
    event: &serde_json::Value,
) -> Result<StatusCode, StatusCode> {
    let obj = &event["data"]["object"];
    let customer_id = obj["customer"].as_str().ok_or(StatusCode::BAD_REQUEST)?;
    let subscription_id = obj["subscription"]
        .as_str()
        .ok_or(StatusCode::BAD_REQUEST)?;

    let tenant_repo = state.tenant_repo();

    // Find tenant by Stripe customer ID
    let tenant = tenant_repo
        .get_by_stripe_customer(customer_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let tenant = match tenant {
        Some(t) => t,
        None => {
            tracing::warn!(customer_id, "No tenant found for Stripe customer");
            return Ok(StatusCode::OK);
        }
    };

    // Fetch subscription details to determine the plan
    let billing = state.billing_provider();
    let sub_info = billing
        .get_subscription(subscription_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let limits = PlanLimits::for_plan(&sub_info.plan);
    let limits_json = serde_json::to_value(&limits).unwrap_or_default();

    tenant_repo
        .update_subscription(tenant.id, subscription_id, &sub_info.plan, limits_json)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    tracing::info!(
        tenant_id = %tenant.id,
        plan = %sub_info.plan,
        subscription_id,
        "Checkout completed — subscription activated"
    );

    Ok(StatusCode::OK)
}

/// Handle `customer.subscription.updated` — sync plan changes.
async fn handle_subscription_updated(
    state: &AppState,
    event: &serde_json::Value,
) -> Result<StatusCode, StatusCode> {
    let obj = &event["data"]["object"];
    let customer_id = obj["customer"].as_str().ok_or(StatusCode::BAD_REQUEST)?;
    let subscription_id = obj["id"].as_str().ok_or(StatusCode::BAD_REQUEST)?;

    let tenant_repo = state.tenant_repo();

    let tenant = tenant_repo
        .get_by_stripe_customer(customer_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let tenant = match tenant {
        Some(t) => t,
        None => {
            tracing::warn!(customer_id, "No tenant found for Stripe customer");
            return Ok(StatusCode::OK);
        }
    };

    let billing = state.billing_provider();
    let sub_info = billing
        .get_subscription(subscription_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let limits = PlanLimits::for_plan(&sub_info.plan);
    let limits_json = serde_json::to_value(&limits).unwrap_or_default();

    tenant_repo
        .update_subscription(tenant.id, subscription_id, &sub_info.plan, limits_json)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    tracing::info!(
        tenant_id = %tenant.id,
        plan = %sub_info.plan,
        "Subscription updated"
    );

    Ok(StatusCode::OK)
}

/// Handle `customer.subscription.deleted` — downgrade to starter.
async fn handle_subscription_deleted(
    state: &AppState,
    event: &serde_json::Value,
) -> Result<StatusCode, StatusCode> {
    let obj = &event["data"]["object"];
    let customer_id = obj["customer"].as_str().ok_or(StatusCode::BAD_REQUEST)?;

    let tenant_repo = state.tenant_repo();

    let tenant = tenant_repo
        .get_by_stripe_customer(customer_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let tenant = match tenant {
        Some(t) => t,
        None => {
            tracing::warn!(customer_id, "No tenant found for Stripe customer");
            return Ok(StatusCode::OK);
        }
    };

    let limits = PlanLimits::for_plan("starter");
    let limits_json = serde_json::to_value(&limits).unwrap_or_default();

    tenant_repo
        .update_subscription(tenant.id, "", "starter", limits_json)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    tracing::info!(
        tenant_id = %tenant.id,
        "Subscription deleted — downgraded to starter"
    );

    Ok(StatusCode::OK)
}
