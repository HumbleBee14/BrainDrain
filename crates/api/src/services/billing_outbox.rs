//! Durable billing outbox.
//!
//! Provides crash-safe billing event capture via a PostgreSQL outbox table.
//! All billing-producing operations write to `billing_outbox` transactionally.
//! A background relay worker moves rows into `billing_events` (the reporting ledger).
//!
//! # Relay semantics
//! - Uses a single connection for advisory lock + claim + deliver + unlock.
//! - Advisory lock is always released on all exit paths (guard pattern).
//! - `SELECT ... FOR UPDATE SKIP LOCKED` within a transaction for row-level safety.
//! - Idempotent delivery: uses outbox row's `(id, created_at)` as billing_events
//!   composite PK with `ON CONFLICT ON CONSTRAINT billing_events_pkey DO NOTHING`.
//! - `created_at` is preserved from outbox to ledger (correct partition routing).
//! - Failed deliveries are retried every poll interval up to 5 attempts.

use sqlx::{Connection, PgPool};
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

/// Background relay that moves outbox rows into billing_events.
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

/// Return type for process_batch distinguishing "no rows" from "lock held".
enum BatchResult {
    /// Processed N rows (0 = no pending rows).
    Processed(usize),
    /// Skipped because another instance holds the advisory lock.
    LockHeld,
}

impl BillingOutboxRelay {
    pub fn new(db: PgPool, poll_interval: Duration) -> Self {
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let handle = tokio::spawn(relay_loop(db, poll_interval, shutdown_rx));
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

async fn relay_loop(db: PgPool, poll_interval: Duration, mut shutdown_rx: oneshot::Receiver<()>) {
    let mut interval = tokio::time::interval(poll_interval);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = interval.tick() => {
                if let Err(e) = process_batch(&db).await {
                    tracing::warn!(error = %e, "Billing outbox relay batch failed");
                }
            }
            _ = &mut shutdown_rx => {
                // Drain: keep processing until truly empty (not just lock-held)
                tracing::info!("Billing outbox relay draining...");
                for _ in 0..10 {
                    match process_batch(&db).await {
                        Ok(BatchResult::Processed(0)) => break,
                        Ok(BatchResult::Processed(_)) => continue,
                        Ok(BatchResult::LockHeld) => {
                            // Another instance has the lock — wait briefly and retry
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

/// Claim and deliver a batch. Advisory lock is always released on all exit paths.
async fn process_batch(db: &PgPool) -> Result<BatchResult, sqlx::Error> {
    let mut conn = db.acquire().await?;

    // Try advisory lock on this connection
    let locked: (bool,) = sqlx::query_as("SELECT pg_try_advisory_lock($1)")
        .bind(RELAY_LOCK_ID)
        .fetch_one(&mut *conn)
        .await?;

    if !locked.0 {
        return Ok(BatchResult::LockHeld);
    }

    // Guard: always unlock on the same connection, even on error
    let result = do_relay_work(&mut conn).await;

    // Release advisory lock on the SAME connection that acquired it
    let _ = sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(RELAY_LOCK_ID)
        .execute(&mut *conn)
        .await;

    result
}

/// The actual relay work, separated so the caller can guarantee advisory unlock.
async fn do_relay_work(
    conn: &mut sqlx::pool::PoolConnection<sqlx::Postgres>,
) -> Result<BatchResult, sqlx::Error> {
    let mut tx = conn.begin().await?;

    let rows = sqlx::query_as::<_, OutboxRow>(
        "SELECT id, tenant_id, operation, resource_id, tokens_in, tokens_out, \
                gpu_seconds, cost_usd, metadata, created_at \
         FROM billing_outbox \
         WHERE delivered_at IS NULL AND attempt_count < $1 \
         ORDER BY created_at \
         LIMIT $2 \
         FOR UPDATE SKIP LOCKED",
    )
    .bind(MAX_ATTEMPTS)
    .bind(RELAY_BATCH_SIZE)
    .fetch_all(&mut *tx)
    .await?;

    if rows.is_empty() {
        tx.commit().await?;
        return Ok(BatchResult::Processed(0));
    }

    let count = rows.len();
    let mut delivered = Vec::with_capacity(count);
    let mut failed: Vec<(Uuid, String)> = Vec::new();

    for row in &rows {
        match deliver_to_ledger(&mut tx, row).await {
            Ok(()) => delivered.push(row.id),
            Err(e) => failed.push((row.id, e.to_string())),
        }
    }

    if !delivered.is_empty() {
        sqlx::query("UPDATE billing_outbox SET delivered_at = NOW() WHERE id = ANY($1)")
            .bind(&delivered)
            .execute(&mut *tx)
            .await?;
    }

    for (id, error) in &failed {
        sqlx::query(
            "UPDATE billing_outbox \
             SET attempt_count = attempt_count + 1, last_error = $1 \
             WHERE id = $2",
        )
        .bind(error)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;

    if !delivered.is_empty() {
        tracing::info!(
            delivered = delivered.len(),
            failed = failed.len(),
            "Billing outbox relay batch processed"
        );
    }

    Ok(BatchResult::Processed(count))
}

/// Insert a single outbox row into the billing_events ledger.
///
/// Uses the outbox row's `(id, created_at)` as the billing_events composite PK.
/// `ON CONFLICT ... DO NOTHING` makes delivery idempotent. `created_at` is
/// preserved from the outbox so the row lands in the correct monthly partition.
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

/// Cleanup delivered outbox rows older than the retention period.
#[allow(dead_code)]
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
};
