//! Teacher-GPU spend cap.
//!
//! Extraction burns our own metered GPU time (unlike Stage 1, where teacher
//! cost sat on the tenant's own API key), so it gets its own monthly budget
//! line — separate from the general plan spend cap in `plan_service` — that
//! an operator can configure per plan tier via `TEACHER_GPU_SPEND_CAP_*`.
//! Shape mirrors `plan_service::check_spend_cap`: resolve the cap, sum
//! committed spend for the month, refuse if the estimate would push over it.

use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::repositories::traits::BillingEventRepository;
use crate::services::plan_service::current_month_start;

/// User-facing refusal message. Verbatim from the Stage 2 plan's UX spec
/// ("Spend cap hit") — the workflow surfaces this unmodified.
pub const SPEND_CAP_MESSAGE: &str = "This run reached your GPU spending cap for teachers. Raise the cap in Settings → Billing or resume with a smaller dataset.";

/// Whether admitting an extraction estimated at `estimate_cost_usd` would push
/// the tenant's teacher-GPU spend this month over `cap`. `cap = None` (the
/// tenant's plan has no `TEACHER_GPU_SPEND_CAP_*` configured) never refuses.
///
/// Additive/prospective, like `PlanLimits::storage_would_exceed`: exactly at
/// the cap is allowed, one cent over is not.
pub fn teacher_gpu_spend_would_exceed_cap(
    cap: Option<f64>,
    already_spent_usd: f64,
    estimate_cost_usd: f64,
) -> bool {
    match cap {
        Some(cap) => already_spent_usd + estimate_cost_usd > cap,
        None => false,
    }
}

/// Refuse an extraction whose estimated cost would push the tenant's
/// teacher-GPU spend this month over its configured cap. `cap` is resolved by
/// the caller (`Config::teacher_gpu_spend_cap`) from the tenant's plan, since
/// this module has no `Config` dependency of its own.
///
/// Not yet called from a route — the extraction-admission endpoint that
/// invokes this lands in a later Stage 2 task.
#[allow(dead_code)]
pub async fn check_teacher_gpu_spend_cap(
    billing_repo: &dyn BillingEventRepository,
    tenant_id: Uuid,
    cap: Option<f64>,
    estimate_cost_usd: f64,
) -> AppResult<()> {
    let Some(cap) = cap else {
        return Ok(());
    };

    let spent = billing_repo
        .sum_cost_since_for_operation(
            tenant_id,
            platform_shared::enums::BillingOperation::Extraction
                .to_string()
                .as_str(),
            current_month_start(),
        )
        .await?;

    if teacher_gpu_spend_would_exceed_cap(Some(cap), spent, estimate_cost_usd) {
        return Err(AppError::Forbidden {
            message: SPEND_CAP_MESSAGE.to_string(),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unset_cap_never_refuses() {
        assert!(!teacher_gpu_spend_would_exceed_cap(None, 0.0, 1_000_000.0));
    }

    #[test]
    fn exactly_at_cap_is_allowed() {
        assert!(!teacher_gpu_spend_would_exceed_cap(Some(50.0), 40.0, 10.0));
    }

    #[test]
    fn one_cent_over_cap_is_refused() {
        assert!(teacher_gpu_spend_would_exceed_cap(Some(50.0), 40.0, 10.01));
    }

    #[test]
    fn already_over_cap_refuses_any_further_estimate() {
        assert!(teacher_gpu_spend_would_exceed_cap(Some(50.0), 60.0, 0.01));
    }

    #[test]
    fn zero_estimate_never_pushes_over_an_unbreached_cap() {
        assert!(!teacher_gpu_spend_would_exceed_cap(Some(50.0), 10.0, 0.0));
    }
}
