use uuid::Uuid;

use crate::auth::AuthenticatedUser;
use crate::repositories::traits::AuditLogRepository;

/// Convenience service for logging audit events.
///
/// Best-effort: audit failures log a warning but never fail the primary operation.
pub struct AuditLogger;

impl AuditLogger {
    /// Log an action performed by an authenticated user.
    pub async fn log(
        repo: &dyn AuditLogRepository,
        user: &AuthenticatedUser,
        action: &str,
        resource_type: &str,
        resource_id: Option<Uuid>,
        metadata: serde_json::Value,
    ) {
        if let Err(e) = repo
            .create(
                user.tenant_id,
                &user.user_id,
                action,
                resource_type,
                resource_id,
                metadata,
            )
            .await
        {
            tracing::warn!(
                action,
                resource_type,
                ?resource_id,
                error = %e,
                "Failed to write audit log"
            );
        }
    }

    /// Log an action performed by the system (no user context).
    #[allow(dead_code)]
    pub async fn log_system(
        repo: &dyn AuditLogRepository,
        tenant_id: Uuid,
        action: &str,
        resource_type: &str,
        resource_id: Option<Uuid>,
        metadata: serde_json::Value,
    ) {
        if let Err(e) = repo
            .create(
                tenant_id,
                "system",
                action,
                resource_type,
                resource_id,
                metadata,
            )
            .await
        {
            tracing::warn!(
                action,
                resource_type,
                ?resource_id,
                error = %e,
                "Failed to write system audit log"
            );
        }
    }
}
