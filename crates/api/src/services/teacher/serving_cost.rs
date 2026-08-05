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

use platform_shared::enums::{DistillMethod, GpuClass};
use serde_json::Value;
use uuid::Uuid;

use crate::services::billing_outbox::enqueue_in_tx_with_id;

/// Devices a class of container is given, or one for a class we cannot name.
///
/// Taken from `GpuClass` rather than a table here: a new multi-device variant
/// added there without a matching entry here would price a two-card run as one.
fn device_count(gpu_class: Option<&str>) -> u32 {
    gpu_class
        .and_then(|class| class.to_lowercase().parse::<GpuClass>().ok())
        .map(GpuClass::device_count)
        .unwrap_or(1)
}

/// Fraction of a container's GPU cost the resident teacher accounts for.
///
/// Read from the job's persisted `teacher` block, which is where an admitted plan
/// lives — *not* from its hyperparams. `distill_method` reaches the trainer's
/// hyperparams only as a runtime dict the workflow assembles in memory; the
/// `hyperparams` column is written once at creation and never carries it, so a
/// share computed from that column is always zero.
///
/// Only an on-policy run has a teacher inside the container: every other mode
/// reaches its teacher through the tenant's own API key, or not at all. The
/// teacher holds all but one card and the student holds the last, which is exact
/// only because every device in a class is the same type.
pub fn teacher_serving_share(gpu_class: Option<&str>, teacher_config: Option<&Value>) -> f64 {
    let method = teacher_config
        .and_then(|block| block.pointer("/extraction/distill_method"))
        .and_then(Value::as_str);
    if method != Some(DistillMethod::OnPolicy.to_string().as_str()) {
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

/// Ledger id for the reservation written when an on-policy run is admitted,
/// holding the teacher's estimated share until a terminal charge replaces it.
///
/// Outcome-free, because at admission there is no outcome yet — and distinct
/// from every terminal id, because the terminal writer deletes this row rather
/// than colliding with it.
pub fn teacher_reservation_billing_event_id(job_id: Uuid) -> Uuid {
    Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("teacher-serving-reservation:{job_id}").as_bytes(),
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
///
/// Also retires the run's admission reservation in the same transaction: the
/// real charge replaces the estimate atomically, so no ordering of crashes can
/// leave the tenant holding both. A reservation the relay already reaped and
/// delivered has billed the teacher's time at the estimate — the terminal
/// teacher row is skipped then, because writing it would charge that time twice.
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

    let reservation_id = teacher_reservation_billing_event_id(job_id);
    let voided = sqlx::query(
        "DELETE FROM billing_outbox WHERE id = $1 AND tenant_id = $2 AND delivered_at IS NULL",
    )
    .bind(reservation_id)
    .bind(tenant_id)
    .execute(&mut **tx)
    .await?;

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

    if voided.rows_affected() == 0 {
        let delivered_charge: Option<f64> = sqlx::query_scalar(
            "SELECT cost_usd::FLOAT8 FROM billing_outbox \
             WHERE id = $1 AND tenant_id = $2 AND delivered_at IS NOT NULL",
        )
        .bind(reservation_id)
        .bind(tenant_id)
        .fetch_optional(&mut **tx)
        .await?;
        // A reservation delivered at zero was voided for a run that had not
        // started yet; if the run went on to hold a GPU anyway, its teacher
        // time is still unbilled and the terminal row must be written.
        if let Some(estimate_billed) = delivered_charge.filter(|charge| *charge > 0.0) {
            tracing::warn!(
                training_job_id = %job_id,
                estimate_billed,
                "Teacher reservation was already reaped and delivered; skipping the terminal teacher charge"
            );
            return Ok(());
        }
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

    /// The block an admitted improve pass actually persists, produced by the code
    /// that persists it rather than written out here — a hand-built map is how a
    /// share that is always zero in production passed its tests.
    fn on_policy() -> Value {
        use crate::services::teacher::on_policy::{attach_to_teacher_config, plan_on_policy};
        use chrono::Utc;
        use platform_db::models::Dataset;

        let dataset = Dataset {
            id: uuid::Uuid::new_v4(),
            tenant_id: uuid::Uuid::new_v4(),
            project_id: uuid::Uuid::new_v4(),
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

        attach_to_teacher_config(Some(serde_json::json!({"model": "t"})), Some(&plan))
            .expect("a block to live on")
            .expect("a block")
    }

    #[test]
    fn a_paired_class_gives_the_teacher_one_of_its_two_cards() {
        let block = on_policy();

        assert_eq!(
            teacher_serving_share(Some("a10080gb_dual"), Some(&block)),
            0.5
        );
        assert_eq!(teacher_serving_share(Some("h100_dual"), Some(&block)), 0.5);
    }

    #[test]
    fn a_class_name_is_matched_however_it_was_stored() {
        assert_eq!(
            teacher_serving_share(Some("A10080GB_DUAL"), Some(&on_policy())),
            0.5
        );
    }

    #[test]
    fn a_single_card_run_has_no_teacher_beside_the_student() {
        let block = on_policy();

        assert_eq!(teacher_serving_share(Some("a10080gb"), Some(&block)), 0.0);
        assert_eq!(teacher_serving_share(None, Some(&block)), 0.0);
    }

    /// Every run that predates fidelity upgrades, and every plain distill run.
    #[test]
    fn a_run_with_no_admitted_plan_is_billed_whole() {
        assert_eq!(teacher_serving_share(Some("a10080gb_dual"), None), 0.0);
        assert_eq!(
            teacher_serving_share(
                Some("a10080gb_dual"),
                Some(&serde_json::json!({"model": "t"}))
            ),
            0.0
        );
    }

    /// Every other mode reaches its teacher through the tenant's own key, so none
    /// of its GPU time is ours to count against the teacher budget.
    #[test]
    fn only_an_on_policy_run_splits_its_bill() {
        for block in [
            serde_json::json!({}),
            serde_json::json!({"extraction": {"distill_method": "logit"}}),
            serde_json::json!({"extraction": {"distill_method": null}}),
            serde_json::json!({"extraction": "logit"}),
        ] {
            assert_eq!(
                teacher_serving_share(Some("a10080gb_dual"), Some(&block)),
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

    /// Pinned against the worker's own test of the same input, like the terminal
    /// ids above: the worker deletes this row when it closes a run, and a scheme
    /// drift means it deletes nothing while the relay bills the estimate anyway.
    #[test]
    fn the_reservation_id_matches_the_one_the_worker_deletes() {
        let job: Uuid = JOB.parse().expect("a fixed id");

        assert_eq!(
            teacher_reservation_billing_event_id(job).to_string(),
            "158bff52-8237-5d12-a293-b3f29f0e2095"
        );
    }

    /// The reservation must never collide with a terminal row: the terminal
    /// writer deletes it by id, and a collision would delete a real charge.
    #[test]
    fn the_reservation_id_is_distinct_from_every_terminal_id() {
        let job: Uuid = JOB.parse().expect("a fixed id");

        for outcome in ["completed", "failed", "cancelled"] {
            assert_ne!(
                teacher_reservation_billing_event_id(job),
                training_billing_event_id(job, outcome)
            );
            assert_ne!(
                teacher_reservation_billing_event_id(job),
                teacher_serving_billing_event_id(job, outcome)
            );
        }
    }
}
