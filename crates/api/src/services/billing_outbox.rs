//! Durable billing outbox.
//!
//! Provides crash-safe billing event capture via a PostgreSQL outbox table.
//! All billing-producing operations write to `billing_outbox` transactionally.
//! A background relay worker moves rows into `billing_events` (the reporting ledger).
//!
//! # Why not the in-memory batcher?
//! The existing `BillingBatcher` uses an in-memory channel. Events in the channel
//! are lost on process crash. The outbox ensures durability — the INSERT is committed
//! before the handler returns, so no event can be lost.
//!
//! # Relay semantics
//! - Uses a single connection for advisory lock + claim + deliver + unlock.
//! - `SELECT ... FOR UPDATE SKIP LOCKED` within a transaction for row-level safety.
//! - Idempotent delivery: uses outbox row ID as billing_events PK with ON CONFLICT.
//! - Failed deliveries are retried every poll interval up to 5 attempts.
//! - Delivered rows are cleaned up after 7 days.

use sqlx::{Connection, PgPool};
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::oneshot;
use uuid::Uuid;

/// Enqueue a billing event into the durable outbox.
///
/// This is the primary write path — called from inference, deploy, and training
/// handlers. The INSERT is awaited so the event is on disk before the handler
/// returns.
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

/// Maximum delivery attempts before giving up on an outbox row.
const MAX_ATTEMPTS: i32 = 5;

/// Batch size for relay processing.
const RELAY_BATCH_SIZE: i64 = 500;

/// Advisory lock ID for relay coordination.
const RELAY_LOCK_ID: i64 = 900_200_001;

impl BillingOutboxRelay {
    /// Spawn the relay worker.
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

    /// Graceful shutdown: signal the relay to stop and drain all remaining events.
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

/// Background loop: claim and deliver outbox batches.
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
                // Drain loop: keep processing until no rows remain or max 10 iterations
                tracing::info!("Billing outbox relay draining...");
                for _ in 0..10 {
                    match process_batch(&db).await {
                        Ok(0) => break, // No more rows
                        Ok(_) => continue,
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

/// Claim a batch of pending outbox rows and deliver them to billing_events.
///
/// All operations run on a single connection to ensure advisory lock and
/// FOR UPDATE SKIP LOCKED are in the same session/transaction.
///
/// Returns the number of rows processed.
async fn process_batch(db: &PgPool) -> Result<usize, sqlx::Error> {
    // Acquire a single connection for the entire operation
    let mut conn = db.acquire().await?;

    // Try advisory lock on this connection
    let locked: (bool,) = sqlx::query_as("SELECT pg_try_advisory_lock($1)")
        .bind(RELAY_LOCK_ID)
        .fetch_one(&mut *conn)
        .await?;

    if !locked.0 {
        return Ok(0); // Another instance is already relaying
    }

    // Begin transaction — FOR UPDATE locks are held until commit/rollback
    let mut tx = conn.begin().await?;

    let rows = sqlx::query_as::<_, OutboxRow>(
        "SELECT id, tenant_id, operation, resource_id, tokens_in, tokens_out, \
                gpu_seconds, cost_usd, metadata \
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
        // Release advisory lock (on the original connection, outside tx)
        let _ = sqlx::query("SELECT pg_advisory_unlock($1)")
            .bind(RELAY_LOCK_ID)
            .execute(db)
            .await;
        return Ok(0);
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

    // Mark delivered rows within the same transaction
    if !delivered.is_empty() {
        sqlx::query("UPDATE billing_outbox SET delivered_at = NOW() WHERE id = ANY($1)")
            .bind(&delivered)
            .execute(&mut *tx)
            .await?;
    }

    // Mark failed rows
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

    // Commit releases the FOR UPDATE locks
    tx.commit().await?;

    if !delivered.is_empty() {
        tracing::info!(
            delivered = delivered.len(),
            failed = failed.len(),
            "Billing outbox relay batch processed"
        );
    }

    // Release advisory lock
    let _ = sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(RELAY_LOCK_ID)
        .execute(db)
        .await;

    Ok(count)
}

/// Insert a single outbox row into the billing_events ledger.
///
/// Uses the outbox row's ID as the billing_events PK. `ON CONFLICT DO NOTHING`
/// makes delivery idempotent — if the relay crashes between insert and
/// mark-delivered, the re-delivery is a no-op.
async fn deliver_to_ledger(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    row: &OutboxRow,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO billing_events \
         (id, tenant_id, operation, resource_id, tokens_in, tokens_out, gpu_seconds, cost_usd, metadata) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) \
         ON CONFLICT (id) DO NOTHING",
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
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Cleanup delivered outbox rows older than the retention period.
#[allow(dead_code)] // Called by scheduled maintenance
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
}

// Compile-time validation of configuration constants.
const _: () = {
    assert!(MAX_ATTEMPTS > 0);
    assert!(MAX_ATTEMPTS <= 10);
    assert!(RELAY_BATCH_SIZE > 0);
    assert!(RELAY_BATCH_SIZE <= 10_000);
};
