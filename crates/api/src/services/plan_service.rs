use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::repositories::traits::TenantRepository;

/// Resource limits for each subscription plan.
///
/// Computed from the plan name string. The `plan_limits` JSONB column
/// in the database is reserved for potential future per-tenant overrides.
#[derive(Debug, Clone, Serialize, Deserialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct PlanLimits {
    pub max_projects: i64,
    pub max_models: i64,
    pub max_team_members: i64,
    pub max_training_pairs: i64,
    pub max_storage_gb: i64,
}

impl PlanLimits {
    pub fn for_plan(plan: &str) -> Self {
        match plan {
            "growth" => PlanLimits {
                max_projects: 10,
                max_models: 10,
                max_team_members: 10,
                max_training_pairs: 50_000,
                max_storage_gb: 50,
            },
            "pro" => PlanLimits {
                max_projects: 100,
                max_models: 50,
                max_team_members: 50,
                max_training_pairs: 500_000,
                max_storage_gb: 500,
            },
            // starter/free — default tier
            _ => {
                if plan != "starter" && plan != "free" && !plan.is_empty() {
                    tracing::warn!(plan = plan, "Unknown plan — defaulting to starter limits");
                }
                PlanLimits {
                    max_projects: 2,
                    max_models: 2,
                    max_team_members: 1,
                    max_training_pairs: 1_000,
                    max_storage_gb: 5,
                }
            }
        }
    }

    /// Whether adding `additional_bytes` to the tenant's `current_bytes` would
    /// exceed the plan's storage allowance.
    pub fn storage_would_exceed(&self, current_bytes: i64, additional_bytes: i64) -> bool {
        let max_bytes = self.max_storage_gb.saturating_mul(BYTES_PER_GB);
        current_bytes.saturating_add(additional_bytes) > max_bytes
    }

    /// Whether the tenant has already reached its training-pair allowance.
    pub fn training_pairs_exhausted(&self, current_pairs: i64) -> bool {
        current_pairs >= self.max_training_pairs
    }
}

pub struct PlanService;

impl PlanService {
    /// Check whether the tenant has reached the limit for a given resource.
    ///
    /// Returns `Err(Forbidden)` if the current count meets or exceeds the
    /// plan limit for the specified resource.
    ///
    /// NOTE: Prefer the atomic `create_with_limit` pattern on repositories
    /// where available. This non-atomic check is kept for resources that
    /// don't yet have an atomic variant.
    #[allow(dead_code)]
    pub async fn check_limit(
        tenant_repo: &dyn TenantRepository,
        tenant_id: Uuid,
        resource: &str,
        current_count: i64,
    ) -> AppResult<()> {
        let tenant = tenant_repo
            .get_by_id(tenant_id)
            .await?
            .ok_or(AppError::NotFound {
                message: "Tenant not found".into(),
            })?;

        let limits = PlanLimits::for_plan(&tenant.plan);

        let max = match resource {
            "projects" => limits.max_projects,
            "models" => limits.max_models,
            "team_members" => limits.max_team_members,
            "training_pairs" => limits.max_training_pairs,
            _ => {
                tracing::warn!(
                    resource = resource,
                    "Unknown resource type in plan limit check — allowing"
                );
                return Ok(());
            }
        };

        if current_count >= max {
            return Err(AppError::Forbidden {
                message: format!(
                    "Plan limit reached: maximum {} {} on your current plan",
                    max, resource
                ),
            });
        }

        Ok(())
    }

    /// Get the plan limits for a tenant.
    pub async fn get_limits(
        tenant_repo: &dyn TenantRepository,
        tenant_id: Uuid,
    ) -> AppResult<PlanLimits> {
        let tenant = tenant_repo
            .get_by_id(tenant_id)
            .await?
            .ok_or(AppError::NotFound {
                message: "Tenant not found".into(),
            })?;

        Ok(PlanLimits::for_plan(&tenant.plan))
    }

    /// Reject an upload that would push the tenant's stored bytes over its plan
    /// storage allowance.
    pub async fn check_storage_limit(
        tenant_repo: &dyn TenantRepository,
        tenant_id: Uuid,
        current_bytes: i64,
        additional_bytes: i64,
    ) -> AppResult<()> {
        let limits = Self::get_limits(tenant_repo, tenant_id).await?;
        if limits.storage_would_exceed(current_bytes, additional_bytes) {
            return Err(AppError::Forbidden {
                message: format!(
                    "Storage limit reached: your plan allows up to {} GB. Upgrade or delete files to add more.",
                    limits.max_storage_gb
                ),
            });
        }
        Ok(())
    }

    /// Reject dataset generation once the tenant has reached its plan's total
    /// training-pair allowance.
    pub async fn check_training_pairs_limit(
        tenant_repo: &dyn TenantRepository,
        tenant_id: Uuid,
        current_pairs: i64,
    ) -> AppResult<()> {
        let limits = Self::get_limits(tenant_repo, tenant_id).await?;
        if limits.training_pairs_exhausted(current_pairs) {
            return Err(AppError::Forbidden {
                message: format!(
                    "Training-pair limit reached: your plan allows up to {} training pairs. Upgrade to generate more.",
                    limits.max_training_pairs
                ),
            });
        }
        Ok(())
    }
}

const BYTES_PER_GB: i64 = 1024 * 1024 * 1024;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_limit_boundary() {
        let free = PlanLimits::for_plan("free");
        let cap = free.max_storage_gb * BYTES_PER_GB;
        // Exactly at the cap is allowed; one byte over is not.
        assert!(!free.storage_would_exceed(cap - 100, 100));
        assert!(free.storage_would_exceed(cap - 100, 101));
        assert!(!free.storage_would_exceed(0, 0));
    }

    #[test]
    fn storage_limit_scales_with_plan() {
        let free = PlanLimits::for_plan("free");
        let pro = PlanLimits::for_plan("pro");
        let ten_gb = 10 * BYTES_PER_GB;
        // 10 GB exceeds the 5 GB free tier but fits the pro tier.
        assert!(free.storage_would_exceed(0, ten_gb));
        assert!(!pro.storage_would_exceed(0, ten_gb));
    }

    #[test]
    fn training_pairs_exhaustion() {
        let free = PlanLimits::for_plan("free");
        assert!(!free.training_pairs_exhausted(free.max_training_pairs - 1));
        assert!(free.training_pairs_exhausted(free.max_training_pairs));
        assert!(free.training_pairs_exhausted(free.max_training_pairs + 1));
    }
}
