use async_trait::async_trait;
use uuid::Uuid;

use crate::error::AppResult;

/// Subscription information returned by the billing provider.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SubscriptionInfo {
    pub id: String,
    pub status: String,
    pub plan: String,
    pub current_period_end: i64,
}

/// Vendor-agnostic billing provider trait.
///
/// Implementations can be swapped (Stripe, Paddle, etc.) without
/// changing business logic. Use `NoOpBillingProvider` for dev mode.
#[allow(dead_code)]
#[async_trait]
pub trait BillingProvider: Send + Sync {
    async fn create_customer(&self, tenant_id: Uuid, email: &str, name: &str) -> AppResult<String>;

    async fn create_checkout_session(
        &self,
        customer_id: &str,
        plan: &str,
        success_url: &str,
        cancel_url: &str,
    ) -> AppResult<String>;

    async fn create_portal_session(&self, customer_id: &str, return_url: &str)
    -> AppResult<String>;

    async fn get_subscription(&self, subscription_id: &str) -> AppResult<SubscriptionInfo>;

    async fn cancel_subscription(&self, subscription_id: &str) -> AppResult<()>;
}
