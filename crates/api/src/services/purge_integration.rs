//! Integration proof that deleting a project or a model really removes both the
//! rows and the stored objects, and touches nothing that belongs to anything
//! else.
//!
//! Requires live infrastructure (PostgreSQL with the `app_rls` role, Redis,
//! MinIO). Ignored by default. Run it with `make infra` up:
//!
//! ```bash
//! DATABASE_URL=postgres://platform:platform_dev@localhost:5432/platform \
//! DATABASE_RLS_URL=postgres://app_rls:app_rls_dev_password@localhost:5432/platform \
//! REDIS_URL=redis://localhost:6379 \
//! S3_ENDPOINT=http://localhost:9000 S3_ACCESS_KEY=minioadmin \
//! S3_SECRET_KEY=minioadmin S3_BUCKET=platform-dev \
//! cargo test -p platform-api purge_integration -- --ignored --nocapture
//! ```

use bytes::Bytes;
use platform_shared::s3_paths;
use platform_storage::ObjectStorage;
use sqlx::Row;
use uuid::Uuid;

use crate::app_state::AppState;
use crate::config::Config;
use crate::services::purge_service::PurgeService;

/// Build a real AppState against the local stack, or `None` when the
/// environment is not configured (so the test skips rather than fails).
async fn app_state() -> Option<AppState> {
    std::env::var("DATABASE_URL").ok()?;
    let config = Config::from_env().ok()?;
    AppState::new(config).await.ok()
}

/// One project's worth of seeded rows and objects.
struct Fixture {
    tenant: Uuid,
    project: Uuid,
    dataset: Uuid,
    job: Uuid,
    model: Uuid,
    keys: Vec<String>,
}

async fn seed(state: &AppState, tenant: Uuid) -> Fixture {
    let db = state.db();
    let project = Uuid::new_v4();
    let dataset = Uuid::new_v4();
    let job = Uuid::new_v4();
    let model = Uuid::new_v4();
    let document = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO tenants (id, clerk_org_id, name, plan) VALUES ($1, $2, $3, 'starter') \
         ON CONFLICT DO NOTHING",
    )
    .bind(tenant)
    .bind(format!("org_purge_{}", tenant.simple()))
    .bind(format!("purge-test-{tenant}"))
    .execute(db)
    .await
    .expect("seed tenant");

    sqlx::query("INSERT INTO projects (id, tenant_id, name) VALUES ($1, $2, $3)")
        .bind(project)
        .bind(tenant)
        .bind("purge-test-project")
        .execute(db)
        .await
        .expect("seed project");

    let upload_key = s3_paths::upload_path(tenant, project, document, "pdf");
    sqlx::query(
        "INSERT INTO documents (id, tenant_id, project_id, filename, file_size, mime_type, storage_path) \
         VALUES ($1, $2, $3, 'doc.pdf', 1024, 'application/pdf', $4)",
    )
    .bind(document)
    .bind(tenant)
    .bind(project)
    .bind(&upload_key)
    .execute(db)
    .await
    .expect("seed document");

    let dataset_key = s3_paths::dataset_path(tenant, project, dataset);
    sqlx::query(
        "INSERT INTO datasets (id, tenant_id, project_id, name, format, storage_path, status, pair_count, size_bytes) \
         VALUES ($1, $2, $3, 'ds', 'chatml', $4, 'approved', 10, 2048)",
    )
    .bind(dataset)
    .bind(tenant)
    .bind(project)
    .bind(&dataset_key)
    .execute(db)
    .await
    .expect("seed dataset");

    sqlx::query(
        "INSERT INTO training_jobs (id, tenant_id, project_id, dataset_id, base_model, method, mode, status) \
         VALUES ($1, $2, $3, $4, 'meta-llama/Llama-3.2-1B', 'qlora', 'quick', 'completed')",
    )
    .bind(job)
    .bind(tenant)
    .bind(project)
    .bind(dataset)
    .execute(db)
    .await
    .expect("seed training job");

    // The trainer writes the adapter under the JOB id and stores that prefix.
    let adapter_prefix = s3_paths::adapter_prefix(tenant, job);
    sqlx::query(
        "INSERT INTO models (id, tenant_id, project_id, training_job_id, name, base_model, adapter_path, adapter_size_bytes, version) \
         VALUES ($1, $2, $3, $4, 'purge-test-model', 'meta-llama/Llama-3.2-1B', $5, 4096, 1)",
    )
    .bind(model)
    .bind(tenant)
    .bind(project)
    .bind(job)
    .bind(&adapter_prefix)
    .execute(db)
    .await
    .expect("seed model");

    // A child row that must go with the model.
    sqlx::query(
        "INSERT INTO api_keys (tenant_id, model_id, name, key_hash, key_prefix) \
         VALUES ($1, $2, 'purge-test-key', $3, 'pk_test')",
    )
    .bind(tenant)
    .bind(model)
    .bind(format!("hash_{}", model.simple()))
    .execute(db)
    .await
    .expect("seed api key");

    let keys = vec![
        upload_key,
        s3_paths::parsed_path(tenant, project, document),
        dataset_key.clone(),
        // Teacher logprob artifacts nest under the dataset's own key.
        dataset_key.replace(".jsonl", "-teacher-logprobs/deadbeef/00001.json"),
        format!("{adapter_prefix}adapter_model.safetensors"),
        format!("{adapter_prefix}adapter_config.json"),
        format!("{}shard-0.pt", s3_paths::checkpoint_prefix(tenant, job)),
        s3_paths::export_path(tenant, model, "model.gguf"),
        format!("chunks/{tenant}/{project}/batch-0.jsonl"),
        format!("pairs/{tenant}/{project}/batch-0.jsonl"),
        format!("pair-checkpoints/{tenant}/{project}/run-1/chunk-0.json"),
    ];
    for key in &keys {
        state
            .storage()
            .put(key, Bytes::from_static(b"x"), "application/octet-stream")
            .await
            .expect("seed object");
    }

    Fixture {
        tenant,
        project,
        dataset,
        job,
        model,
        keys,
    }
}

async fn count(state: &AppState, sql: &str, id: Uuid) -> i64 {
    sqlx::query(sql)
        .bind(id)
        .fetch_one(state.db())
        .await
        .expect("count query")
        .get::<i64, _>(0)
}

async fn cleanup(state: &AppState, tenant: Uuid) {
    let _ = sqlx::query("DELETE FROM tenants WHERE id = $1")
        .bind(tenant)
        .execute(state.db())
        .await;
    for prefix in s3_paths::tenant_prefixes(tenant) {
        let _ = state.storage().delete_prefix(&prefix).await;
    }
}

#[tokio::test]
#[ignore = "requires live Postgres + Redis + MinIO (see module docs)"]
async fn purging_a_project_removes_every_row_and_object() {
    let Some(state) = app_state().await else {
        eprintln!("skipping: infrastructure env not configured");
        return;
    };
    let tenant = Uuid::new_v4();
    let f = seed(&state, tenant).await;
    // A second project under the same tenant must survive untouched.
    let other = seed(&state, tenant).await;

    let summary = PurgeService::purge_project(&state, f.tenant, f.project)
        .await
        .expect("purge project");

    assert_eq!(summary.objects_deleted, f.keys.len(), "object count");
    assert_eq!(summary.jobs_stopped, 0, "nothing was running");

    for key in &f.keys {
        assert!(
            !state.storage().exists(key).await.expect("exists"),
            "{key} survived the purge"
        );
    }
    assert_eq!(
        count(
            &state,
            "SELECT COUNT(*) FROM projects WHERE id = $1",
            f.project
        )
        .await,
        0
    );
    assert_eq!(
        count(
            &state,
            "SELECT COUNT(*) FROM datasets WHERE id = $1",
            f.dataset
        )
        .await,
        0
    );
    assert_eq!(
        count(
            &state,
            "SELECT COUNT(*) FROM training_jobs WHERE id = $1",
            f.job
        )
        .await,
        0
    );
    assert_eq!(
        count(&state, "SELECT COUNT(*) FROM models WHERE id = $1", f.model).await,
        0
    );
    assert_eq!(
        count(
            &state,
            "SELECT COUNT(*) FROM api_keys WHERE model_id = $1",
            f.model
        )
        .await,
        0,
        "api keys must cascade with the model"
    );

    for key in &other.keys {
        assert!(
            state.storage().exists(key).await.expect("exists"),
            "{key} belonged to another project and was wrongly deleted"
        );
    }
    assert_eq!(
        count(
            &state,
            "SELECT COUNT(*) FROM models WHERE id = $1",
            other.model
        )
        .await,
        1
    );

    cleanup(&state, tenant).await;
}

#[tokio::test]
#[ignore = "requires live Postgres + Redis + MinIO (see module docs)"]
async fn purging_a_model_leaves_the_project_and_its_data_intact() {
    let Some(state) = app_state().await else {
        eprintln!("skipping: infrastructure env not configured");
        return;
    };
    let tenant = Uuid::new_v4();
    let f = seed(&state, tenant).await;

    let summary = PurgeService::purge_model(&state, f.tenant, f.model)
        .await
        .expect("purge model");

    // Adapter (2 files), its checkpoint and its export go; documents, dataset
    // and the generation intermediates stay with the project.
    assert_eq!(summary.objects_deleted, 4, "adapter + checkpoint + export");

    let model_owned = [
        format!(
            "{}adapter_model.safetensors",
            s3_paths::adapter_prefix(f.tenant, f.job)
        ),
        format!("{}shard-0.pt", s3_paths::checkpoint_prefix(f.tenant, f.job)),
        s3_paths::export_path(f.tenant, f.model, "model.gguf"),
    ];
    for key in &model_owned {
        assert!(
            !state.storage().exists(key).await.expect("exists"),
            "{key} survived"
        );
    }

    assert_eq!(
        count(&state, "SELECT COUNT(*) FROM models WHERE id = $1", f.model).await,
        0
    );
    assert_eq!(
        count(
            &state,
            "SELECT COUNT(*) FROM training_jobs WHERE id = $1",
            f.job
        )
        .await,
        0,
        "the run goes with the model so its plan slot is released"
    );
    assert_eq!(
        count(
            &state,
            "SELECT COUNT(*) FROM projects WHERE id = $1",
            f.project
        )
        .await,
        1,
        "the project must survive"
    );
    assert_eq!(
        count(
            &state,
            "SELECT COUNT(*) FROM datasets WHERE id = $1",
            f.dataset
        )
        .await,
        1
    );
    assert!(
        state
            .storage()
            .exists(&s3_paths::dataset_path(f.tenant, f.project, f.dataset))
            .await
            .expect("exists"),
        "dataset object must survive a model delete"
    );

    cleanup(&state, tenant).await;
}
