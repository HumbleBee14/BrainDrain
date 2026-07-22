//! Integration proof that Row-Level Security actually isolates tenants on the
//! least-privilege `app_rls` connection.
//!
//! Requires a live PostgreSQL with the `app_rls` role provisioned (migration
//! 017). Ignored by default because it needs infrastructure. Run it with:
//!
//! ```bash
//! DATABASE_URL=postgres://platform:platform_dev@localhost:5432/platform \
//! DATABASE_RLS_URL=postgres://app_rls:app_rls_dev_password@localhost:5432/platform \
//! cargo test -p platform-db --test rls_isolation -- --ignored --nocapture
//! ```

use sqlx::Row;
use uuid::Uuid;

/// Build the owner + app_rls pools from env, applying migrations. Returns `None`
/// (so the test skips) when the env is not configured.
async fn pools() -> Option<(sqlx::PgPool, sqlx::PgPool)> {
    let owner_url = std::env::var("DATABASE_URL").ok()?;
    let rls_url = std::env::var("DATABASE_RLS_URL").ok()?;
    let owner = platform_db::create_pool(&owner_url, 5)
        .await
        .expect("owner pool");
    platform_db::run_migrations(&owner)
        .await
        .expect("migrations");
    let rls = platform_db::create_pool(&rls_url, 5)
        .await
        .expect("app_rls pool");
    Some((owner, rls))
}

#[tokio::test]
#[ignore = "requires live Postgres + app_rls role (DATABASE_URL + DATABASE_RLS_URL)"]
async fn rls_isolates_tenants_on_app_rls_pool() {
    let Some((owner, rls)) = pools().await else {
        eprintln!("skipping rls_isolation: DATABASE_URL / DATABASE_RLS_URL not set");
        return;
    };

    // The app_rls role must genuinely be subject to RLS (not superuser/owner/bypass).
    platform_db::assert_rls_enforced(&rls)
        .await
        .expect("app_rls must be subject to RLS");

    // Seed two tenants + one project each via the owner connection (bypasses
    // RLS). Postgres generates a unique clerk_org_id so reruns don't collide.
    let tenant_a: Uuid = sqlx::query_scalar(
        "INSERT INTO tenants (clerk_org_id, name) \
         VALUES ('rls-test-a-' || gen_random_uuid()::text, 'A') RETURNING id",
    )
    .fetch_one(&owner)
    .await
    .unwrap();
    let tenant_b: Uuid = sqlx::query_scalar(
        "INSERT INTO tenants (clerk_org_id, name) \
         VALUES ('rls-test-b-' || gen_random_uuid()::text, 'B') RETURNING id",
    )
    .fetch_one(&owner)
    .await
    .unwrap();
    sqlx::query("INSERT INTO projects (tenant_id, name) VALUES ($1, 'proj-a')")
        .bind(tenant_a)
        .execute(&owner)
        .await
        .unwrap();
    sqlx::query("INSERT INTO projects (tenant_id, name) VALUES ($1, 'proj-b')")
        .bind(tenant_b)
        .execute(&owner)
        .await
        .unwrap();

    // Under tenant A's context, a raw SELECT with NO WHERE clause must return
    // ONLY tenant A's rows — this is the real proof that RLS (not the app's
    // WHERE tenant_id) is doing the filtering.
    let mut tx = platform_db::tenant::begin_tenant_tx(&rls, tenant_a)
        .await
        .unwrap();
    let rows = sqlx::query("SELECT tenant_id FROM projects")
        .fetch_all(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert!(!rows.is_empty(), "tenant A should see its own project");
    for row in &rows {
        let tid: Uuid = row.get("tenant_id");
        assert_eq!(
            tid, tenant_a,
            "RLS leaked a row belonging to another tenant"
        );
    }

    // With NO tenant context set, RLS must fail closed → zero visible rows.
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM projects")
        .fetch_one(&rls)
        .await
        .unwrap();
    assert_eq!(
        count, 0,
        "with no tenant context, RLS must expose zero rows (fail-closed)"
    );

    // Cleanup via owner (cascades to projects).
    sqlx::query("DELETE FROM tenants WHERE id = ANY($1)")
        .bind(vec![tenant_a, tenant_b])
        .execute(&owner)
        .await
        .unwrap();
}
