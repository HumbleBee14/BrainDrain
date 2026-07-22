use platform_shared::enums::TeamRole;

use crate::auth::AuthenticatedUser;
use crate::config::Config;
use crate::error::{AppError, AppResult};

/// Check that the user's role meets the minimum required level.
/// Returns 403 Forbidden if insufficient.
pub fn require_role(user: &AuthenticatedUser, minimum: TeamRole) -> AppResult<()> {
    if user.role < minimum {
        return Err(AppError::Forbidden {
            message: "Insufficient permissions".to_string(),
        });
    }
    Ok(())
}

/// Check that the caller is a platform administrator.
///
/// Platform-admin endpoints manage shared, cross-tenant infrastructure (e.g.
/// the inference-instance fleet). A tenant `Owner` role is NOT sufficient —
/// that only grants authority within a single tenant. Authority is granted
/// explicitly via the `PLATFORM_ADMIN_USER_IDS` (auth subject / `sub`) and
/// `PLATFORM_ADMIN_EMAILS` allowlists. Both empty ⇒ nobody is a platform admin
/// (deny-all, the secure default).
pub fn require_platform_admin(user: &AuthenticatedUser, config: &Config) -> AppResult<()> {
    let by_id = config
        .platform_admin_user_ids_list()
        .iter()
        .any(|id| id == &user.user_id);

    let by_email = match &user.email {
        Some(email) => {
            let email = email.to_lowercase();
            config
                .platform_admin_emails_list()
                .iter()
                .any(|allowed| allowed == &email)
        }
        None => false,
    };

    if by_id || by_email {
        Ok(())
    } else {
        Err(AppError::Forbidden {
            message: "Platform administrator access required".to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn user(user_id: &str, email: Option<&str>) -> AuthenticatedUser {
        AuthenticatedUser {
            user_id: user_id.to_string(),
            tenant_id: Uuid::nil(),
            org_id: None,
            email: email.map(|s| s.to_string()),
            // Even a tenant Owner must NOT pass the platform-admin gate on role alone.
            role: TeamRole::Owner,
        }
    }

    fn config_with(user_ids: &str, emails: &str) -> Config {
        let mut cfg = Config::test_default();
        cfg.platform_admin_user_ids = user_ids.to_string();
        cfg.platform_admin_emails = emails.to_string();
        cfg
    }

    #[test]
    fn empty_allowlists_deny_everyone() {
        // Deny-all is the secure default — even a tenant Owner is rejected.
        let cfg = config_with("", "");
        assert!(require_platform_admin(&user("user_1", Some("a@x.com")), &cfg).is_err());
    }

    #[test]
    fn user_id_allowlist_grants() {
        let cfg = config_with("user_1, user_2", "");
        assert!(require_platform_admin(&user("user_2", None), &cfg).is_ok());
        assert!(require_platform_admin(&user("user_3", None), &cfg).is_err());
    }

    #[test]
    fn email_allowlist_grants_case_insensitively() {
        let cfg = config_with("", "Admin@Example.com");
        assert!(require_platform_admin(&user("u", Some("admin@example.com")), &cfg).is_ok());
        assert!(require_platform_admin(&user("u", Some("ADMIN@EXAMPLE.COM")), &cfg).is_ok());
        assert!(require_platform_admin(&user("u", Some("other@example.com")), &cfg).is_err());
    }

    #[test]
    fn missing_email_claim_cannot_match_email_allowlist() {
        let cfg = config_with("", "admin@example.com");
        assert!(require_platform_admin(&user("u", None), &cfg).is_err());
    }

    #[test]
    fn either_allowlist_is_sufficient() {
        let cfg = config_with("ops_bot", "admin@example.com");
        assert!(require_platform_admin(&user("ops_bot", None), &cfg).is_ok());
        assert!(require_platform_admin(&user("someone", Some("admin@example.com")), &cfg).is_ok());
    }
}
