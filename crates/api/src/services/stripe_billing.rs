use async_trait::async_trait;
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::services::billing_provider::{BillingProvider, SubscriptionInfo};

/// Stripe price IDs for each plan tier.
struct StripePrices {
    starter: Option<String>,
    growth: Option<String>,
    pro: Option<String>,
}

/// Stripe billing provider using raw HTTP calls via `reqwest::Client`.
///
/// All API calls go to `https://api.stripe.com/v1/...` using
/// form-encoded POST bodies with Bearer token auth.
pub struct StripeBillingProvider {
    client: reqwest::Client,
    secret_key: String,
    prices: StripePrices,
}

impl StripeBillingProvider {
    pub fn new(
        client: reqwest::Client,
        secret_key: String,
        price_starter: Option<String>,
        price_growth: Option<String>,
        price_pro: Option<String>,
    ) -> Self {
        Self {
            client,
            secret_key,
            prices: StripePrices {
                starter: price_starter,
                growth: price_growth,
                pro: price_pro,
            },
        }
    }

    fn price_id_for_plan(&self, plan: &str) -> AppResult<String> {
        let price = match plan {
            "starter" => &self.prices.starter,
            "growth" => &self.prices.growth,
            "pro" => &self.prices.pro,
            _ => {
                return Err(AppError::BadRequest {
                    message: format!("Unknown plan: {plan}"),
                });
            }
        };

        price.clone().ok_or_else(|| AppError::BadRequest {
            message: format!("Price not configured for plan: {plan}"),
        })
    }
}

#[async_trait]
impl BillingProvider for StripeBillingProvider {
    async fn create_customer(&self, tenant_id: Uuid, email: &str, name: &str) -> AppResult<String> {
        let resp = self
            .client
            .post("https://api.stripe.com/v1/customers")
            .bearer_auth(&self.secret_key)
            .form(&[
                ("email", email),
                ("name", name),
                ("metadata[tenant_id]", &tenant_id.to_string()),
            ])
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("Stripe request failed: {e}")))?;

        let body: serde_json::Value = resp.json().await.map_err(|e| {
            AppError::Internal(anyhow::anyhow!("Failed to parse Stripe response: {e}"))
        })?;

        if let Some(err) = body.get("error") {
            let msg = err["message"].as_str().unwrap_or("Unknown Stripe error");
            return Err(AppError::Internal(anyhow::anyhow!("Stripe error: {msg}")));
        }

        body["id"].as_str().map(|s| s.to_string()).ok_or_else(|| {
            AppError::Internal(anyhow::anyhow!("Missing customer id in Stripe response"))
        })
    }

    async fn create_checkout_session(
        &self,
        customer_id: &str,
        plan: &str,
        success_url: &str,
        cancel_url: &str,
    ) -> AppResult<String> {
        let price_id = self.price_id_for_plan(plan)?;

        let resp = self
            .client
            .post("https://api.stripe.com/v1/checkout/sessions")
            .bearer_auth(&self.secret_key)
            .form(&[
                ("customer", customer_id),
                ("mode", "subscription"),
                ("line_items[0][price]", &price_id),
                ("line_items[0][quantity]", "1"),
                ("success_url", success_url),
                ("cancel_url", cancel_url),
            ])
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("Stripe request failed: {e}")))?;

        let body: serde_json::Value = resp.json().await.map_err(|e| {
            AppError::Internal(anyhow::anyhow!("Failed to parse Stripe response: {e}"))
        })?;

        if let Some(err) = body.get("error") {
            let msg = err["message"].as_str().unwrap_or("Unknown Stripe error");
            return Err(AppError::Internal(anyhow::anyhow!("Stripe error: {msg}")));
        }

        body["url"].as_str().map(|s| s.to_string()).ok_or_else(|| {
            AppError::Internal(anyhow::anyhow!("Missing checkout URL in Stripe response"))
        })
    }

    async fn create_portal_session(
        &self,
        customer_id: &str,
        return_url: &str,
    ) -> AppResult<String> {
        let resp = self
            .client
            .post("https://api.stripe.com/v1/billing_portal/sessions")
            .bearer_auth(&self.secret_key)
            .form(&[("customer", customer_id), ("return_url", return_url)])
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("Stripe request failed: {e}")))?;

        let body: serde_json::Value = resp.json().await.map_err(|e| {
            AppError::Internal(anyhow::anyhow!("Failed to parse Stripe response: {e}"))
        })?;

        if let Some(err) = body.get("error") {
            let msg = err["message"].as_str().unwrap_or("Unknown Stripe error");
            return Err(AppError::Internal(anyhow::anyhow!("Stripe error: {msg}")));
        }

        body["url"].as_str().map(|s| s.to_string()).ok_or_else(|| {
            AppError::Internal(anyhow::anyhow!("Missing portal URL in Stripe response"))
        })
    }

    async fn get_subscription(&self, subscription_id: &str) -> AppResult<SubscriptionInfo> {
        let url = format!("https://api.stripe.com/v1/subscriptions/{subscription_id}");

        let resp = self
            .client
            .get(&url)
            .bearer_auth(&self.secret_key)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("Stripe request failed: {e}")))?;

        let body: serde_json::Value = resp.json().await.map_err(|e| {
            AppError::Internal(anyhow::anyhow!("Failed to parse Stripe response: {e}"))
        })?;

        if let Some(err) = body.get("error") {
            let msg = err["message"].as_str().unwrap_or("Unknown Stripe error");
            return Err(AppError::Internal(anyhow::anyhow!("Stripe error: {msg}")));
        }

        let id = body["id"].as_str().unwrap_or("").to_string();
        let status = body["status"].as_str().unwrap_or("unknown").to_string();
        let current_period_end = body["current_period_end"].as_i64().unwrap_or(0);

        // Extract plan from the first subscription item's price metadata or product
        let plan = body["items"]["data"][0]["price"]["lookup_key"]
            .as_str()
            .or_else(|| body["items"]["data"][0]["price"]["metadata"]["plan"].as_str())
            .unwrap_or("starter")
            .to_string();

        Ok(SubscriptionInfo {
            id,
            status,
            plan,
            current_period_end,
        })
    }

    async fn cancel_subscription(&self, subscription_id: &str) -> AppResult<()> {
        let url = format!("https://api.stripe.com/v1/subscriptions/{subscription_id}");

        let resp = self
            .client
            .delete(&url)
            .bearer_auth(&self.secret_key)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("Stripe request failed: {e}")))?;

        let body: serde_json::Value = resp.json().await.map_err(|e| {
            AppError::Internal(anyhow::anyhow!("Failed to parse Stripe response: {e}"))
        })?;

        if let Some(err) = body.get("error") {
            let msg = err["message"].as_str().unwrap_or("Unknown Stripe error");
            return Err(AppError::Internal(anyhow::anyhow!("Stripe error: {msg}")));
        }

        Ok(())
    }
}

/// No-op billing provider for development mode when Stripe keys are not configured.
pub struct NoOpBillingProvider;

#[async_trait]
impl BillingProvider for NoOpBillingProvider {
    async fn create_customer(
        &self,
        _tenant_id: Uuid,
        _email: &str,
        _name: &str,
    ) -> AppResult<String> {
        Err(AppError::BadRequest {
            message: "Billing not configured".to_string(),
        })
    }

    async fn create_checkout_session(
        &self,
        _customer_id: &str,
        _plan: &str,
        _success_url: &str,
        _cancel_url: &str,
    ) -> AppResult<String> {
        Err(AppError::BadRequest {
            message: "Billing not configured".to_string(),
        })
    }

    async fn create_portal_session(
        &self,
        _customer_id: &str,
        _return_url: &str,
    ) -> AppResult<String> {
        Err(AppError::BadRequest {
            message: "Billing not configured".to_string(),
        })
    }

    async fn get_subscription(&self, _subscription_id: &str) -> AppResult<SubscriptionInfo> {
        Err(AppError::BadRequest {
            message: "Billing not configured".to_string(),
        })
    }

    async fn cancel_subscription(&self, _subscription_id: &str) -> AppResult<()> {
        Err(AppError::BadRequest {
            message: "Billing not configured".to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn noop_billing_never_fakes_success() {
        let p = NoOpBillingProvider;
        assert!(
            p.create_checkout_session("cus", "growth", "s", "c")
                .await
                .is_err()
        );
        assert!(p.create_portal_session("cus", "r").await.is_err());
        assert!(
            p.create_customer(Uuid::nil(), "e@x.com", "n")
                .await
                .is_err()
        );
    }
}
