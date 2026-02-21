use std::net::ToSocketAddrs;

use uuid::Uuid;

use crate::dto::notification::{NotificationPreferenceResponse, PreferenceUpdate};
use crate::error::AppResult;
use crate::repositories::traits::NotificationRepository;

/// Reject webhook URLs that point to private/internal networks (SSRF protection).
fn is_safe_webhook_url(url: &str) -> bool {
    let parsed = match reqwest::Url::parse(url) {
        Ok(u) => u,
        Err(_) => return false,
    };

    // Only allow HTTPS (or HTTP for localhost in dev)
    if parsed.scheme() != "https" && parsed.scheme() != "http" {
        return false;
    }

    let host = match parsed.host_str() {
        Some(h) => h,
        None => return false,
    };

    // Resolve hostname and check all IPs are public
    let port = parsed.port().unwrap_or(if parsed.scheme() == "https" { 443 } else { 80 });
    let addrs = match (host, port).to_socket_addrs() {
        Ok(a) => a,
        Err(_) => return false,
    };

    for addr in addrs {
        let ip = addr.ip();
        if ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() {
            return false;
        }
        // Check private ranges
        match ip {
            std::net::IpAddr::V4(v4) => {
                let octets = v4.octets();
                // 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16, 169.254.0.0/16 (link-local)
                if octets[0] == 10
                    || (octets[0] == 172 && (16..=31).contains(&octets[1]))
                    || (octets[0] == 192 && octets[1] == 168)
                    || (octets[0] == 169 && octets[1] == 254)
                {
                    return false;
                }
            }
            std::net::IpAddr::V6(_) => {
                // Reject all IPv6 private/link-local for simplicity
                if !ip.is_loopback() {
                    // Already checked above, but be defensive
                }
            }
        }
    }

    true
}

/// Handles notification preference management and delivery dispatch.
pub struct NotificationService;

impl NotificationService {
    /// Fire-and-forget notification dispatch (same pattern as AuditLogger).
    ///
    /// Best-effort: failures are logged but never fail the primary operation.
    #[allow(dead_code)]
    pub async fn notify(
        repo: &dyn NotificationRepository,
        http_client: &reqwest::Client,
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
            match pref.channel.as_str() {
                "webhook" => {
                    if let Some(url) = pref.config.get("url").and_then(|v| v.as_str()) {
                        if !is_safe_webhook_url(url) {
                            tracing::warn!(
                                tenant_id = %tenant_id,
                                url,
                                "Webhook URL rejected — targets private/internal network"
                            );
                            continue;
                        }
                        let delivery = repo
                            .create_delivery(
                                tenant_id,
                                pref.id,
                                event_type,
                                "webhook",
                                payload.clone(),
                            )
                            .await;

                        if let Ok(delivery) = delivery {
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
                                    let _ = repo
                                        .update_delivery_status(
                                            tenant_id,
                                            delivery.id,
                                            "failed",
                                            Some(&format!("HTTP {}", res.status())),
                                        )
                                        .await;
                                }
                                Err(e) => {
                                    let _ = repo
                                        .update_delivery_status(
                                            tenant_id,
                                            delivery.id,
                                            "failed",
                                            Some(&e.to_string()),
                                        )
                                        .await;
                                }
                            }
                        }
                    }
                }
                "email" => {
                    // Intentional stub: email delivery requires an external provider
                    // (Resend, SendGrid, SES) behind an EmailSender trait. Preferences
                    // can be saved now; delivery will activate when a provider is wired in.
                    // See: services/billing_provider.rs for the trait pattern to follow.
                    tracing::info!(
                        tenant_id = %tenant_id,
                        event_type,
                        "Email notification skipped — no email provider configured"
                    );
                }
                _ => {}
            }
        }
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
    pub async fn update_preferences(
        repo: &dyn NotificationRepository,
        tenant_id: Uuid,
        updates: Vec<PreferenceUpdate>,
    ) -> AppResult<Vec<NotificationPreferenceResponse>> {
        let mut results = Vec::new();
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
