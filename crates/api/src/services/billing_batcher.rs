use sqlx::PgPool;
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

/// A billing event to be batch-inserted.
#[derive(Debug)]
pub struct BillingEvent {
    pub tenant_id: Uuid,
    pub operation: String,
    pub resource_id: Option<Uuid>,
    pub tokens_in: i64,
    pub tokens_out: i64,
    pub gpu_seconds: i32,
    pub cost_usd: f64,
    pub metadata: serde_json::Value,
}

/// Collects billing events via a channel and bulk-inserts them periodically.
///
/// Instead of one DB write per inference request (fire-and-forget `tokio::spawn`),
/// events are sent to a bounded channel. A background task flushes the buffer
/// every `flush_interval` or when `batch_size` events accumulate.
///
/// At 10K req/min this reduces ~10K individual INSERTs to ~12 bulk inserts/min.
pub struct BillingBatcher {
    sender: mpsc::Sender<BillingEvent>,
    /// Shutdown signal + flush task handle (taken by the first shutdown() call).
    shutdown: Mutex<Option<ShutdownHandle>>,
}

struct ShutdownHandle {
    signal: oneshot::Sender<()>,
    flush_handle: tokio::task::JoinHandle<()>,
}

impl BillingBatcher {
    /// Spawn the billing batcher with a background flush task.
    ///
    /// - `channel_capacity`: bounded channel size (events dropped if full)
    /// - `batch_size`: flush after this many events accumulate
    /// - `flush_interval`: flush at least this often (even if batch not full)
    pub fn new(
        db: PgPool,
        channel_capacity: usize,
        batch_size: usize,
        flush_interval: Duration,
    ) -> Self {
        let (sender, receiver) = mpsc::channel(channel_capacity);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();

        let flush_handle = tokio::spawn(flush_loop(
            db,
            receiver,
            batch_size,
            flush_interval,
            shutdown_rx,
        ));

        Self {
            sender,
            shutdown: Mutex::new(Some(ShutdownHandle {
                signal: shutdown_tx,
                flush_handle,
            })),
        }
    }

    /// Send a billing event for batch insertion. Non-blocking.
    /// Returns false if the channel is full (event is dropped).
    pub fn send(&self, event: BillingEvent) -> bool {
        match self.sender.try_send(event) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(_)) => {
                tracing::warn!("Billing batcher channel full — dropping event");
                false
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                tracing::error!("Billing batcher channel closed — dropping event");
                false
            }
        }
    }

    /// Graceful shutdown: signal the flush loop to drain remaining events and exit.
    ///
    /// Safe to call multiple times — subsequent calls are no-ops.
    /// Uses `&self` so it can be called through `Arc<BillingBatcher>`.
    pub async fn shutdown(&self) {
        let handle = self
            .shutdown
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();

        if let Some(handle) = handle {
            // Signal the flush loop to stop accepting new events and drain
            let _ = handle.signal.send(());
            if let Err(e) = handle.flush_handle.await {
                tracing::error!(error = %e, "Billing batcher flush task panicked");
            }
            tracing::info!("Billing batcher shutdown complete");
        }
    }
}

/// Background loop: collects events and bulk-inserts them.
async fn flush_loop(
    db: PgPool,
    mut receiver: mpsc::Receiver<BillingEvent>,
    batch_size: usize,
    flush_interval: Duration,
    mut shutdown_rx: oneshot::Receiver<()>,
) {
    let mut buffer: Vec<BillingEvent> = Vec::with_capacity(batch_size);
    let mut interval = tokio::time::interval(flush_interval);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut consecutive_failures: usize = 0;

    loop {
        tokio::select! {
            // Receive events
            event = receiver.recv() => {
                match event {
                    Some(e) => {
                        buffer.push(e);
                        if buffer.len() >= batch_size {
                            flush_batch(&db, &mut buffer, &mut consecutive_failures).await;
                        }
                    }
                    None => {
                        // Channel closed — drain remaining and exit
                        if !buffer.is_empty() {
                            flush_batch(&db, &mut buffer, &mut consecutive_failures).await;
                        }
                        tracing::info!("Billing batcher shut down (channel closed)");
                        return;
                    }
                }
            }
            // Periodic flush
            _ = interval.tick() => {
                if !buffer.is_empty() {
                    flush_batch(&db, &mut buffer, &mut consecutive_failures).await;
                }
            }
            // Explicit shutdown signal
            _ = &mut shutdown_rx => {
                // Drain any remaining events from the channel
                receiver.close();
                while let Some(e) = receiver.recv().await {
                    buffer.push(e);
                }
                if !buffer.is_empty() {
                    flush_batch(&db, &mut buffer, &mut consecutive_failures).await;
                }
                tracing::info!("Billing batcher shut down (explicit signal)");
                return;
            }
        }
    }
}

/// Maximum consecutive flush failures before dropping events to prevent unbounded memory growth.
const MAX_FLUSH_RETRIES: usize = 3;

/// Bulk-insert a batch of billing events using QueryBuilder.
///
/// On failure, events are retained in the buffer for the next flush attempt.
/// After MAX_FLUSH_RETRIES consecutive failures, events are dropped to prevent
/// unbounded memory growth (better to lose billing data than OOM the process).
async fn flush_batch(
    db: &PgPool,
    buffer: &mut Vec<BillingEvent>,
    consecutive_failures: &mut usize,
) {
    let count = buffer.len();

    let mut builder = sqlx::QueryBuilder::new(
        "INSERT INTO billing_events (tenant_id, operation, resource_id, tokens_in, tokens_out, gpu_seconds, cost_usd, metadata) ",
    );

    builder.push_values(buffer.iter(), |mut b, event| {
        b.push_bind(event.tenant_id)
            .push_bind(&event.operation)
            .push_bind(event.resource_id)
            .push_bind(event.tokens_in)
            .push_bind(event.tokens_out)
            .push_bind(event.gpu_seconds)
            .push_bind(event.cost_usd)
            .push_bind(&event.metadata);
    });

    match builder.build().execute(db).await {
        Ok(_) => {
            buffer.clear();
            tracing::debug!(count, "Billing batcher flushed events");
            *consecutive_failures = 0;
        }
        Err(e) => {
            *consecutive_failures += 1;
            if *consecutive_failures >= MAX_FLUSH_RETRIES {
                tracing::error!(
                    count,
                    consecutive_failures = *consecutive_failures,
                    error = %e,
                    "Billing batcher flush failed too many times — dropping events"
                );
                buffer.clear();
                *consecutive_failures = 0;
            } else {
                tracing::warn!(
                    count,
                    consecutive_failures = *consecutive_failures,
                    error = %e,
                    "Billing batcher flush failed — retaining events for retry"
                );
                // Events stay in buffer for the next flush attempt
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn billing_event_fields() {
        let event = BillingEvent {
            tenant_id: Uuid::new_v4(),
            operation: "inference".to_string(),
            resource_id: Some(Uuid::new_v4()),
            tokens_in: 100,
            tokens_out: 200,
            gpu_seconds: 0,
            cost_usd: 0.001,
            metadata: serde_json::json!({"key": "value"}),
        };
        assert_eq!(event.operation, "inference");
        assert_eq!(event.tokens_in, 100);
        assert_eq!(event.tokens_out, 200);
    }

    #[test]
    fn cost_calculation_is_reasonable() {
        // $0.15 per 1M input tokens, $0.60 per 1M output tokens
        let tokens_in: i64 = 1_000_000;
        let tokens_out: i64 = 1_000_000;
        let input_cost = tokens_in as f64 * 0.15 / 1_000_000.0;
        let output_cost = tokens_out as f64 * 0.60 / 1_000_000.0;
        let total = input_cost + output_cost;
        assert!((total - 0.75).abs() < 0.001);
    }
}
