use async_trait::async_trait;
use lettre::message::Mailbox;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

use crate::error::{AppError, AppResult};
use crate::services::email_provider::{EmailMessage, EmailProvider};

/// SMTP-backed email provider. Works with any SMTP email service; the choice is
/// pure configuration.
pub struct SmtpEmailProvider {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: Mailbox,
}

impl SmtpEmailProvider {
    /// Port 465 uses implicit TLS; other ports use STARTTLS. Fails fast on an
    /// invalid `from` address or TLS setup.
    pub fn new(
        host: &str,
        port: u16,
        username: String,
        password: String,
        from: &str,
    ) -> AppResult<Self> {
        let from: Mailbox = from.parse().map_err(|e| {
            AppError::Internal(anyhow::anyhow!("Invalid EMAIL_FROM address {from:?}: {e}"))
        })?;

        let builder = if port == 465 {
            AsyncSmtpTransport::<Tokio1Executor>::relay(host)
        } else {
            AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(host)
        }
        .map_err(|e| AppError::Internal(anyhow::anyhow!("SMTP transport setup failed: {e}")))?;

        let transport = builder
            .port(port)
            .credentials(Credentials::new(username, password))
            .build();

        Ok(Self { transport, from })
    }
}

#[async_trait]
impl EmailProvider for SmtpEmailProvider {
    async fn send(&self, message: EmailMessage) -> AppResult<()> {
        let to: Mailbox = message.to.parse().map_err(|e| AppError::BadRequest {
            message: format!("Invalid recipient email address {:?}: {e}", message.to),
        })?;

        let email = Message::builder()
            .from(self.from.clone())
            .to(to)
            .subject(message.subject)
            .body(message.body)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to build email: {e}")))?;

        self.transport
            .send(email)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("SMTP send failed: {e}")))?;

        Ok(())
    }
}

/// Fallback when SMTP is unconfigured: every `send` errors, so email is never
/// silently dropped.
pub struct NoOpEmailProvider;

#[async_trait]
impl EmailProvider for NoOpEmailProvider {
    async fn send(&self, _message: EmailMessage) -> AppResult<()> {
        Err(AppError::Internal(anyhow::anyhow!(
            "email provider not configured — set SMTP_HOST/SMTP_PORT/SMTP_USERNAME/SMTP_PASSWORD/EMAIL_FROM"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn noop_never_fakes_success() {
        let result = NoOpEmailProvider
            .send(EmailMessage {
                to: "a@b.com".to_string(),
                subject: "s".to_string(),
                body: "b".to_string(),
            })
            .await;
        assert!(result.is_err());
    }

    #[test]
    fn rejects_invalid_from_address() {
        assert!(
            SmtpEmailProvider::new(
                "smtp.example.com",
                587,
                "user".to_string(),
                "pass".to_string(),
                "not-an-email",
            )
            .is_err()
        );
    }

    #[tokio::test]
    async fn builds_with_valid_settings() {
        assert!(
            SmtpEmailProvider::new(
                "smtp.example.com",
                587,
                "user".to_string(),
                "pass".to_string(),
                "Platform <noreply@example.com>",
            )
            .is_ok()
        );
    }
}
