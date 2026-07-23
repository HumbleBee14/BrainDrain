use async_trait::async_trait;

use crate::error::AppResult;

/// A transactional email (plain text).
#[derive(Debug, Clone)]
pub struct EmailMessage {
    pub to: String,
    pub subject: String,
    pub body: String,
}

/// Vendor-agnostic email sender. Implementations talk to an existing provider
/// over SMTP; swapping providers is config-only. A provider-specific HTTP-API
/// sender can later live behind the same trait.
#[async_trait]
pub trait EmailProvider: Send + Sync {
    /// Returns `Err` on any failure — including "not configured" — so a
    /// non-send is never mistaken for success.
    async fn send(&self, message: EmailMessage) -> AppResult<()>;
}
