use std::net::IpAddr;

use uuid::Uuid;

use crate::dto::notification::{
    NotificationDeliveryResponse, NotificationPreferenceResponse, PreferenceUpdate,
};
use crate::error::{AppError, AppResult};
use crate::repositories::traits::NotificationRepository;

/// Returns `true` if the IP address is in a private, reserved, or internal range.
fn is_private_ip(ip: &IpAddr) -> bool {
    if ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() {
        return true;
    }
    match ip {
        IpAddr::V4(v4) => {
            let octets = v4.octets();
            // 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16, 169.254.0.0/16 (link-local)
            octets[0] == 10
                || (octets[0] == 172 && (16..=31).contains(&octets[1]))
                || (octets[0] == 192 && octets[1] == 168)
                || (octets[0] == 169 && octets[1] == 254)
        }
        IpAddr::V6(v6) => {
            let segments = v6.segments();
            // fc00::/7 — unique local addresses
            (segments[0] & 0xfe00) == 0xfc00
                // fe80::/10 — link-local
                || (segments[0] & 0xffc0) == 0xfe80
                // ::1/128 — loopback (already covered above, defensive)
                || v6.is_loopback()
                // ::ffff:0:0/96 — IPv4-mapped (check the embedded v4)
                || segments[0..5] == [0, 0, 0, 0, 0]
                    && segments[5] == 0xffff
                    && is_private_ip(&IpAddr::V4(std::net::Ipv4Addr::new(
                        (segments[6] >> 8) as u8,
                        segments[6] as u8,
                        (segments[7] >> 8) as u8,
                        segments[7] as u8,
                    )))
        }
    }
}

/// Reject webhook URLs that point to private/internal networks (SSRF protection).
/// Uses async DNS resolution to avoid blocking the Tokio runtime.
async fn is_safe_webhook_url(url: &str) -> bool {
    let parsed = match reqwest::Url::parse(url) {
        Ok(u) => u,
        Err(_) => return false,
    };

    // Only allow HTTPS or HTTP schemes
    if parsed.scheme() != "https" && parsed.scheme() != "http" {
        return false;
    }

    let host = match parsed.host_str() {
        Some(h) => h,
        None => return false,
    };

    // Async DNS resolution — avoids blocking the Tokio runtime
    let port = parsed
        .port()
        .unwrap_or(if parsed.scheme() == "https" { 443 } else { 80 });
    let addrs = match tokio::net::lookup_host(format!("{host}:{port}")).await {
        Ok(a) => a,
        Err(_) => return false,
    };

    let mut found_any = false;
    for addr in addrs {
        found_any = true;
        if is_private_ip(&addr.ip()) {
            return false;
        }
    }

    // Reject if DNS resolved to zero addresses
    found_any
}

/// Handles notification preference management and delivery dispatch.
pub struct NotificationService;

impl NotificationService {
    /// Emit a notification by durably enqueuing a delivery row per enabled
    /// preference. The background delivery worker dispatches them (webhook,
    /// email); in-app rows are read directly by the client. Best-effort: a
    /// failure is logged and never fails the primary operation.
    pub async fn notify(
        repo: &dyn NotificationRepository,
        tenant_id: Uuid,
        event_type: &str,
        payload: serde_json::Value,
    ) {
        let prefs = match repo.get_enabled_preferences(tenant_id, event_type).await {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(error = %e, "Failed to load notification preferences");
                return;
            }
        };

        for pref in prefs {
            if let Err(e) = repo
                .create_delivery(
                    tenant_id,
                    pref.id,
                    event_type,
                    &pref.channel,
                    payload.clone(),
                )
                .await
            {
                tracing::warn!(
                    tenant_id = %tenant_id,
                    channel = %pref.channel,
                    error = %e,
                    "Failed to enqueue notification delivery"
                );
            }
        }
    }

    /// Send a test webhook to validate the configured URL.
    /// Creates a delivery record so the result is visible in delivery history.
    pub async fn test_webhook(
        repo: &dyn NotificationRepository,
        http_client: &reqwest::Client,
        tenant_id: Uuid,
        preference_id: Uuid,
    ) -> AppResult<NotificationDeliveryResponse> {
        let pref =
            repo.get_preference(tenant_id, preference_id)
                .await?
                .ok_or(AppError::NotFound {
                    message: format!("Notification preference {} not found", preference_id),
                })?;

        if pref.channel != "webhook" {
            return Err(AppError::BadRequest {
                message: "Test is only available for webhook channels".into(),
            });
        }

        let url = pref
            .config
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or(AppError::BadRequest {
                message: "No webhook URL configured for this preference".into(),
            })?;

        if !is_safe_webhook_url(url).await {
            return Err(AppError::BadRequest {
                message: "Webhook URL targets a private or internal network".into(),
            });
        }

        let payload = serde_json::json!({
            "event": "test",
            "message": "This is a test webhook delivery",
            "timestamp": chrono::Utc::now().to_rfc3339(),
        });

        let delivery = repo
            .create_delivery(tenant_id, preference_id, "test", "webhook", payload.clone())
            .await?;

        let result = http_client
            .post(url)
            .json(&payload)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await;

        match result {
            Ok(res) if res.status().is_success() => {
                let _ = repo
                    .update_delivery_status(tenant_id, delivery.id, "sent", None)
                    .await;
            }
            Ok(res) => {
                let msg = format!("HTTP {}", res.status());
                let _ = repo
                    .update_delivery_status(tenant_id, delivery.id, "failed", Some(&msg))
                    .await;
            }
            Err(e) => {
                let _ = repo
                    .update_delivery_status(tenant_id, delivery.id, "failed", Some(&e.to_string()))
                    .await;
            }
        }

        // Re-fetch to get updated status
        let updated = repo
            .get_delivery(tenant_id, delivery.id)
            .await?
            .unwrap_or(delivery);

        Ok(updated.into())
    }

    /// Retry a failed delivery by re-sending the original payload to the webhook URL.
    pub async fn retry_delivery(
        repo: &dyn NotificationRepository,
        http_client: &reqwest::Client,
        tenant_id: Uuid,
        delivery_id: Uuid,
    ) -> AppResult<NotificationDeliveryResponse> {
        let delivery =
            repo.get_delivery(tenant_id, delivery_id)
                .await?
                .ok_or(AppError::NotFound {
                    message: format!("Notification delivery {} not found", delivery_id),
                })?;

        if delivery.status != "failed" {
            return Err(AppError::BadRequest {
                message: format!(
                    "Only failed deliveries can be retried. Current status: {}",
                    delivery.status
                ),
            });
        }

        let pref = repo
            .get_preference(tenant_id, delivery.preference_id)
            .await?
            .ok_or(AppError::NotFound {
                message: format!(
                    "Notification preference {} not found",
                    delivery.preference_id
                ),
            })?;

        let url = pref
            .config
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or(AppError::BadRequest {
                message: "No webhook URL configured for this preference".into(),
            })?;

        if !is_safe_webhook_url(url).await {
            return Err(AppError::BadRequest {
                message: "Webhook URL targets a private or internal network".into(),
            });
        }

        let result = http_client
            .post(url)
            .json(&delivery.payload)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await;

        match result {
            Ok(res) if res.status().is_success() => {
                let _ = repo
                    .update_delivery_status(tenant_id, delivery.id, "sent", None)
                    .await;
            }
            Ok(res) => {
                let msg = format!("HTTP {}", res.status());
                let _ = repo
                    .update_delivery_status(tenant_id, delivery.id, "failed", Some(&msg))
                    .await;
            }
            Err(e) => {
                let _ = repo
                    .update_delivery_status(tenant_id, delivery.id, "failed", Some(&e.to_string()))
                    .await;
            }
        }

        let updated = repo
            .get_delivery(tenant_id, delivery.id)
            .await?
            .unwrap_or(delivery);

        Ok(updated.into())
    }

    /// List all notification preferences for a tenant.
    pub async fn list_preferences(
        repo: &dyn NotificationRepository,
        tenant_id: Uuid,
    ) -> AppResult<Vec<NotificationPreferenceResponse>> {
        let prefs = repo.list_preferences(tenant_id).await?;
        Ok(prefs.into_iter().map(Into::into).collect())
    }

    /// Batch-update notification preferences (upsert semantics).
    /// Validates webhook URLs at save time to reject SSRF targets immediately.
    pub async fn update_preferences(
        repo: &dyn NotificationRepository,
        tenant_id: Uuid,
        updates: Vec<PreferenceUpdate>,
    ) -> AppResult<Vec<NotificationPreferenceResponse>> {
        let mut results = Vec::new();
        for update in &updates {
            // Validate webhook URLs at save time — reject private/internal targets
            if update.channel == "webhook"
                && let Some(ref config) = update.config
                && let Some(url) = config.get("url").and_then(|v| v.as_str())
                && !is_safe_webhook_url(url).await
            {
                return Err(AppError::BadRequest {
                    message: format!(
                        "Webhook URL rejected: {} targets a private or internal network",
                        url
                    ),
                });
            }
        }
        for update in updates {
            let pref = repo
                .upsert_preference(
                    tenant_id,
                    &update.channel,
                    &update.event_type,
                    update.enabled,
                    update.config.unwrap_or(serde_json::json!({})),
                )
                .await?;
            results.push(pref.into());
        }
        Ok(results)
    }
}
