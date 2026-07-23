use std::net::IpAddr;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use hmac::{Hmac, Mac};
use sha2::Sha256;
use tokio::sync::oneshot;

use crate::repositories::traits::NotificationRepository;
use crate::services::email_provider::{EmailMessage, EmailProvider};

/// Maximum delivery attempts before a delivery is considered permanently failed.
const MAX_DELIVERY_ATTEMPTS: i32 = 5;

/// Number of deliveries to fetch per poll cycle.
const BATCH_SIZE: i64 = 50;

/// How long a claimed delivery is leased to this worker. Longer than the
/// longest per-attempt dispatch timeout so an in-flight delivery is never
/// re-claimed; a crashed worker's rows become eligible again after this.
const CLAIM_LEASE_SECS: i64 = 120;

/// Record a failed delivery and dead-letter (log loudly) when it has exhausted
/// its retry budget.
async fn mark_failed(
    repo: &dyn NotificationRepository,
    delivery: &platform_db::models::NotificationDelivery,
    error: &str,
) {
    let _ = repo
        .update_delivery_status(delivery.tenant_id, delivery.id, "failed", Some(error))
        .await;
    if delivery.attempts + 1 >= MAX_DELIVERY_ATTEMPTS {
        tracing::error!(
            delivery_id = %delivery.id,
            tenant_id = %delivery.tenant_id,
            event_type = %delivery.event_type,
            channel = %delivery.channel,
            attempts = delivery.attempts + 1,
            error,
            "Notification delivery exhausted retries — dead-lettered"
        );
    }
}

/// Background worker that polls for pending notification deliveries and dispatches them.
///
/// Follows the same lifecycle pattern as `BillingBatcher`:
/// - Spawns a background `tokio::spawn` task on creation
/// - Periodic poll loop with graceful shutdown via oneshot signal
/// - `shutdown()` is safe to call multiple times (idempotent via `Mutex<Option<...>>`)
pub struct DeliveryWorker {
    shutdown: Mutex<Option<ShutdownHandle>>,
}

struct ShutdownHandle {
    signal: oneshot::Sender<()>,
    task_handle: tokio::task::JoinHandle<()>,
}

impl DeliveryWorker {
    /// Disabled worker used when feature flags turn delivery processing off.
    pub fn disabled() -> Self {
        Self {
            shutdown: Mutex::new(None),
        }
    }

    /// Spawn the delivery worker with a background poll loop.
    ///
    /// - `poll_interval`: how often to check for pending deliveries
    pub fn new(
        repo: Arc<dyn NotificationRepository>,
        http_client: reqwest::Client,
        email_provider: Arc<dyn EmailProvider>,
        poll_interval: Duration,
    ) -> Self {
        let (shutdown_tx, shutdown_rx) = oneshot::channel();

        let task_handle = tokio::spawn(poll_loop(
            repo,
            http_client,
            email_provider,
            poll_interval,
            shutdown_rx,
        ));

        Self {
            shutdown: Mutex::new(Some(ShutdownHandle {
                signal: shutdown_tx,
                task_handle,
            })),
        }
    }

    /// Graceful shutdown: signal the poll loop to stop and wait for it to finish.
    pub async fn shutdown(&self) {
        let handle = self
            .shutdown
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();

        if let Some(handle) = handle {
            let _ = handle.signal.send(());
            if let Err(e) = handle.task_handle.await {
                tracing::error!(error = %e, "Delivery worker task panicked");
            }
            tracing::info!("Delivery worker shutdown complete");
        }
    }
}

/// Background loop: polls for pending deliveries and dispatches them.
async fn poll_loop(
    repo: Arc<dyn NotificationRepository>,
    http_client: reqwest::Client,
    email_provider: Arc<dyn EmailProvider>,
    poll_interval: Duration,
    mut shutdown_rx: oneshot::Receiver<()>,
) {
    let mut interval = tokio::time::interval(poll_interval);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    tracing::info!(
        poll_interval_secs = poll_interval.as_secs(),
        "Delivery worker started"
    );

    loop {
        tokio::select! {
            _ = interval.tick() => {
                process_pending(&*repo, &http_client, &*email_provider).await;
            }
            _ = &mut shutdown_rx => {
                // Process one final batch before exiting
                process_pending(&*repo, &http_client, &*email_provider).await;
                tracing::info!("Delivery worker shut down (explicit signal)");
                return;
            }
        }
    }
}

/// Fetch and process a batch of pending deliveries.
async fn process_pending(
    repo: &dyn NotificationRepository,
    http_client: &reqwest::Client,
    email_provider: &dyn EmailProvider,
) {
    let deliveries = match repo
        .claim_pending_deliveries(MAX_DELIVERY_ATTEMPTS, BATCH_SIZE, CLAIM_LEASE_SECS)
        .await
    {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!(error = %e, "Failed to fetch pending deliveries");
            return;
        }
    };

    if deliveries.is_empty() {
        return;
    }

    tracing::debug!(count = deliveries.len(), "Processing pending deliveries");

    for delivery in deliveries {
        // Look up the preference to get the webhook URL
        let pref = match repo
            .get_preference(delivery.tenant_id, delivery.preference_id)
            .await
        {
            Ok(Some(p)) => p,
            Ok(None) => {
                tracing::warn!(
                    delivery_id = %delivery.id,
                    preference_id = %delivery.preference_id,
                    "Preference not found — marking delivery as failed"
                );
                mark_failed(repo, &delivery, "Notification preference deleted").await;
                continue;
            }
            Err(e) => {
                tracing::warn!(
                    delivery_id = %delivery.id,
                    error = %e,
                    "Failed to fetch preference"
                );
                continue;
            }
        };

        match pref.channel.as_str() {
            "webhook" => {
                dispatch_webhook(repo, http_client, &delivery, &pref).await;
            }
            "email" => {
                dispatch_email(repo, email_provider, &delivery, &pref).await;
            }
            "in_app" => {
                // In-app deliveries are read directly by the client; there is no
                // external dispatch. Mark sent so the row leaves the pending poll.
                let _ = repo
                    .update_delivery_status(delivery.tenant_id, delivery.id, "sent", None)
                    .await;
            }
            other => {
                tracing::warn!(
                    delivery_id = %delivery.id,
                    channel = other,
                    "Unknown delivery channel — marking as failed"
                );
                mark_failed(repo, &delivery, &format!("Unknown channel: {other}")).await;
            }
        }
    }
}

/// Dispatch a single webhook delivery with SSRF protection and timeout.
async fn dispatch_webhook(
    repo: &dyn NotificationRepository,
    http_client: &reqwest::Client,
    delivery: &platform_db::models::NotificationDelivery,
    pref: &platform_db::models::NotificationPreference,
) {
    let url = match pref.config.get("url").and_then(|v| v.as_str()) {
        Some(u) => u,
        None => {
            mark_failed(repo, delivery, "No webhook URL configured").await;
            return;
        }
    };

    // SSRF protection — re-validate on every attempt (DNS can change)
    if !is_safe_webhook_url(url).await {
        tracing::warn!(
            delivery_id = %delivery.id,
            url,
            "Webhook URL targets private/internal network — marking as failed"
        );
        mark_failed(
            repo,
            delivery,
            "Webhook URL targets a private or internal network",
        )
        .await;
        return;
    }

    // Exponential backoff timeout: 10s base, capped at 30s
    let timeout_secs = std::cmp::min(10 * 2u64.pow(delivery.attempts as u32), 30);

    // Serialize once so the signature covers the exact bytes sent.
    let body = match serde_json::to_vec(&delivery.payload) {
        Ok(b) => b,
        Err(e) => {
            mark_failed(
                repo,
                delivery,
                &format!("Payload serialization failed: {e}"),
            )
            .await;
            return;
        }
    };

    let mut request = http_client
        .post(url)
        .header("content-type", "application/json")
        .timeout(Duration::from_secs(timeout_secs));

    // Optional HMAC-SHA256 signature so receivers can verify authenticity when a
    // shared secret is configured on the webhook preference.
    if let Some(secret) = pref.config.get("secret").and_then(|v| v.as_str())
        && let Some(signature) = sign_payload(secret, &body)
    {
        request = request.header("x-webhook-signature", format!("sha256={signature}"));
    }

    let result = request.body(body).send().await;

    match result {
        Ok(res) if res.status().is_success() => {
            let _ = repo
                .update_delivery_status(delivery.tenant_id, delivery.id, "sent", None)
                .await;
            tracing::debug!(
                delivery_id = %delivery.id,
                status = %res.status(),
                "Webhook delivered successfully"
            );
        }
        Ok(res) => {
            let status = res.status();
            mark_failed(repo, delivery, &format!("HTTP {status}")).await;
            tracing::debug!(
                delivery_id = %delivery.id,
                status = %status,
                attempt = delivery.attempts + 1,
                "Webhook delivery failed"
            );
        }
        Err(e) => {
            mark_failed(repo, delivery, &e.to_string()).await;
            tracing::debug!(
                delivery_id = %delivery.id,
                error = %e,
                attempt = delivery.attempts + 1,
                "Webhook delivery error"
            );
        }
    }
}

/// Hex-encoded HMAC-SHA256 of `body` keyed by `secret`, or `None` if the key is
/// unusable.
fn sign_payload(secret: &str, body: &[u8]) -> Option<String> {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).ok()?;
    mac.update(body);
    let bytes = mac.finalize().into_bytes();
    Some(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Send one email delivery through the provider and record the outcome.
async fn dispatch_email(
    repo: &dyn NotificationRepository,
    email_provider: &dyn EmailProvider,
    delivery: &platform_db::models::NotificationDelivery,
    pref: &platform_db::models::NotificationPreference,
) {
    let address = match pref.config.get("address").and_then(|v| v.as_str()) {
        Some(a) if !a.is_empty() => a.to_string(),
        _ => {
            mark_failed(repo, delivery, "No email address configured").await;
            return;
        }
    };

    let subject = delivery
        .payload
        .get("subject")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("Notification: {}", delivery.event_type));

    let body = delivery
        .payload
        .get("message")
        .or_else(|| delivery.payload.get("body"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| serde_json::to_string_pretty(&delivery.payload).unwrap_or_default());

    let result = email_provider
        .send(EmailMessage {
            to: address,
            subject,
            body,
        })
        .await;

    match result {
        Ok(()) => {
            let _ = repo
                .update_delivery_status(delivery.tenant_id, delivery.id, "sent", None)
                .await;
            tracing::debug!(delivery_id = %delivery.id, "Email delivered successfully");
        }
        Err(e) => {
            let msg = e.to_string();
            mark_failed(repo, delivery, &msg).await;
            tracing::debug!(
                delivery_id = %delivery.id,
                error = %msg,
                attempt = delivery.attempts + 1,
                "Email delivery failed"
            );
        }
    }
}

/// Returns `true` if the IP address is in a private, reserved, or internal range.
fn is_private_ip(ip: &IpAddr) -> bool {
    if ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() {
        return true;
    }
    match ip {
        IpAddr::V4(v4) => {
            let octets = v4.octets();
            octets[0] == 10
                || (octets[0] == 172 && (16..=31).contains(&octets[1]))
                || (octets[0] == 192 && octets[1] == 168)
                || (octets[0] == 169 && octets[1] == 254)
        }
        IpAddr::V6(v6) => {
            let segments = v6.segments();
            (segments[0] & 0xfe00) == 0xfc00
                || (segments[0] & 0xffc0) == 0xfe80
                || v6.is_loopback()
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
async fn is_safe_webhook_url(url: &str) -> bool {
    let parsed = match reqwest::Url::parse(url) {
        Ok(u) => u,
        Err(_) => return false,
    };

    if parsed.scheme() != "https" && parsed.scheme() != "http" {
        return false;
    }

    let host = match parsed.host_str() {
        Some(h) => h,
        None => return false,
    };

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

    found_any
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_ips_detected() {
        assert!(is_private_ip(&"127.0.0.1".parse().unwrap()));
        assert!(is_private_ip(&"10.0.0.1".parse().unwrap()));
        assert!(is_private_ip(&"172.16.0.1".parse().unwrap()));
        assert!(is_private_ip(&"192.168.1.1".parse().unwrap()));
        assert!(is_private_ip(&"169.254.0.1".parse().unwrap()));
        assert!(is_private_ip(&"::1".parse().unwrap()));
    }

    #[test]
    fn public_ips_allowed() {
        assert!(!is_private_ip(&"8.8.8.8".parse().unwrap()));
        assert!(!is_private_ip(&"1.1.1.1".parse().unwrap()));
        assert!(!is_private_ip(&"203.0.113.1".parse().unwrap()));
    }

    #[test]
    fn exponential_backoff_capped() {
        // attempt 0 → 10s, attempt 1 → 20s, attempt 2 → 30s (capped), attempt 3 → 30s (capped)
        assert_eq!(std::cmp::min(10 * 2u64.pow(0), 30), 10);
        assert_eq!(std::cmp::min(10 * 2u64.pow(1), 30), 20);
        assert_eq!(std::cmp::min(10 * 2u64.pow(2), 30), 30);
        assert_eq!(std::cmp::min(10 * 2u64.pow(3), 30), 30);
    }
}
