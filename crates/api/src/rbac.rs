use platform_shared::enums::TeamRole;

use crate::auth::AuthenticatedUser;
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
