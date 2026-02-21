use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::repositories::traits::TenantRepository;

/// Resource limits for each subscription plan.
///
/// Computed from the plan name string. The `plan_limits` JSONB column
/// in the database is reserved for potential future per-tenant overrides.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
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
            _ => PlanLimits {
                max_projects: 2,
                max_models: 2,
                max_team_members: 1,
                max_training_pairs: 1_000,
                max_storage_gb: 5,
            },
        }
    }
}

pub struct PlanService;

#[allow(dead_code)]
impl PlanService {
    /// Check whether the tenant has reached the limit for a given resource.
    ///
    /// Returns `Err(Forbidden)` if the current count meets or exceeds the
    /// plan limit for the specified resource.
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
            _ => return Ok(()),
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
}
