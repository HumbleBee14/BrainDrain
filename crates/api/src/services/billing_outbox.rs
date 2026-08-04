//! Durable billing outbox.
//!
//! Provides crash-safe billing event capture via a PostgreSQL outbox table.
//! A background relay moves rows into `billing_events` (the reporting ledger).
//!
//! # Relay semantics
//! - Single connection for advisory lock + claim + deliver + unlock.
//! - Advisory lock always released (guard pattern) on all exit paths.
//! - `FOR UPDATE SKIP LOCKED` within a transaction for row-level safety.
//! - Per-row savepoints: one poison row cannot stall the batch.
//! - Idempotent delivery via `ON CONFLICT ON CONSTRAINT billing_events_pkey`.
//! - `created_at` preserved from outbox for correct partition routing.
//! - Relay loops until empty per tick (not one batch per tick).
//! - Failed deliveries retried every poll interval up to 5 attempts.
//!
//! # Pending reservations
//! Rows whose final cost is only known after an async operation are written
//! first as a *pending* reservation carrying a conservative fallback charge, and
//! corrected with actuals afterwards. The relay withholds such a row from the
//! ledger until it is finalized, and reaps it at the fallback if nothing ever
//! finalizes it. Streaming inference reserves here (`*_stream_pending`);
//! extraction reserves in the worker that owns the GPU pass, the same place
//! training's own charge is written from — so this module holds only the relay
//! side of that contract (`reap_stale_pending_extractions` and the delivery
//! filter below).

use sqlx::PgPool;
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::oneshot;
use uuid::Uuid;

/// Enqueue a billing event into the durable outbox.
///
/// The INSERT is awaited — the event is on disk before this returns.
#[allow(clippy::too_many_arguments)]
pub async fn enqueue(
    db: &PgPool,
    tenant_id: Uuid,
    operation: &str,
    resource_id: Option<Uuid>,
    tokens_in: i64,
    tokens_out: i64,
    gpu_seconds: i32,
    cost_usd: f64,
    metadata: serde_json::Value,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO billing_outbox \
         (tenant_id, operation, resource_id, tokens_in, tokens_out, gpu_seconds, cost_usd, metadata) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(tenant_id)
    .bind(operation)
    .bind(resource_id)
    .bind(tokens_in)
    .bind(tokens_out)
    .bind(gpu_seconds)
    .bind(cost_usd)
    .bind(metadata)
    .execute(db)
    .await?;
    Ok(())
}

/// Enqueue a pending streaming inference row before the SSE response starts.
///
/// The row is inserted durably with a conservative fallback charge so a crash
/// during streaming can still be finalized by the stale-pending reaper.
#[allow(clippy::too_many_arguments)]
pub async fn enqueue_stream_pending(
    db: &PgPool,
    tenant_id: Uuid,
    resource_id: Option<Uuid>,
    fallback_tokens_in: i64,
    fallback_tokens_out: i64,
    fallback_cost_usd: f64,
    metadata: serde_json::Value,
) -> Result<Uuid, sqlx::Error> {
    let row_id = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO billing_outbox \
         (id, tenant_id, operation, resource_id, tokens_in, tokens_out, gpu_seconds, cost_usd, metadata) \
         VALUES ($1, $2, 'inference', $3, $4, $5, 0, $6, $7)",
    )
    .bind(row_id)
    .bind(tenant_id)
    .bind(resource_id)
    .bind(fallback_tokens_in)
    .bind(fallback_tokens_out)
    .bind(fallback_cost_usd)
    .bind(metadata_with_stream_state(metadata, true, false))
    .execute(db)
    .await?;

    Ok(row_id)
}

/// Finalize a pending streaming row with actual token usage.
#[allow(clippy::too_many_arguments)]
pub async fn finalize_stream_pending(
    db: &PgPool,
    row_id: Uuid,
    tokens_in: i64,
    tokens_out: i64,
    cost_usd: f64,
    metadata: serde_json::Value,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE billing_outbox \
         SET tokens_in = $2, tokens_out = $3, cost_usd = $4, metadata = $5 \
         WHERE id = $1 \
           AND delivered_at IS NULL \
           AND COALESCE((metadata->>'stream_pending')::boolean, false) = true",
    )
    .bind(row_id)
    .bind(tokens_in)
    .bind(tokens_out)
    .bind(cost_usd)
    .bind(metadata_with_stream_state(metadata, false, false))
    .execute(db)
    .await?;

    Ok(())
}

/// Cancel a pending reservation that will never be finalized because no
/// billable work occurred (e.g. the upstream inference call failed).
///
/// Best-effort: only removes rows still pending and undelivered. A failure here
/// merely leaves the row for the reaper to deliver as the conservative
/// fallback, so we log and swallow rather than fail the caller.
pub async fn cancel_stream_pending(db: &PgPool, row_id: Uuid) {
    let result = sqlx::query(
        "DELETE FROM billing_outbox \
         WHERE id = $1 \
           AND delivered_at IS NULL \
           AND COALESCE((metadata->>'stream_pending')::boolean, false) = true",
    )
    .bind(row_id)
    .execute(db)
    .await;

    if let Err(e) = result {
        tracing::error!(error = %e, row_id = %row_id, "Failed to cancel pending billing reservation");
    }
}

/// Enqueue into billing_outbox within an existing transaction.
#[allow(clippy::too_many_arguments)]
pub async fn enqueue_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: Uuid,
    operation: &str,
    resource_id: Option<Uuid>,
    tokens_in: i64,
    tokens_out: i64,
    gpu_seconds: i32,
    cost_usd: f64,
    metadata: serde_json::Value,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO billing_outbox \
         (tenant_id, operation, resource_id, tokens_in, tokens_out, gpu_seconds, cost_usd, metadata) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(tenant_id)
    .bind(operation)
    .bind(resource_id)
    .bind(tokens_in)
    .bind(tokens_out)
    .bind(gpu_seconds)
    .bind(cost_usd)
    .bind(metadata)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

fn metadata_with_stream_state(
    metadata: serde_json::Value,
    stream_pending: bool,
    stream_reaped: bool,
) -> serde_json::Value {
    let mut metadata = match metadata {
        serde_json::Value::Object(map) => map,
        _ => serde_json::Map::new(),
    };
    metadata.insert(
        "stream_pending".to_string(),
        serde_json::Value::Bool(stream_pending),
    );
    metadata.insert(
        "stream_reaped".to_string(),
        serde_json::Value::Bool(stream_reaped),
    );
    serde_json::Value::Object(metadata)
}

pub struct BillingOutboxRelay {
    shutdown: Mutex<Option<ShutdownHandle>>,
}

struct ShutdownHandle {
    signal: oneshot::Sender<()>,
    handle: tokio::task::JoinHandle<()>,
}

const MAX_ATTEMPTS: i32 = 5;
const RELAY_BATCH_SIZE: i64 = 500;
const RELAY_LOCK_ID: i64 = 900_200_001;
const STREAM_PENDING_STALE_SECS: i64 = 300;
/// Extraction is a GPU batch job, not an SSE response — it can legitimately
/// run far longer than `STREAM_PENDING_STALE_SECS` allows, so it gets its own,
/// much longer staleness window before the reaper trusts its fallback cost.
///
/// This window must exceed the longest run the extraction activity itself
/// permits (`timeout_teacher_extraction_hours`, 6h by default, over two
/// attempts), because reaping a *live* run delivers its estimate to the ledger
/// and the worker's later finalization can no longer correct it — the tenant
/// would then be billed the quote instead of the measured GPU time on every run
/// past the window. A day is comfortably past that and still bounded.
const EXTRACTION_PENDING_STALE_SECS: i64 = 86_400;
/// How often the relay prunes delivered outbox rows. Coarse on purpose — the
/// buffer only needs bounding, not tight trimming.
const CLEANUP_INTERVAL: Duration = Duration::from_secs(3600);

enum BatchResult {
    Processed(usize),
    LockHeld,
}

impl BillingOutboxRelay {
    pub fn new(db: PgPool, poll_interval: Duration, retention_days: i32) -> Self {
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let handle = tokio::spawn(relay_loop(db, poll_interval, retention_days, shutdown_rx));
        Self {
            shutdown: Mutex::new(Some(ShutdownHandle {
                signal: shutdown_tx,
                handle,
            })),
        }
    }

    pub async fn shutdown(&self) {
        let handle = self
            .shutdown
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();

        if let Some(handle) = handle {
            let _ = handle.signal.send(());
            if let Err(e) = handle.handle.await {
                tracing::error!(error = %e, "Billing outbox relay task panicked");
            }
            tracing::info!("Billing outbox relay shutdown complete");
        }
    }
}

async fn relay_loop(
    db: PgPool,
    poll_interval: Duration,
    retention_days: i32,
    mut shutdown_rx: oneshot::Receiver<()>,
) {
    let mut interval = tokio::time::interval(poll_interval);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    // Coarse cadence for pruning delivered rows. Consume the immediate first
    // tick so we don't prune the instant the process boots.
    let mut cleanup_interval = tokio::time::interval(CLEANUP_INTERVAL);
    cleanup_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    cleanup_interval.tick().await;

    loop {
        tokio::select! {
            // Prune delivered outbox rows past the retention window so the
            // buffer table cannot grow without bound. Disabled when
            // retention_days <= 0. The permanent ledger keeps the data; these
            // are just already-relayed buffer rows.
            _ = cleanup_interval.tick(), if retention_days > 0 => {
                match cleanup_delivered(&db, retention_days).await {
                    Ok(n) if n > 0 => {
                        tracing::info!(pruned = n, retention_days, "Pruned delivered billing-outbox rows");
                    }
                    Ok(_) => {}
                    Err(e) => tracing::warn!(error = %e, "Billing outbox cleanup failed"),
                }
            }
            _ = interval.tick() => {
                // Drain all pending batches per tick (not just one)
                loop {
                    match process_batch(&db).await {
                        Ok(BatchResult::Processed(0)) => break,
                        Ok(BatchResult::Processed(_)) => continue,
                        Ok(BatchResult::LockHeld) => break,
                        Err(e) => {
                            tracing::warn!(error = %e, "Billing outbox relay batch failed");
                            break;
                        }
                    }
                }
            }
            _ = &mut shutdown_rx => {
                tracing::info!("Billing outbox relay draining...");
                for _ in 0..100 {
                    match process_batch(&db).await {
                        Ok(BatchResult::Processed(0)) => break,
                        Ok(BatchResult::Processed(_)) => continue,
                        Ok(BatchResult::LockHeld) => {
                            tokio::time::sleep(Duration::from_millis(200)).await;
                            continue;
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "Drain batch failed");
                            break;
                        }
                    }
                }
                tracing::info!("Billing outbox relay stopped");
                return;
            }
        }
    }
}

/// Claim and deliver a batch.
///
/// Uses `pg_try_advisory_xact_lock` (transaction-scoped) instead of session-level
/// advisory locks. This is compatible with PgBouncer transaction mode, where
/// server connections may be reassigned between transactions. The lock is
/// automatically released when the transaction commits or rolls back.
async fn process_batch(db: &PgPool) -> Result<BatchResult, sqlx::Error> {
    let mut tx = db.begin().await?;

    let locked: (bool,) = sqlx::query_as("SELECT pg_try_advisory_xact_lock($1)")
        .bind(RELAY_LOCK_ID)
        .fetch_one(&mut *tx)
        .await?;

    if !locked.0 {
        tx.rollback().await?;
        return Ok(BatchResult::LockHeld);
    }

    let result = do_relay_work(&mut tx).await;

    match &result {
        Ok(_) => tx.commit().await?,
        Err(_) => tx.rollback().await?,
    }

    result
}

/// Relay work with per-row savepoints so one poison row cannot stall the batch.
/// The advisory xact lock is held for the entire transaction (caller commits).
async fn do_relay_work(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<BatchResult, sqlx::Error> {
    reap_stale_pending_streams(tx).await?;
    reap_stale_pending_extractions(tx).await?;

    let rows = sqlx::query_as::<_, OutboxRow>(
        "SELECT id, tenant_id, operation, resource_id, tokens_in, tokens_out, \
                gpu_seconds, cost_usd, metadata, created_at \
         FROM billing_outbox \
         WHERE delivered_at IS NULL \
           AND attempt_count < $1 \
           AND COALESCE((metadata->>'stream_pending')::boolean, false) = false \
           AND COALESCE((metadata->>'extraction_pending')::boolean, false) = false \
         ORDER BY created_at \
         LIMIT $2 \
         FOR UPDATE SKIP LOCKED",
    )
    .bind(MAX_ATTEMPTS)
    .bind(RELAY_BATCH_SIZE)
    .fetch_all(&mut **tx)
    .await?;

    if rows.is_empty() {
        return Ok(BatchResult::Processed(0));
    }

    let count = rows.len();
    let mut delivered = Vec::with_capacity(count);
    let mut failed: Vec<(Uuid, String)> = Vec::new();

    for row in &rows {
        // Savepoint per row: if delivery fails, rollback only this row's work,
        // not the entire batch. The FOR UPDATE lock is still held.
        sqlx::query("SAVEPOINT row_delivery")
            .execute(&mut **tx)
            .await?;

        match deliver_to_ledger(tx, row).await {
            Ok(()) => {
                sqlx::query("RELEASE SAVEPOINT row_delivery")
                    .execute(&mut **tx)
                    .await?;
                delivered.push(row.id);
            }
            Err(e) => {
                sqlx::query("ROLLBACK TO SAVEPOINT row_delivery")
                    .execute(&mut **tx)
                    .await?;
                failed.push((row.id, e.to_string()));
            }
        }
    }

    // Mark delivered
    if !delivered.is_empty() {
        sqlx::query("UPDATE billing_outbox SET delivered_at = NOW() WHERE id = ANY($1)")
            .bind(&delivered)
            .execute(&mut **tx)
            .await?;
    }

    // Mark failed with retry metadata
    for (id, error) in &failed {
        sqlx::query(
            "UPDATE billing_outbox \
             SET attempt_count = attempt_count + 1, last_error = $1 \
             WHERE id = $2",
        )
        .bind(error)
        .bind(id)
        .execute(&mut **tx)
        .await?;
    }

    if !delivered.is_empty() || !failed.is_empty() {
        tracing::info!(
            delivered = delivered.len(),
            failed = failed.len(),
            "Billing outbox relay batch processed"
        );
    }

    Ok(BatchResult::Processed(count))
}

async fn reap_stale_pending_streams(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE billing_outbox \
         SET metadata = jsonb_set(
                 jsonb_set(metadata, '{stream_pending}', 'false'::jsonb, true),
                 '{stream_reaped}', 'true'::jsonb, true
             ) \
         WHERE delivered_at IS NULL \
           AND COALESCE((metadata->>'stream_pending')::boolean, false) = true \
           AND created_at < NOW() - make_interval(secs => $1)",
    )
    .bind(STREAM_PENDING_STALE_SECS as f64)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Same reap as `reap_stale_pending_streams`, for the extraction reservations
/// the ML worker writes before it starts a teacher's GPU pass — covers a
/// crashed, terminated or cancelled pass that never came back to replace its
/// estimate with what the GPU actually cost.
async fn reap_stale_pending_extractions(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE billing_outbox \
         SET metadata = jsonb_set(
                 jsonb_set(metadata, '{extraction_pending}', 'false'::jsonb, true),
                 '{extraction_reaped}', 'true'::jsonb, true
             ) \
         WHERE delivered_at IS NULL \
           AND COALESCE((metadata->>'extraction_pending')::boolean, false) = true \
           AND created_at < NOW() - make_interval(secs => $1)",
    )
    .bind(EXTRACTION_PENDING_STALE_SECS as f64)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Idempotent delivery: uses outbox `(id, created_at)` as billing_events composite PK.
async fn deliver_to_ledger(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    row: &OutboxRow,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO billing_events \
         (id, tenant_id, operation, resource_id, tokens_in, tokens_out, \
          gpu_seconds, cost_usd, metadata, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) \
         ON CONFLICT ON CONSTRAINT billing_events_pkey DO NOTHING",
    )
    .bind(row.id)
    .bind(row.tenant_id)
    .bind(&row.operation)
    .bind(row.resource_id)
    .bind(row.tokens_in)
    .bind(row.tokens_out)
    .bind(row.gpu_seconds)
    .bind(row.cost_usd)
    .bind(&row.metadata)
    .bind(row.created_at)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Delete outbox rows already delivered longer ago than `retention_days`.
/// Invoked periodically by the relay loop (see `CLEANUP_INTERVAL`).
pub async fn cleanup_delivered(db: &PgPool, retention_days: i32) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        "DELETE FROM billing_outbox \
         WHERE delivered_at IS NOT NULL \
           AND delivered_at < NOW() - make_interval(days => $1)",
    )
    .bind(retention_days)
    .execute(db)
    .await?;
    Ok(result.rows_affected())
}

#[derive(sqlx::FromRow)]
struct OutboxRow {
    id: Uuid,
    tenant_id: Uuid,
    operation: String,
    resource_id: Option<Uuid>,
    tokens_in: i64,
    tokens_out: i64,
    gpu_seconds: i32,
    cost_usd: f64,
    metadata: serde_json::Value,
    created_at: chrono::DateTime<chrono::Utc>,
}

const _: () = {
    assert!(MAX_ATTEMPTS > 0);
    assert!(MAX_ATTEMPTS <= 10);
    assert!(RELAY_BATCH_SIZE > 0);
    assert!(RELAY_BATCH_SIZE <= 10_000);
    assert!(STREAM_PENDING_STALE_SECS > 0);
    assert!(STREAM_PENDING_STALE_SECS <= 600);
    assert!(EXTRACTION_PENDING_STALE_SECS > STREAM_PENDING_STALE_SECS);
    assert!(EXTRACTION_PENDING_STALE_SECS <= 86_400);
};

#[cfg(test)]
mod tests {
    use super::*;

    // ── Metadata construction ──

    #[test]
    fn metadata_with_stream_state_sets_flags() {
        let meta = serde_json::json!({"model": "llama-3"});
        let result = metadata_with_stream_state(meta, true, false);
        assert_eq!(result["stream_pending"], true);
        assert_eq!(result["stream_reaped"], false);
        assert_eq!(result["model"], "llama-3");
    }

    #[test]
    fn metadata_with_stream_state_handles_non_object() {
        let result = metadata_with_stream_state(serde_json::Value::Null, true, true);
        assert_eq!(result["stream_pending"], true);
        assert_eq!(result["stream_reaped"], true);
    }

    #[test]
    fn metadata_finalize_clears_pending() {
        let meta = serde_json::json!({"model": "llama-3"});
        let pending = metadata_with_stream_state(meta, true, false);
        assert_eq!(pending["stream_pending"], true);

        // Finalize should set pending=false
        let finalized = metadata_with_stream_state(pending, false, false);
        assert_eq!(finalized["stream_pending"], false);
        assert_eq!(finalized["stream_reaped"], false);
        assert_eq!(finalized["model"], "llama-3");
    }

    #[test]
    fn metadata_reap_marks_reaped() {
        let meta = serde_json::json!({"model": "llama-3"});
        let pending = metadata_with_stream_state(meta, true, false);
        let reaped = metadata_with_stream_state(pending, false, true);
        assert_eq!(reaped["stream_pending"], false);
        assert_eq!(reaped["stream_reaped"], true);
    }

    // ── Constants and invariants ──

    #[test]
    fn relay_constants_are_reasonable() {
        assert_eq!(MAX_ATTEMPTS, 5);
        assert_eq!(RELAY_BATCH_SIZE, 500);
        assert_eq!(STREAM_PENDING_STALE_SECS, 300);
        assert_eq!(EXTRACTION_PENDING_STALE_SECS, 86_400);
    }

    /// A reservation reaped while its run is still going bills the estimate and
    /// can never be corrected, so the window has to outlast the longest
    /// extraction the worker will run: `timeout_teacher_extraction_hours` (6h)
    /// across `maximum_attempts` (2).
    #[test]
    fn extraction_window_outlasts_the_longest_extraction_the_worker_allows() {
        let longest_extraction_secs = 6 * 3600 * 2;
        assert!(EXTRACTION_PENDING_STALE_SECS > longest_extraction_secs);
    }

    #[test]
    fn relay_lock_id_is_distinct_from_idempotency_lock() {
        // The idempotency cleanup uses 900_100_001.
        // Relay must use a different lock ID to avoid conflicts.
        assert_ne!(RELAY_LOCK_ID, 900_100_001);
    }

    // ── BatchResult ──

    #[test]
    fn batch_result_distinguishes_lock_held_from_empty() {
        let lock_held = BatchResult::LockHeld;
        let empty = BatchResult::Processed(0);
        let some = BatchResult::Processed(5);

        assert!(matches!(lock_held, BatchResult::LockHeld));
        assert!(matches!(empty, BatchResult::Processed(0)));
        assert!(matches!(some, BatchResult::Processed(5)));
    }
}
