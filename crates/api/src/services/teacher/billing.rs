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
    use crate::repositories::billing_event_repo::{InferenceUsageDay, UsageSummary};
    use futures::future::BoxFuture;
    use platform_db::models::BillingEvent;
    use std::sync::Mutex;

    /// Ledger stand-in that answers only the one query the cap makes, recording
    /// which operation it was asked about. Everything else is out of scope for a
    /// spend cap and would be a bug to call.
    struct LedgerStub {
        extraction_spend: f64,
        asked: Mutex<Vec<String>>,
    }

    impl LedgerStub {
        fn with_spend(extraction_spend: f64) -> Self {
            Self {
                extraction_spend,
                asked: Mutex::new(Vec::new()),
            }
        }

        fn operations_asked_about(&self) -> Vec<String> {
            self.asked.lock().expect("stub lock").clone()
        }
    }

    impl BillingEventRepository for LedgerStub {
        fn sum_cost_since_for_operation(
            &self,
            _tenant_id: Uuid,
            operation: &str,
            _since: chrono::DateTime<chrono::Utc>,
        ) -> BoxFuture<'_, AppResult<f64>> {
            self.asked
                .lock()
                .expect("stub lock")
                .push(operation.to_string());
            let spend = if operation == "extraction" {
                self.extraction_spend
            } else {
                0.0
            };
            Box::pin(async move { Ok(spend) })
        }

        fn create(
            &self,
            _tenant_id: Uuid,
            _operation: &str,
            _resource_id: Option<Uuid>,
            _tokens_in: i64,
            _tokens_out: i64,
            _gpu_seconds: i32,
            _cost_usd: f64,
            _metadata: serde_json::Value,
        ) -> BoxFuture<'_, AppResult<BillingEvent>> {
            unimplemented!("the spend cap only reads")
        }

        fn list_by_tenant(
            &self,
            _tenant_id: Uuid,
            _offset: i64,
            _limit: i64,
        ) -> BoxFuture<'_, AppResult<Vec<BillingEvent>>> {
            unimplemented!("not part of spend-cap accounting")
        }

        fn count_by_tenant(&self, _tenant_id: Uuid) -> BoxFuture<'_, AppResult<i64>> {
            unimplemented!("not part of spend-cap accounting")
        }

        fn sum_by_resource(
            &self,
            _tenant_id: Uuid,
            _resource_id: Uuid,
        ) -> BoxFuture<'_, AppResult<UsageSummary>> {
            unimplemented!("not part of spend-cap accounting")
        }

        fn usage_by_day(
            &self,
            _tenant_id: Uuid,
            _days: i32,
        ) -> BoxFuture<'_, AppResult<Vec<(String, f64)>>> {
            unimplemented!("not part of spend-cap accounting")
        }

        fn usage_totals(&self, _tenant_id: Uuid) -> BoxFuture<'_, AppResult<(f64, i64, i64)>> {
            unimplemented!("not part of spend-cap accounting")
        }

        fn sum_cost_since(
            &self,
            _tenant_id: Uuid,
            _since: chrono::DateTime<chrono::Utc>,
        ) -> BoxFuture<'_, AppResult<f64>> {
            unimplemented!("the teacher-GPU cap is its own budget line")
        }

        fn inference_usage_by_day(
            &self,
            _tenant_id: Uuid,
            _days: i32,
        ) -> BoxFuture<'_, AppResult<Vec<InferenceUsageDay>>> {
            unimplemented!("not part of spend-cap accounting")
        }
    }

    /// The end-to-end shape of the cap: recorded extraction spend in the ledger
    /// plus a new run's estimate over the cap must refuse. This is the assertion
    /// that was vacuous while nothing wrote an `extraction` row — the sum was
    /// always 0.0, so no estimate could ever push it over.
    #[tokio::test]
    async fn recorded_extraction_spend_plus_a_new_estimate_refuses() {
        let ledger = LedgerStub::with_spend(45.00);

        let refusal = check_teacher_gpu_spend_cap(&ledger, Uuid::new_v4(), Some(50.00), 6.00)
            .await
            .expect_err("45 + 6 is over a 50 cap");

        assert!(matches!(refusal, AppError::Forbidden { .. }));
        assert_eq!(refusal.to_string(), SPEND_CAP_MESSAGE);
    }

    #[tokio::test]
    async fn recorded_spend_that_still_fits_admits() {
        let ledger = LedgerStub::with_spend(45.00);

        check_teacher_gpu_spend_cap(&ledger, Uuid::new_v4(), Some(50.00), 5.00)
            .await
            .expect("45 + 5 is exactly at a 50 cap");
    }

    /// The cap sums the same operation the worker writes. If either side renames
    /// it, the recorded spend silently stops counting.
    #[tokio::test]
    async fn the_cap_counts_the_extraction_operation() {
        let ledger = LedgerStub::with_spend(100.00);

        let _ = check_teacher_gpu_spend_cap(&ledger, Uuid::new_v4(), Some(1.00), 0.01).await;

        assert_eq!(
            ledger.operations_asked_about(),
            vec![platform_shared::enums::BillingOperation::Extraction.to_string()]
        );
    }

    #[tokio::test]
    async fn an_unset_cap_never_queries_the_ledger() {
        let ledger = LedgerStub::with_spend(1_000_000.0);

        check_teacher_gpu_spend_cap(&ledger, Uuid::new_v4(), None, 999.0)
            .await
            .expect("no cap, no refusal");

        assert!(ledger.operations_asked_about().is_empty());
    }

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
