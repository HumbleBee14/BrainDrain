//! Teacher-GPU spend cap.
//!
//! Extraction and on-policy improve passes burn our own metered GPU time
//! (unlike Stage 1, where teacher cost sat on the tenant's own API key), so they
//! share a monthly budget line — separate from the general plan spend cap in
//! `plan_service` — that an operator can configure per plan tier via
//! `TEACHER_GPU_SPEND_CAP_*`. Shape mirrors `plan_service::check_spend_cap`:
//! resolve the cap, sum committed spend for the month, refuse if the estimate
//! would push over it.
//!
//! The sum spans every operation in `BillingOperation::teacher_gpu_operations`.
//! Counting only one of them would leave the other unbounded: a tenant could run
//! improve passes back to back, each admitted against a total that never grew.
//!
//! It also spans both ledgers: `billing_events` (delivered) and the undelivered
//! rows still in `billing_outbox` — reservations for runs currently holding a
//! GPU and terminal charges awaiting the relay. Reading only `billing_events`
//! admits every run in that window against a total none of them contribute to.
//! Both ledgers are read in one statement — one snapshot — because the relay
//! moves a row between them in a single commit: read separately, a row
//! delivered between the reads is counted by neither, and the cap admits
//! against a total missing spend that just became real.

use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::repositories::traits::BillingEventRepository;
use crate::services::plan_service::current_month_start;
use crate::services::teacher::cost::ExtractionEstimate;
use crate::services::teacher::serving_cost::{split_teacher_serving_cost, teacher_serving_share};

/// User-facing refusal message. Verbatim from the Stage 2 plan's UX spec
/// ("Spend cap hit") — the workflow surfaces this unmodified.
pub const SPEND_CAP_MESSAGE: &str = "This run reached your GPU spending cap for teachers. Raise the cap in Settings → Billing or resume with a smaller dataset.";

/// A durable claim on the teacher-GPU budget, written by the repository in the
/// same transaction that creates the run it pays for.
///
/// Before this row exists, an admitted on-policy run is invisible: it holds a
/// GPU for hours while the cap admits its successors against a total it never
/// joined. The row is withheld from the ledger while `reservation_pending` is
/// set; the run's terminal charge retires it, and the relay reaps one whose run
/// died without ever writing that charge.
pub struct TeacherSpendReservation {
    /// The teacher's slice of the admission estimate — what the run will
    /// contribute to the budget if it never reports back.
    pub gpu_seconds: i32,
    pub cost_usd: f64,
    pub metadata: serde_json::Value,
    /// Re-checked under the creation transaction's lock. `None` reserves
    /// without refusing: capless tenants still pay for crashed runs.
    pub cap_usd: Option<f64>,
    pub month_start: chrono::DateTime<chrono::Utc>,
    /// The operations whose delivered and in-flight spend count against the
    /// cap — `BillingOperation::teacher_gpu_operations`, stringly because the
    /// repository binds them straight into SQL.
    pub counted_operations: Vec<String>,
}

impl TeacherSpendReservation {
    /// Refuse if writing this reservation would push `already_spent_usd` over
    /// its cap.
    ///
    /// The policy — comparison and message — lives here in the service layer;
    /// the repository only executes it against the sum it read inside the
    /// locked admission transaction.
    pub fn admit_against(&self, already_spent_usd: f64) -> AppResult<()> {
        if teacher_gpu_spend_would_exceed_cap(self.cap_usd, already_spent_usd, self.cost_usd) {
            return Err(AppError::Forbidden {
                message: SPEND_CAP_MESSAGE.to_string(),
            });
        }
        Ok(())
    }
}

/// The reservation an on-policy admission must write, or `None` for a run with
/// no resident teacher to reserve for.
///
/// Derived from the same persisted `teacher` block and the same split the
/// terminal writers use, so the reservation and the charge that replaces it can
/// never disagree about whose GPU time it was.
pub fn admission_reservation(
    gpu_class: Option<&str>,
    teacher_config: Option<&serde_json::Value>,
    estimate: &ExtractionEstimate,
    cap: Option<f64>,
) -> Option<TeacherSpendReservation> {
    let share = teacher_serving_share(gpu_class, teacher_config);
    if share <= 0.0 {
        return None;
    }
    let estimate_seconds = (estimate.est_gpu_hours * 3600.0).round() as i32;
    let (_, (teacher_seconds, teacher_cost)) =
        split_teacher_serving_cost(estimate_seconds, estimate.est_cost_usd, share);

    Some(TeacherSpendReservation {
        gpu_seconds: teacher_seconds,
        cost_usd: teacher_cost,
        metadata: serde_json::json!({
            "reservation_pending": true,
            "reservation_reaped": false,
            "gpu_class": gpu_class,
            "teacher_device_share": share,
            "est_cost_usd": teacher_cost,
        }),
        cap_usd: cap,
        month_start: current_month_start(),
        counted_operations: platform_shared::enums::BillingOperation::teacher_gpu_operations()
            .iter()
            .map(|operation| operation.to_string())
            .collect(),
    })
}

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

/// Refuse a teacher-GPU run whose estimated cost would push the tenant's
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

    let operations: Vec<String> =
        platform_shared::enums::BillingOperation::teacher_gpu_operations()
            .iter()
            .map(|operation| operation.to_string())
            .collect();
    let spent = billing_repo
        .sum_delivered_and_in_flight_cost_since(tenant_id, &operations, current_month_start())
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

    /// Ledger stand-in that answers only the queries the cap makes, recording
    /// which operations it was asked about. Everything else is out of scope for a
    /// spend cap and would be a bug to call.
    struct LedgerStub {
        extraction_spend: f64,
        teacher_serving_spend: f64,
        undelivered_spend: f64,
        asked: Mutex<Vec<Vec<String>>>,
    }

    impl LedgerStub {
        fn empty() -> Self {
            Self {
                extraction_spend: 0.0,
                teacher_serving_spend: 0.0,
                undelivered_spend: 0.0,
                asked: Mutex::new(Vec::new()),
            }
        }

        fn with_spend(extraction_spend: f64) -> Self {
            Self {
                extraction_spend,
                ..Self::empty()
            }
        }

        fn with_teacher_serving_spend(teacher_serving_spend: f64) -> Self {
            Self {
                teacher_serving_spend,
                ..Self::empty()
            }
        }

        fn with_undelivered_spend(undelivered_spend: f64) -> Self {
            Self {
                undelivered_spend,
                ..Self::empty()
            }
        }

        fn reads_made(&self) -> Vec<Vec<String>> {
            self.asked.lock().expect("stub lock").clone()
        }
    }

    impl BillingEventRepository for LedgerStub {
        fn sum_delivered_and_in_flight_cost_since(
            &self,
            _tenant_id: Uuid,
            operations: &[String],
            _since: chrono::DateTime<chrono::Utc>,
        ) -> BoxFuture<'_, AppResult<f64>> {
            self.asked
                .lock()
                .expect("stub lock")
                .push(operations.to_vec());
            let spend = operations
                .iter()
                .map(|operation| match operation.as_str() {
                    "extraction" => self.extraction_spend,
                    "teacher_serving" => self.teacher_serving_spend + self.undelivered_spend,
                    _ => 0.0,
                })
                .sum();
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

        fn usage_by_operation(
            &self,
            _tenant_id: Uuid,
        ) -> BoxFuture<'_, AppResult<Vec<(String, f64, i64)>>> {
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

    /// The cap sums the same operations the workers write. If either side renames
    /// one, or a new teacher-GPU operation is added to the enum without being
    /// summed here, the recorded spend silently stops counting.
    ///
    /// One read, not one per ledger per operation: the relay moves a row from
    /// outbox to ledger in a single commit, so a row it delivers between two
    /// separate reads would be counted by neither.
    #[tokio::test]
    async fn the_cap_counts_every_teacher_gpu_operation_in_one_read() {
        let ledger = LedgerStub::with_spend(100.00);

        let _ = check_teacher_gpu_spend_cap(&ledger, Uuid::new_v4(), Some(1.00), 0.01).await;

        let expected: Vec<String> =
            platform_shared::enums::BillingOperation::teacher_gpu_operations()
                .iter()
                .map(|operation| operation.to_string())
                .collect();
        assert_eq!(ledger.reads_made(), vec![expected]);
    }

    /// A run holding a GPU right now has its charge in the outbox, not the
    /// ledger. A cap that reads only the ledger admits a second run — and a
    /// third, and a fourth — for as long as the first one is still training.
    #[tokio::test]
    async fn spend_still_in_the_outbox_counts_against_the_cap() {
        let ledger = LedgerStub::with_undelivered_spend(45.00);

        let refusal = check_teacher_gpu_spend_cap(&ledger, Uuid::new_v4(), Some(50.00), 6.00)
            .await
            .expect_err("45 of in-flight spend + 6 is over a 50 cap");

        assert!(matches!(refusal, AppError::Forbidden { .. }));
    }

    /// Delivered and in-flight spend are one budget line, not two.
    #[tokio::test]
    async fn delivered_and_in_flight_spend_accumulate_against_one_cap() {
        let ledger = LedgerStub {
            extraction_spend: 30.00,
            undelivered_spend: 25.00,
            ..LedgerStub::empty()
        };

        check_teacher_gpu_spend_cap(&ledger, Uuid::new_v4(), Some(50.00), 0.01)
            .await
            .expect_err("30 delivered + 25 in flight already exceeds a 50 cap");
    }

    /// On-policy spend lands under `teacher_serving`, not `extraction`. Counting
    /// only extraction left improve passes unbounded: each was admitted against a
    /// total that its own predecessors never contributed to.
    #[tokio::test]
    async fn recorded_on_policy_spend_alone_can_breach_the_cap() {
        let ledger = LedgerStub::with_teacher_serving_spend(45.00);

        let refusal = check_teacher_gpu_spend_cap(&ledger, Uuid::new_v4(), Some(50.00), 6.00)
            .await
            .expect_err("45 of on-policy spend + 6 is over a 50 cap");

        assert!(matches!(refusal, AppError::Forbidden { .. }));
    }

    /// The two operations share one budget line, so neither can be spent up to the
    /// cap independently of the other.
    #[tokio::test]
    async fn extraction_and_on_policy_spend_accumulate_against_one_cap() {
        let ledger = LedgerStub {
            extraction_spend: 30.00,
            teacher_serving_spend: 25.00,
            ..LedgerStub::empty()
        };

        check_teacher_gpu_spend_cap(&ledger, Uuid::new_v4(), Some(50.00), 0.01)
            .await
            .expect_err("30 + 25 already exceeds a 50 cap");
    }

    #[tokio::test]
    async fn an_unset_cap_never_queries_the_ledger() {
        let ledger = LedgerStub::with_spend(1_000_000.0);

        check_teacher_gpu_spend_cap(&ledger, Uuid::new_v4(), None, 999.0)
            .await
            .expect("no cap, no refusal");

        assert!(ledger.reads_made().is_empty());
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

    /// One statement, both ledgers. Splitting this into a ledger read and an
    /// outbox read is how a row the relay delivers mid-check vanishes from both
    /// sums — under-counting spend at the exact moment it becomes real.
    #[test]
    fn the_spend_sum_reads_both_ledgers_in_one_statement() {
        use crate::repositories::billing_event_repo::DELIVERED_AND_IN_FLIGHT_COST_SQL;

        assert!(DELIVERED_AND_IN_FLIGHT_COST_SQL.contains("billing_events"));
        assert!(DELIVERED_AND_IN_FLIGHT_COST_SQL.contains("billing_outbox"));
        assert!(DELIVERED_AND_IN_FLIGHT_COST_SQL.contains("delivered_at IS NULL"));
    }

    /// The refusal the repository executes inside the admission transaction is
    /// the same policy, same message, as the service's own pre-check.
    #[test]
    fn a_reservation_refuses_its_own_admission_over_the_cap() {
        let (plan, block) = admitted_improve_pass();
        let reservation = admission_reservation(
            Some(&plan.gpu_class),
            Some(&block),
            &plan.estimate,
            Some(50.0),
        )
        .expect("an on-policy run reserves");

        reservation
            .admit_against(50.0 - reservation.cost_usd)
            .expect("exactly at the cap is allowed");
        let refusal = reservation
            .admit_against(50.01 - reservation.cost_usd)
            .expect_err("one cent over the cap is refused");

        assert_eq!(refusal.to_string(), SPEND_CAP_MESSAGE);
    }

    // ── Admission reservations ──

    /// The plan and teacher block an admitted improve pass actually persists,
    /// produced by the code that persists them — a hand-built map is how a
    /// share that was always zero in production once passed its tests.
    fn admitted_improve_pass() -> (
        crate::services::teacher::on_policy::OnPolicyPlan,
        serde_json::Value,
    ) {
        use crate::services::teacher::on_policy::{attach_to_teacher_config, plan_on_policy};
        use chrono::Utc;
        use platform_db::models::Dataset;

        let dataset = Dataset {
            id: Uuid::new_v4(),
            tenant_id: Uuid::new_v4(),
            project_id: Uuid::new_v4(),
            name: "d".to_string(),
            storage_path: None,
            format: "chatml".to_string(),
            status: "approved".to_string(),
            pair_count: Some(100),
            stats: serde_json::json!({}),
            config: serde_json::json!({"teacher": {
                "host": "inference.example.com",
                "model": "Qwen/Qwen3-32B",
                "policy": "allowed",
            }}),
            error: None,
            prompt_tokens: None,
            completion_tokens: None,
            scored_completion_tokens: None,
            token_count_tokenizer_hash: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let plan = plan_on_policy(
            &dataset,
            "Qwen/Qwen3-8B",
            &serde_json::json!({}),
            "tenants/t/models/parent/",
            40.0,
            |_class| 6.00,
        )
        .expect("a hosted teacher is plannable");
        let block = attach_to_teacher_config(Some(serde_json::json!({"model": "t"})), Some(&plan))
            .expect("a block to live on")
            .expect("a block");
        (plan, block)
    }

    /// The reservation is the teacher's slice of the admission estimate — what
    /// the terminal split will bill if the run completes at exactly its quote —
    /// and it counts the same operations the cap sums.
    #[test]
    fn an_improve_pass_reserves_the_teachers_slice_of_its_estimate() {
        let (plan, block) = admitted_improve_pass();

        let reservation = admission_reservation(
            Some(&plan.gpu_class),
            Some(&block),
            &plan.estimate,
            Some(50.0),
        )
        .expect("an on-policy run reserves");

        let estimate_seconds = (plan.estimate.est_gpu_hours * 3600.0).round() as i32;
        let (_, (teacher_seconds, teacher_cost)) = split_teacher_serving_cost(
            estimate_seconds,
            plan.estimate.est_cost_usd,
            teacher_serving_share(Some(&plan.gpu_class), Some(&block)),
        );
        assert!(reservation.cost_usd > 0.0);
        assert_eq!(reservation.gpu_seconds, teacher_seconds);
        assert_eq!(reservation.cost_usd, teacher_cost);
        assert_eq!(reservation.cap_usd, Some(50.0));
        assert_eq!(reservation.metadata["reservation_pending"], true);
        assert_eq!(
            reservation.counted_operations,
            platform_shared::enums::BillingOperation::teacher_gpu_operations()
                .iter()
                .map(|operation| operation.to_string())
                .collect::<Vec<_>>()
        );
    }

    /// A capless tenant still reserves: the row is what bills a crashed run,
    /// not only what the cap reads.
    #[test]
    fn a_capless_tenant_still_reserves() {
        let (plan, block) = admitted_improve_pass();

        let reservation =
            admission_reservation(Some(&plan.gpu_class), Some(&block), &plan.estimate, None)
                .expect("the reservation exists to bill, not only to gate");

        assert_eq!(reservation.cap_usd, None);
        assert!(reservation.cost_usd > 0.0);
    }

    /// Every run without a resident teacher — plain SFT, plain distill, logit
    /// extraction — has no teacher share to reserve.
    #[test]
    fn a_run_with_no_resident_teacher_reserves_nothing() {
        let (plan, _) = admitted_improve_pass();

        assert!(admission_reservation(Some(&plan.gpu_class), None, &plan.estimate, None).is_none());
        assert!(
            admission_reservation(
                Some(&plan.gpu_class),
                Some(&serde_json::json!({"model": "t"})),
                &plan.estimate,
                None
            )
            .is_none()
        );
    }
}
