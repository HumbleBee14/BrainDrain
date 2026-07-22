//! Tenant-scoped database access for Row-Level Security enforcement.
//!
//! Every authenticated request has a `tenant_id`. For RLS to filter rows,
//! the Postgres session variable `app.tenant_id` must be set on the connection
//! running the query. The RLS policies (see the `app_tenant_id()` helper in the
//! migrations) fail closed: with no tenant set, no rows are visible.
//!
//! Tenant-scoped repository methods run their queries inside a transaction
//! opened by [`begin_tenant_tx`], which sets `app.tenant_id` before any query:
//!
//! ```rust,ignore
//! use platform_db::tenant::begin_tenant_tx;
//!
//! let mut tx = begin_tenant_tx(pool, tenant_id).await?;
//! let rows = sqlx::query_as::<_, Project>("SELECT * FROM projects WHERE tenant_id = $1")
//!     .bind(tenant_id)
//!     .fetch_all(&mut *tx)
//!     .await?;
//! tx.commit().await?;
//! ```
//!
//! The pool's `before_acquire` hook (see `lib.rs`) resets `app.tenant_id` to ''
//! on every checkout, so a prior request's context can never leak into the next.
//! `SET LOCAL` (via `set_config(..., true)`) additionally scopes the value to the
//! transaction's lifetime.

use sqlx::{PgConnection, PgPool, Postgres, Transaction};
use uuid::Uuid;

/// Open a transaction with `app.tenant_id` set for RLS enforcement.
///
/// The returned transaction has the tenant context applied via `SET LOCAL`, so
/// every query run on it is filtered by the RLS policies. The caller runs their
/// queries against `&mut *tx` and calls `tx.commit().await?` when done (dropping
/// without committing rolls back, as usual).
pub async fn begin_tenant_tx(
    pool: &PgPool,
    tenant_id: Uuid,
) -> Result<Transaction<'_, Postgres>, sqlx::Error> {
    let mut tx = pool.begin().await?;
    // set_config(..., is_local = true) == SET LOCAL: scoped to this transaction.
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await?;
    Ok(tx)
}

/// Set tenant context on a connection you already hold (e.g. inside a
/// transaction you opened yourself). Prefer [`begin_tenant_tx`] for the common
/// case of "open a tenant-scoped transaction".
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Compile-only: proves the `begin_tenant_tx` + `&mut *tx` executor pattern
    /// type-checks with a real borrowing query across an await. Never executed
    /// (no DB in unit tests); the retrofit depends on this shape compiling.
    #[allow(dead_code)]
    async fn _compiles(pool: &PgPool, tenant_id: Uuid) -> Result<i64, sqlx::Error> {
        let mut tx = begin_tenant_tx(pool, tenant_id).await?;
        let n: i64 = sqlx::query_scalar("SELECT 1").fetch_one(&mut *tx).await?;
        tx.commit().await?;
        Ok(n)
    }
}
