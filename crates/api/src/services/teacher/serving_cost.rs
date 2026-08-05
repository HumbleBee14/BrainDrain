//! Dividing one training container's bill between the student and the teacher
//! sharing it, for the runs the control plane closes out itself.
//!
//! The worker does this for every run it finishes. A run the worker never gets
//! to finish — reaped after its worker died, or cancelled by its owner — is
//! closed out here instead, and has to be split the same way or the teacher-GPU
//! spend cap cannot see it: the cap sums `extraction` and `teacher_serving`, so
//! a dual-GPU improve pass billed wholly as `training` is teacher time nobody
//! counts. Losing no revenue, and losing the cap.
//!
//! Every rule below mirrors `teacher_serving_share` / `split_teacher_serving_cost`
//! in the worker's `train_model.py`, including the ledger ids, so that whichever
//! side writes second is a no-op rather than a second charge.

use serde_json::Value;
use uuid::Uuid;

use crate::services::billing_outbox::enqueue_in_tx_with_id;

/// Hyperparam naming the distillation method, written by the platform.
const DISTILL_METHOD_HYPERPARAM: &str = "distill_method";

/// The method whose teacher lives inside the training container.
const ON_POLICY_METHOD: &str = "on_policy";

/// Devices in each multi-device class. Mirrors `GPU_DEVICE_COUNTS` in the
/// worker's constants; a class absent here holds one card.
const GPU_DEVICE_COUNTS: &[(&str, u32)] = &[("a10080gb_dual", 2), ("h100_dual", 2)];

fn device_count(gpu_class: Option<&str>) -> u32 {
    let class = gpu_class.unwrap_or_default().to_lowercase();
    GPU_DEVICE_COUNTS
        .iter()
        .find(|(name, _)| *name == class)
        .map(|(_, count)| *count)
        .unwrap_or(1)
}

/// Fraction of a container's GPU cost the resident teacher accounts for.
///
/// Only an on-policy run has one: every other mode reaches its teacher through
/// the tenant's own API key, or not at all. The teacher holds all but one card
/// and the student holds the last, which is exact only because every device in a
/// class is the same type.
pub fn teacher_serving_share(gpu_class: Option<&str>, hyperparams: &Value) -> f64 {
    if hyperparams
        .get(DISTILL_METHOD_HYPERPARAM)
        .and_then(Value::as_str)
        != Some(ON_POLICY_METHOD)
    {
        return 0.0;
    }
    let devices = device_count(gpu_class);
    if devices < 2 {
        return 0.0;
    }
    f64::from(devices - 1) / f64::from(devices)
}

/// Divide one container's bill into (student, teacher) halves.
///
/// The teacher's share is computed and the student's is the remainder, so the
/// two rows re-add to exactly what the container cost. Splitting both
/// independently would let rounding invent or lose a cent.
pub fn split_teacher_serving_cost(
    gpu_seconds: i32,
    cost_usd: f64,
    share: f64,
) -> ((i32, f64), (i32, f64)) {
    if share <= 0.0 {
        return ((gpu_seconds, cost_usd), (0, 0.0));
    }
    let teacher_seconds = (f64::from(gpu_seconds) * share).round() as i32;
    let teacher_cost = (cost_usd * share * 100.0).round() / 100.0;
    (
        (
            gpu_seconds - teacher_seconds,
            ((cost_usd - teacher_cost) * 100.0).round() / 100.0,
        ),
        (teacher_seconds, teacher_cost),
    )
}

/// Ledger id for one run's student charge. Derived rather than generated so that
/// a worker writing the same outcome later collides with this row instead of
/// charging the tenant a second time.
pub fn training_billing_event_id(job_id: Uuid, outcome: &str) -> Uuid {
    Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("training-billing:{job_id}:{outcome}").as_bytes(),
    )
}

/// Ledger id for the same run's teacher charge.
pub fn teacher_serving_billing_event_id(job_id: Uuid, outcome: &str) -> Uuid {
    Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("teacher-serving-billing:{job_id}:{outcome}").as_bytes(),
    )
}

/// What one closed-out run cost, and how much of it the teacher accounts for.
pub struct RunCharge {
    pub gpu_seconds: i32,
    pub cost_usd: f64,
    pub metadata: Value,
    /// From `teacher_serving_share`. Zero for a run with no resident teacher.
    pub teacher_share: f64,
}

/// Append the outbox row(s) closing out one run, inside the caller's transaction.
///
/// Two rows when a teacher shared the container, so the spend cap sees its time;
/// one otherwise. Both are written with the caller's transaction so a crash can
/// never commit the status change without the charge.
pub async fn enqueue_run_billing(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: Uuid,
    job_id: Uuid,
    outcome: &str,
    charge: RunCharge,
) -> Result<(), sqlx::Error> {
    let RunCharge {
        gpu_seconds,
        cost_usd,
        metadata,
        teacher_share,
    } = charge;
    let ((student_seconds, student_cost), (teacher_seconds, teacher_cost)) =
        split_teacher_serving_cost(gpu_seconds, cost_usd, teacher_share);

    enqueue_in_tx_with_id(
        tx,
        training_billing_event_id(job_id, outcome),
        tenant_id,
        "training",
        Some(job_id),
        student_seconds,
        student_cost,
        metadata.clone(),
    )
    .await?;

    if teacher_share <= 0.0 {
        return Ok(());
    }

    let mut teacher_metadata = metadata;
    if let Some(map) = teacher_metadata.as_object_mut() {
        map.insert(
            "teacher_device_share".to_string(),
            serde_json::json!(teacher_share),
        );
    }

    enqueue_in_tx_with_id(
        tx,
        teacher_serving_billing_event_id(job_id, outcome),
        tenant_id,
        "teacher_serving",
        Some(job_id),
        teacher_seconds,
        teacher_cost,
        teacher_metadata,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    const JOB: &str = "11111111-1111-1111-1111-111111111111";

    fn on_policy() -> Value {
        serde_json::json!({"distill_method": "on_policy"})
    }

    #[test]
    fn a_paired_class_gives_the_teacher_one_of_its_two_cards() {
        assert_eq!(
            teacher_serving_share(Some("a10080gb_dual"), &on_policy()),
            0.5
        );
        assert_eq!(teacher_serving_share(Some("h100_dual"), &on_policy()), 0.5);
    }

    #[test]
    fn a_class_name_is_matched_however_it_was_stored() {
        assert_eq!(
            teacher_serving_share(Some("A10080GB_DUAL"), &on_policy()),
            0.5
        );
    }

    #[test]
    fn a_single_card_run_has_no_teacher_beside_the_student() {
        assert_eq!(teacher_serving_share(Some("a10080gb"), &on_policy()), 0.0);
        assert_eq!(teacher_serving_share(None, &on_policy()), 0.0);
    }

    /// Every other mode reaches its teacher through the tenant's own key, so none
    /// of its GPU time is ours to count against the teacher budget.
    #[test]
    fn only_an_on_policy_run_splits_its_bill() {
        for hyperparams in [
            serde_json::json!({}),
            serde_json::json!({"distill_method": "logit"}),
            serde_json::json!({"distill_method": null}),
        ] {
            assert_eq!(
                teacher_serving_share(Some("a10080gb_dual"), &hyperparams),
                0.0
            );
        }
    }

    #[test]
    fn the_two_rows_re_add_to_what_the_container_cost() {
        let ((student_seconds, student_cost), (teacher_seconds, teacher_cost)) =
            split_teacher_serving_cost(3601, 6.01, 0.5);

        assert_eq!(student_seconds + teacher_seconds, 3601);
        assert_eq!(
            ((student_cost + teacher_cost) * 100.0).round() / 100.0,
            6.01
        );
    }

    #[test]
    fn a_run_with_no_teacher_is_billed_whole() {
        assert_eq!(
            split_teacher_serving_cost(1800, 3.00, 0.0),
            ((1800, 3.00), (0, 0.0))
        );
    }

    /// Pinned against the worker's own test of the same inputs. If either side's
    /// scheme changes, the two stop colliding and a reaped run that the worker
    /// later closes out is billed twice.
    #[test]
    fn the_ledger_ids_match_the_ones_the_worker_writes() {
        let job: Uuid = JOB.parse().expect("a fixed id");

        assert_eq!(
            training_billing_event_id(job, "failed").to_string(),
            "370e6ec6-c631-542b-921c-1a3e9e462fbc"
        );
        assert_eq!(
            teacher_serving_billing_event_id(job, "failed").to_string(),
            "1f8b1c51-493f-55ec-b240-d742d5ae9a13"
        );
        assert_eq!(
            training_billing_event_id(job, "cancelled").to_string(),
            "e79ce3d1-9ac2-50df-9c9f-ffe652223541"
        );
        assert_eq!(
            teacher_serving_billing_event_id(job, "cancelled").to_string(),
            "fefbc12a-0217-5bb1-987d-dee962b2a406"
        );
    }

    #[test]
    fn the_student_and_teacher_rows_are_distinct() {
        let job: Uuid = JOB.parse().expect("a fixed id");

        assert_ne!(
            training_billing_event_id(job, "failed"),
            teacher_serving_billing_event_id(job, "failed")
        );
        assert_ne!(
            training_billing_event_id(job, "failed"),
            training_billing_event_id(job, "cancelled")
        );
    }
}
