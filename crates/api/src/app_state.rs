use platform_storage::s3::{S3Config, S3Storage};
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;

use crate::auth::{AuthProviderChain, ClerkAuthProvider, InternalTokenAuthProvider};
use crate::config::Config;
use crate::error::AppResult;
use crate::repositories::api_key_repo::PgApiKeyRepo;
use crate::repositories::audit_log_repo::PgAuditLogRepo;
use crate::repositories::billing_event_repo::PgBillingEventRepo;
use crate::repositories::data_guide_repo::PgDataGuideRepo;
use crate::repositories::dataset_repo::PgDatasetRepo;
use crate::repositories::document_repo::PgDocumentRepo;
use crate::repositories::evaluation_repo::PgEvaluationRepo;
use crate::repositories::export_repo::PgExportRepo;
use crate::repositories::inference_instance_repo::PgInferenceInstanceRepo;
use crate::repositories::invitation_repo::PgInvitationRepo;
use crate::repositories::model_repo::PgModelRepo;
use crate::repositories::notification_repo::PgNotificationRepo;
use crate::repositories::project_repo::PgProjectRepo;
use crate::repositories::team_member_repo::PgTeamMemberRepo;
use crate::repositories::tenant_repo::PgTenantRepo;
use crate::repositories::training_job_repo::PgTrainingJobRepo;
use crate::repositories::traits::{
    ApiKeyRepository, AuditLogRepository, BillingEventRepository, DataGuideRepository,
    DatasetRepository, DocumentRepository, EvaluationRepository, ExportRepository,
    InferenceInstanceRepository, InvitationRepository, ModelRepository, NotificationRepository,
    ProjectRepository, TeamMemberRepository, TenantRepository, TrainingJobRepository,
};
use crate::services::billing_batcher::BillingBatcher;
use crate::services::billing_outbox::BillingOutboxRelay;
use crate::services::billing_provider::BillingProvider;
use crate::services::circuit_breaker::CircuitBreaker;
use crate::services::delivery_worker::DeliveryWorker;
use crate::services::feature_flags::{
    FeatureFlags, FlagContext, INFERENCE_BACKEND_TGI_ENABLED,
    NOTIFICATIONS_DELIVERY_WORKER_ENABLED, UnleashPollerHandle, build_feature_flags_async,
};
use crate::services::inference_backend::{
    InferenceBackend, build_backend, build_backend_for_instance,
};
use crate::services::stripe_billing::{NoOpBillingProvider, StripeBillingProvider};
use crate::temporal::{TemporalClient, WorkflowOrchestrator};

/// Shared application state available to all route handlers.
///
/// Wrapped in `Arc` for cheap cloning across async tasks.
/// Access in handlers via `State<AppState>`.
#[derive(Clone)]
pub struct AppState {
    inner: Arc<AppStateInner>,
}

struct AppStateInner {
    pub config: Config,
    /// Owner/admin pool — migrations, partition DDL, cross-tenant maintenance.
    pub db: PgPool,
    /// Least-privilege pool subject to RLS — carries tenant request traffic.
    /// Cloned into the tenant-scoped repositories at construction; retained here
    /// for the `db_rls()` accessor used by isolation tests and admin tooling.
    #[allow(dead_code)]
    pub db_rls: PgPool,
    pub redis: redis::aio::ConnectionManager,
    pub storage: S3Storage,
    pub orchestrator: Option<Arc<dyn WorkflowOrchestrator>>,
    pub auth_chain: AuthProviderChain,
    pub internal_auth: Option<InternalTokenAuthProvider>,
    pub http_client: reqwest::Client,
    // Repository trait objects
    pub project_repo: Arc<dyn ProjectRepository>,
    pub document_repo: Arc<dyn DocumentRepository>,
    pub dataset_repo: Arc<dyn DatasetRepository>,
    pub data_guide_repo: Arc<dyn DataGuideRepository>,
    pub training_job_repo: Arc<dyn TrainingJobRepository>,
    pub model_repo: Arc<dyn ModelRepository>,
    pub evaluation_repo: Arc<dyn EvaluationRepository>,
    pub export_repo: Arc<dyn ExportRepository>,
    pub inference_instance_repo: Arc<dyn InferenceInstanceRepository>,
    pub api_key_repo: Arc<dyn ApiKeyRepository>,
    pub billing_event_repo: Arc<dyn BillingEventRepository>,
    pub audit_log_repo: Arc<dyn AuditLogRepository>,
    pub team_member_repo: Arc<dyn TeamMemberRepository>,
    pub invitation_repo: Arc<dyn InvitationRepository>,
    pub notification_repo: Arc<dyn NotificationRepository>,
    pub tenant_repo: Arc<dyn TenantRepository>,
    pub billing_provider: Arc<dyn BillingProvider>,
    pub inference_backend: Arc<dyn InferenceBackend>,
    /// Cached backends for registered inference instances, keyed by base_url.
    /// Ensures circuit breakers accumulate state across requests to the same instance.
    pub instance_backend_cache:
        std::sync::RwLock<std::collections::HashMap<String, Arc<dyn InferenceBackend>>>,
    pub feature_flags: Arc<FeatureFlags>,
    pub unleash_poller: Option<std::sync::Mutex<UnleashPollerHandle>>,
    pub billing_batcher: Arc<BillingBatcher>,
    pub billing_outbox_relay: Option<Arc<BillingOutboxRelay>>,
    pub delivery_worker: Arc<DeliveryWorker>,
}

impl AppState {
    /// Build application state from config.
    ///
    /// Initializes all connections: database pool, Redis, S3 client.
    /// Fails fast if any connection cannot be established.
    pub async fn new(config: Config) -> anyhow::Result<Self> {
        // Owner/admin connection: runs migrations, partition DDL, admin endpoints,
        // and cross-tenant maintenance. Bypasses RLS (it owns the tables).
        let db = platform_db::create_pool(&config.database_url, config.database_max_connections)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to connect to PostgreSQL (owner): {e}"))?;

        tracing::info!("Connected to PostgreSQL (owner role)");

        // Migrations run on the owner connection and MUST happen before the RLS
        // pool is created: migration 017 creates the `app_rls` role the RLS pool
        // connects as. Production provisions the role + runs migrations out of
        // band (`make migrate`), so auto-migration stays gated to dev/staging.
        if config.is_dev() || config.environment == "staging" {
            tracing::info!("Running database migrations (non-production)...");
            platform_db::run_migrations(&db)
                .await
                .map_err(|e| anyhow::anyhow!("Migration failed: {e}"))?;
            tracing::info!("Migrations complete");
        } else {
            tracing::info!("Production mode — skipping auto-migration (use `make migrate`)");
        }

        // RLS connection: least-privilege `app_rls` role that carries all tenant
        // request traffic and is subject to Row-Level Security. Falls back to the
        // owner connection (with a loud warning) when DATABASE_RLS_URL is unset,
        // in which case isolation relies on `WHERE tenant_id` alone.
        let db_rls = match config.database_rls_url.as_deref().filter(|s| !s.is_empty()) {
            Some(rls_url) => {
                let pool = platform_db::create_pool(rls_url, config.database_max_connections)
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!("Failed to connect to PostgreSQL (app_rls): {e}")
                    })?;
                // Refuse to start if this role is exempt from RLS — otherwise
                // tenant traffic would silently run without isolation.
                platform_db::assert_rls_enforced(&pool)
                    .await
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                tracing::info!("Connected to PostgreSQL (app_rls role) — RLS second layer active");
                pool
            }
            None => {
                tracing::warn!(
                    "DATABASE_RLS_URL not set — tenant queries run on the owner connection and \
                     the RLS second layer is INACTIVE (isolation relies on WHERE tenant_id only). \
                     Set DATABASE_RLS_URL to the app_rls role for defense in depth."
                );
                db.clone()
            }
        };

        // Redis
        let redis_client = redis::Client::open(config.redis_url.as_str())
            .map_err(|e| anyhow::anyhow!("Invalid Redis URL: {e}"))?;
        let redis = redis::aio::ConnectionManager::new(redis_client)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to connect to Redis: {e}"))?;

        tracing::info!("Connected to Redis");

        // S3 / Object storage
        let storage = S3Storage::new(S3Config {
            endpoint: config.s3_endpoint.clone(),
            access_key: config.s3_access_key.clone(),
            secret_key: config.s3_secret_key.clone(),
            region: config.s3_region.clone(),
            bucket: config.s3_bucket.clone(),
            force_path_style: config.s3_endpoint.is_some(), // Force path-style for MinIO/R2
        })
        .await;

        tracing::info!("S3 client initialized (bucket: {})", &config.s3_bucket);

        // Workflow orchestrator (optional — API works without it, just can't trigger workflows)
        let orchestrator: Option<Arc<dyn WorkflowOrchestrator>> =
            if !config.temporal_host.is_empty() {
                let client = TemporalClient::new(
                    &config.temporal_host,
                    &config.temporal_namespace,
                    &config.temporal_task_queue,
                );
                tracing::info!(
                    "Workflow orchestrator configured (host: {})",
                    &config.temporal_host
                );
                Some(Arc::new(client))
            } else {
                tracing::warn!("Workflow orchestrator not configured — workflow triggers disabled");
                None
            };

        // Shared HTTP client for outbound requests (connection pooling, 10s default timeout)
        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|e| anyhow::anyhow!("Failed to build HTTP client: {e}"))?;

        // Auth provider chain (uses shared HTTP client for JWKS fetching)
        let auth_chain = AuthProviderChain::new().add(ClerkAuthProvider::new(
            config.clerk_jwks_url.clone(),
            config.is_dev(),
            http_client.clone(),
        ));

        // Internal token auth for worker → API service calls
        let internal_auth = if !config.platform_internal_token.is_empty() {
            tracing::info!("Internal service token configured for worker callbacks");
            Some(InternalTokenAuthProvider::new(
                config.platform_internal_token.clone(),
            ))
        } else {
            tracing::warn!("No PLATFORM_INTERNAL_TOKEN set — worker deploy callbacks will fail");
            None
        };

        tracing::info!("Auth provider chain initialized");

        // Repository trait objects (PgPool is Arc<PoolInner>, cheap to clone).
        //
        // Tenant-scoped repos take the RLS pool (`db_rls`): their queries run
        // inside a `begin_tenant_tx` transaction so RLS filters by tenant.
        // Repos with legitimately cross-tenant methods (api_key auth-by-hash,
        // invitation accept, the global adapter-cap / reaper on models, the
        // notification delivery worker) also take the owner pool (`db`) for
        // those specific methods. Repos over tables without RLS (tenant_repo,
        // inference_instance_repo) use the RLS pool too — app_rls is granted on
        // those tables and there is no RLS to satisfy.
        let project_repo: Arc<dyn ProjectRepository> = Arc::new(PgProjectRepo::new(db_rls.clone()));
        let document_repo: Arc<dyn DocumentRepository> =
            Arc::new(PgDocumentRepo::new(db_rls.clone()));
        let dataset_repo: Arc<dyn DatasetRepository> = Arc::new(PgDatasetRepo::new(db_rls.clone()));
        let data_guide_repo: Arc<dyn DataGuideRepository> =
            Arc::new(PgDataGuideRepo::new(db_rls.clone()));
        let training_job_repo: Arc<dyn TrainingJobRepository> =
            Arc::new(PgTrainingJobRepo::new(db_rls.clone()));
        let model_repo: Arc<dyn ModelRepository> =
            Arc::new(PgModelRepo::new(db_rls.clone(), db.clone()));
        let evaluation_repo: Arc<dyn EvaluationRepository> =
            Arc::new(PgEvaluationRepo::new(db_rls.clone()));
        let export_repo: Arc<dyn ExportRepository> = Arc::new(PgExportRepo::new(db_rls.clone()));
        let inference_instance_repo: Arc<dyn InferenceInstanceRepository> =
            Arc::new(PgInferenceInstanceRepo::new(db_rls.clone()));
        let api_key_repo: Arc<dyn ApiKeyRepository> =
            Arc::new(PgApiKeyRepo::new(db_rls.clone(), db.clone()));
        let billing_event_repo: Arc<dyn BillingEventRepository> =
            Arc::new(PgBillingEventRepo::new(db_rls.clone()));
        let audit_log_repo: Arc<dyn AuditLogRepository> =
            Arc::new(PgAuditLogRepo::new(db_rls.clone()));
        let team_member_repo: Arc<dyn TeamMemberRepository> =
            Arc::new(PgTeamMemberRepo::new(db_rls.clone()));
        let invitation_repo: Arc<dyn InvitationRepository> =
            Arc::new(PgInvitationRepo::new(db_rls.clone(), db.clone()));
        let notification_repo: Arc<dyn NotificationRepository> =
            Arc::new(PgNotificationRepo::new(db_rls.clone(), db.clone()));
        let tenant_repo: Arc<dyn TenantRepository> = Arc::new(PgTenantRepo::new(db_rls.clone()));

        // Billing provider: Stripe when configured, no-op for dev
        let billing_provider: Arc<dyn BillingProvider> =
            if let Some(ref secret_key) = config.stripe_secret_key {
                tracing::info!("Stripe billing provider configured");
                Arc::new(StripeBillingProvider::new(
                    http_client.clone(),
                    secret_key.clone(),
                    config.stripe_price_starter.clone(),
                    config.stripe_price_growth.clone(),
                    config.stripe_price_pro.clone(),
                ))
            } else {
                tracing::warn!("Stripe not configured — billing provider is no-op");
                Arc::new(NoOpBillingProvider)
            };

        // Feature flags: static or Unleash (remote) provider.
        let (feature_flags, unleash_poller) = build_feature_flags_async(&config).await?;
        let feature_flags = Arc::new(feature_flags);
        let unleash_poller = unleash_poller.map(std::sync::Mutex::new);
        let startup_flag_context = FlagContext::default();

        if config.inference_backend_type == "tgi"
            && !feature_flags.is_enabled(INFERENCE_BACKEND_TGI_ENABLED, &startup_flag_context)
        {
            return Err(anyhow::anyhow!(
                "TGI backend selected but feature flag '{}' is disabled",
                INFERENCE_BACKEND_TGI_ENABLED
            ));
        }

        // Inference backend — pluggable serving engine (vLLM / TGI / SGLang).
        // Circuit breaker wraps load_adapter calls; unload is always best-effort.
        let inference_circuit_breaker = CircuitBreaker::new(
            config.vllm_cb_failure_threshold,
            Duration::from_secs(config.vllm_cb_recovery_timeout_secs),
        );
        let inference_backend = build_backend(
            &config.inference_backend_type,
            config.inference_server_url.clone(),
            http_client.clone(),
            inference_circuit_breaker,
        );

        // Billing micro-batcher (in-memory, used when outbox is disabled)
        let billing_batcher = Arc::new(BillingBatcher::new(
            db.clone(),
            config.billing_channel_capacity,
            config.billing_batch_size,
            Duration::from_secs(config.billing_flush_interval_secs),
        ));

        // Durable billing outbox relay (when feature flag is enabled)
        let outbox_enabled = feature_flags.bool_variation(
            crate::services::feature_flags::BILLING_OUTBOX_ENABLED,
            false,
            &crate::services::feature_flags::FlagContext::default(),
        );
        if config.environment == "production" && !outbox_enabled {
            return Err(anyhow::anyhow!(
                "billing.outbox.enabled must be true in production"
            ));
        }
        let billing_outbox_relay = if outbox_enabled {
            tracing::info!("Billing outbox relay enabled (durable billing path)");
            Some(Arc::new(BillingOutboxRelay::new(
                db.clone(),
                Duration::from_secs(config.billing_flush_interval_secs),
            )))
        } else {
            tracing::info!("Billing outbox disabled — using in-memory batcher");
            None
        };

        // Webhook HTTP client: redirects disabled to prevent SSRF bypass via redirect to internal IPs.
        // Separate from the shared http_client since Stripe/Clerk may need redirect support.
        let webhook_http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| anyhow::anyhow!("Failed to build webhook HTTP client: {e}"))?;

        // Notification delivery worker (configurable poll interval) {Current: polls every 10s for pending webhook deliveries)
        let delivery_worker = if feature_flags.bool_variation(
            NOTIFICATIONS_DELIVERY_WORKER_ENABLED,
            true,
            &startup_flag_context,
        ) {
            Arc::new(DeliveryWorker::new(
                Arc::clone(&notification_repo),
                webhook_http_client,
                Duration::from_secs(config.delivery_poll_interval_secs),
            ))
        } else {
            tracing::warn!(
                flag = NOTIFICATIONS_DELIVERY_WORKER_ENABLED,
                "Notification delivery worker disabled by feature flag"
            );
            Arc::new(DeliveryWorker::disabled())
        };

        tracing::info!(
            "Infrastructure hardening initialized (circuit breaker + billing batcher + delivery worker)"
        );

        Ok(Self {
            inner: Arc::new(AppStateInner {
                config,
                db,
                db_rls,
                redis,
                storage,
                orchestrator,
                auth_chain,
                internal_auth,
                http_client,
                project_repo,
                document_repo,
                dataset_repo,
                data_guide_repo,
                training_job_repo,
                model_repo,
                evaluation_repo,
                export_repo,
                inference_instance_repo,
                api_key_repo,
                billing_event_repo,
                audit_log_repo,
                team_member_repo,
                invitation_repo,
                notification_repo,
                tenant_repo,
                billing_provider,
                inference_backend,
                instance_backend_cache: std::sync::RwLock::new(std::collections::HashMap::new()),
                feature_flags,
                unleash_poller,
                billing_batcher,
                billing_outbox_relay,
                delivery_worker,
            }),
        })
    }

    pub fn config(&self) -> &Config {
        &self.inner.config
    }

    pub fn db(&self) -> &PgPool {
        &self.inner.db
    }

    /// The RLS-enforced pool (least-privilege `app_rls` role). Used by
    /// tenant-scoped repositories and by isolation tests.
    #[allow(dead_code)] // Public accessor for isolation tests / future admin tooling.
    pub fn db_rls(&self) -> &PgPool {
        &self.inner.db_rls
    }

    pub fn redis(&self) -> redis::aio::ConnectionManager {
        self.inner.redis.clone()
    }

    pub fn storage(&self) -> &S3Storage {
        &self.inner.storage
    }

    pub fn orchestrator(&self) -> Option<&dyn WorkflowOrchestrator> {
        self.inner.orchestrator.as_deref()
    }

    pub fn auth_chain(&self) -> &AuthProviderChain {
        &self.inner.auth_chain
    }

    pub fn internal_auth(&self) -> Option<&InternalTokenAuthProvider> {
        self.inner.internal_auth.as_ref()
    }

    pub fn http_client(&self) -> &reqwest::Client {
        &self.inner.http_client
    }

    pub fn project_repo(&self) -> &dyn ProjectRepository {
        &*self.inner.project_repo
    }

    pub fn document_repo(&self) -> &dyn DocumentRepository {
        &*self.inner.document_repo
    }

    pub fn dataset_repo(&self) -> &dyn DatasetRepository {
        &*self.inner.dataset_repo
    }

    pub fn data_guide_repo(&self) -> &dyn DataGuideRepository {
        &*self.inner.data_guide_repo
    }

    pub fn training_job_repo(&self) -> &dyn TrainingJobRepository {
        &*self.inner.training_job_repo
    }

    pub fn model_repo(&self) -> &dyn ModelRepository {
        &*self.inner.model_repo
    }

    pub fn evaluation_repo(&self) -> &dyn EvaluationRepository {
        &*self.inner.evaluation_repo
    }

    pub fn export_repo(&self) -> &dyn ExportRepository {
        &*self.inner.export_repo
    }

    pub fn inference_instance_repo(&self) -> &dyn InferenceInstanceRepository {
        &*self.inner.inference_instance_repo
    }

    pub fn api_key_repo(&self) -> &dyn ApiKeyRepository {
        &*self.inner.api_key_repo
    }

    pub fn billing_event_repo(&self) -> &dyn BillingEventRepository {
        &*self.inner.billing_event_repo
    }

    pub fn audit_log_repo(&self) -> &dyn AuditLogRepository {
        &*self.inner.audit_log_repo
    }

    pub fn team_member_repo(&self) -> &dyn TeamMemberRepository {
        &*self.inner.team_member_repo
    }

    pub fn invitation_repo(&self) -> &dyn InvitationRepository {
        &*self.inner.invitation_repo
    }

    pub fn notification_repo(&self) -> &dyn NotificationRepository {
        &*self.inner.notification_repo
    }

    pub fn tenant_repo(&self) -> &dyn TenantRepository {
        &*self.inner.tenant_repo
    }

    pub fn billing_provider(&self) -> &dyn BillingProvider {
        &*self.inner.billing_provider
    }

    pub fn inference_backend(&self) -> &dyn InferenceBackend {
        &*self.inner.inference_backend
    }

    /// Return the global inference backend as an Arc.
    /// Used by the single-instance fallback path so the shared circuit
    /// breaker accumulates state across requests.
    pub fn inference_backend_arc(&self) -> Arc<dyn InferenceBackend> {
        Arc::clone(&self.inner.inference_backend)
    }

    /// Get or create a cached backend for a registered inference instance.
    /// Backends are cached by base_url so circuit breakers accumulate state
    /// across requests to the same instance.
    pub fn build_inference_backend_for_instance(
        &self,
        backend_type: &str,
        server_url: &str,
    ) -> Arc<dyn InferenceBackend> {
        // Fast path: check read lock
        if let Ok(cache) = self.inner.instance_backend_cache.read()
            && let Some(backend) = cache.get(server_url)
        {
            return Arc::clone(backend);
        }

        // Slow path: create and cache
        let backend = build_backend_for_instance(
            backend_type,
            server_url,
            self.inner.http_client.clone(),
            CircuitBreaker::new(
                self.inner.config.vllm_cb_failure_threshold,
                Duration::from_secs(self.inner.config.vllm_cb_recovery_timeout_secs),
            ),
        );

        if let Ok(mut cache) = self.inner.instance_backend_cache.write() {
            cache
                .entry(server_url.to_string())
                .or_insert_with(|| Arc::clone(&backend));
        }

        backend
    }

    pub fn feature_flags(&self) -> &FeatureFlags {
        &self.inner.feature_flags
    }

    pub fn shutdown_unleash_poller(&self) {
        if let Some(poller) = &self.inner.unleash_poller
            && let Ok(mut handle) = poller.lock()
        {
            handle.shutdown();
        }
    }

    #[allow(dead_code)] // Used internally by record_billing_event
    pub fn billing_batcher(&self) -> &BillingBatcher {
        &self.inner.billing_batcher
    }

    /// Get a cloneable handle for explicit shutdown of the billing batcher.
    pub fn billing_batcher_handle(&self) -> Arc<BillingBatcher> {
        Arc::clone(&self.inner.billing_batcher)
    }

    pub fn billing_outbox_relay_handle(&self) -> Option<Arc<BillingOutboxRelay>> {
        self.inner.billing_outbox_relay.as_ref().map(Arc::clone)
    }

    /// Get a cloneable handle for explicit shutdown of the delivery worker.
    pub fn delivery_worker_handle(&self) -> Arc<DeliveryWorker> {
        Arc::clone(&self.inner.delivery_worker)
    }

    /// Record a billing event through the durable outbox (if enabled) or
    /// the in-memory batcher (fallback). This is the required path for
    /// financially authoritative writes.
    #[allow(clippy::too_many_arguments)]
    pub async fn record_billing_event_required(
        &self,
        tenant_id: uuid::Uuid,
        operation: &str,
        resource_id: Option<uuid::Uuid>,
        tokens_in: i64,
        tokens_out: i64,
        gpu_seconds: i32,
        cost_usd: f64,
        metadata: serde_json::Value,
    ) -> AppResult<()> {
        if self.inner.billing_outbox_relay.is_some() {
            crate::services::billing_outbox::enqueue(
                &self.inner.db,
                tenant_id,
                operation,
                resource_id,
                tokens_in,
                tokens_out,
                gpu_seconds,
                cost_usd,
                metadata,
            )
            .await?;
        } else {
            self.inner
                .billing_batcher
                .send(crate::services::billing_batcher::BillingEvent {
                    tenant_id,
                    operation: operation.to_string(),
                    resource_id,
                    tokens_in,
                    tokens_out,
                    gpu_seconds,
                    cost_usd,
                    metadata,
                });
        }
        Ok(())
    }

    /// Best-effort billing write for paths that intentionally do not fail the request.
    #[allow(clippy::too_many_arguments)]
    pub async fn record_billing_event_best_effort(
        &self,
        tenant_id: uuid::Uuid,
        operation: &str,
        resource_id: Option<uuid::Uuid>,
        tokens_in: i64,
        tokens_out: i64,
        gpu_seconds: i32,
        cost_usd: f64,
        metadata: serde_json::Value,
    ) {
        if let Err(e) = self
            .record_billing_event_required(
                tenant_id,
                operation,
                resource_id,
                tokens_in,
                tokens_out,
                gpu_seconds,
                cost_usd,
                metadata,
            )
            .await
        {
            tracing::error!(error = %e, "Failed to record billing event");
        }
    }
}
