//! Tenant-scoped database access for Row-Level Security enforcement.
//!
//! Every authenticated request has a `tenant_id`. For RLS to filter rows
//! correctly, `app.tenant_id` must be set on the connection before queries run.
//!
//! Usage in a service or repository:
//!
//! ```rust,ignore
//! use platform_db::tenant::with_tenant;
//!
//! let result = with_tenant(pool, tenant_id, |conn| async move {
//!     sqlx::query_as::<_, Project>("SELECT * FROM projects")
//!         .fetch_all(conn)
//!         .await
//!         .map_err(Into::into)
//! }).await?;
//! ```
//!
//! ## How it works
//!
//! 1. Acquires a connection from the pool (fast — already connected)
//! 2. Sets `app.tenant_id` via `SET LOCAL` inside a transaction
//! 3. Runs the caller-supplied async closure against the same connection
//! 4. Commits the transaction (releasing `SET LOCAL` scope but preserving writes)
//!
//! The pool's `before_acquire` hook (see `lib.rs`) resets `app.tenant_id` to ''
//! before every checkout, preventing any cross-request leakage.

use std::future::Future;

use sqlx::{PgConnection, PgPool, Postgres, Transaction};
use uuid::Uuid;

/// Execute an async closure with `app.tenant_id` set for RLS enforcement.
///
/// Opens a transaction, sets `SET LOCAL app.tenant_id = $1`, runs `f`,
/// and commits. Any error returned from `f` rolls the transaction back.
pub async fn with_tenant<F, Fut, T, E>(pool: &PgPool, tenant_id: Uuid, f: F) -> Result<T, E>
where
    F: FnOnce(&mut PgConnection) -> Fut,
    Fut: Future<Output = Result<T, E>>,
    E: From<sqlx::Error>,
{
    let mut tx: Transaction<'_, Postgres> = pool.begin().await.map_err(E::from)?;

    // SET LOCAL restricts the setting to this transaction's lifetime.
    // The pool's before_acquire hook resets it to '' on each checkout anyway,
    // but SET LOCAL provides an extra safety net.
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(E::from)?;

    let result = f(&mut tx).await?;

    tx.commit().await.map_err(E::from)?;

    Ok(result)
}

/// Set tenant context on a bare connection (without a full transaction wrapper).
///
/// Use this when you already hold a transaction or need fine-grained control.
/// Prefer `with_tenant` for most use cases.
pub async fn set_tenant_context(
    conn: &mut PgConnection,
    tenant_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(conn)
        .await?;
    Ok(())
}
