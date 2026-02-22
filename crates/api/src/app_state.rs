use platform_storage::s3::{S3Config, S3Storage};
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;

use crate::auth::{AuthProviderChain, ClerkAuthProvider};
use crate::config::Config;
use crate::services::billing_batcher::BillingBatcher;
use crate::services::circuit_breaker::CircuitBreaker;
use crate::repositories::api_key_repo::PgApiKeyRepo;
use crate::repositories::audit_log_repo::PgAuditLogRepo;
use crate::repositories::billing_event_repo::PgBillingEventRepo;
use crate::repositories::dataset_repo::PgDatasetRepo;
use crate::repositories::document_repo::PgDocumentRepo;
use crate::repositories::evaluation_repo::PgEvaluationRepo;
use crate::repositories::export_repo::PgExportRepo;
use crate::repositories::invitation_repo::PgInvitationRepo;
use crate::repositories::model_repo::PgModelRepo;
use crate::repositories::notification_repo::PgNotificationRepo;
use crate::repositories::project_repo::PgProjectRepo;
use crate::repositories::team_member_repo::PgTeamMemberRepo;
use crate::repositories::tenant_repo::PgTenantRepo;
use crate::repositories::training_job_repo::PgTrainingJobRepo;
use crate::repositories::traits::{
    ApiKeyRepository, AuditLogRepository, BillingEventRepository, DatasetRepository,
    DocumentRepository, EvaluationRepository, ExportRepository, InvitationRepository,
    ModelRepository, NotificationRepository, ProjectRepository, TeamMemberRepository,
    TenantRepository, TrainingJobRepository,
};
use crate::services::billing_provider::BillingProvider;
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
    pub db: PgPool,
    pub redis: redis::aio::ConnectionManager,
    pub storage: S3Storage,
    pub orchestrator: Option<Arc<dyn WorkflowOrchestrator>>,
    pub auth_chain: AuthProviderChain,
    pub http_client: reqwest::Client,
    // Repository trait objects
    pub project_repo: Arc<dyn ProjectRepository>,
    pub document_repo: Arc<dyn DocumentRepository>,
    pub dataset_repo: Arc<dyn DatasetRepository>,
    pub training_job_repo: Arc<dyn TrainingJobRepository>,
    pub model_repo: Arc<dyn ModelRepository>,
    pub evaluation_repo: Arc<dyn EvaluationRepository>,
    pub export_repo: Arc<dyn ExportRepository>,
    pub api_key_repo: Arc<dyn ApiKeyRepository>,
    pub billing_event_repo: Arc<dyn BillingEventRepository>,
    pub audit_log_repo: Arc<dyn AuditLogRepository>,
    pub team_member_repo: Arc<dyn TeamMemberRepository>,
    pub invitation_repo: Arc<dyn InvitationRepository>,
    pub notification_repo: Arc<dyn NotificationRepository>,
    pub tenant_repo: Arc<dyn TenantRepository>,
    pub billing_provider: Arc<dyn BillingProvider>,
    pub vllm_circuit_breaker: CircuitBreaker,
    pub billing_batcher: Arc<BillingBatcher>,
}

impl AppState {
    /// Build application state from config.
    ///
    /// Initializes all connections: database pool, Redis, S3 client.
    /// Fails fast if any connection cannot be established.
    pub async fn new(config: Config) -> anyhow::Result<Self> {
        // Database
        let db = platform_db::create_pool(&config.database_url, config.database_max_connections)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to connect to PostgreSQL: {e}"))?;

        tracing::info!("Connected to PostgreSQL");

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
                    "ml-pipeline",
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
            .unwrap_or_default();

        // Auth provider chain (uses shared HTTP client for JWKS fetching)
        let auth_chain = AuthProviderChain::new().add(ClerkAuthProvider::new(
            config.clerk_jwks_url.clone(),
            config.is_dev(),
            http_client.clone(),
        ));

        tracing::info!("Auth provider chain initialized");

        // Repository trait objects (PgPool is Arc<PoolInner>, cheap to clone)
        let project_repo: Arc<dyn ProjectRepository> = Arc::new(PgProjectRepo::new(db.clone()));
        let document_repo: Arc<dyn DocumentRepository> = Arc::new(PgDocumentRepo::new(db.clone()));
        let dataset_repo: Arc<dyn DatasetRepository> = Arc::new(PgDatasetRepo::new(db.clone()));
        let training_job_repo: Arc<dyn TrainingJobRepository> =
            Arc::new(PgTrainingJobRepo::new(db.clone()));
        let model_repo: Arc<dyn ModelRepository> = Arc::new(PgModelRepo::new(db.clone()));
        let evaluation_repo: Arc<dyn EvaluationRepository> =
            Arc::new(PgEvaluationRepo::new(db.clone()));
        let export_repo: Arc<dyn ExportRepository> = Arc::new(PgExportRepo::new(db.clone()));
        let api_key_repo: Arc<dyn ApiKeyRepository> = Arc::new(PgApiKeyRepo::new(db.clone()));
        let billing_event_repo: Arc<dyn BillingEventRepository> =
            Arc::new(PgBillingEventRepo::new(db.clone()));
        let audit_log_repo: Arc<dyn AuditLogRepository> = Arc::new(PgAuditLogRepo::new(db.clone()));
        let team_member_repo: Arc<dyn TeamMemberRepository> =
            Arc::new(PgTeamMemberRepo::new(db.clone()));
        let invitation_repo: Arc<dyn InvitationRepository> =
            Arc::new(PgInvitationRepo::new(db.clone()));
        let notification_repo: Arc<dyn NotificationRepository> =
            Arc::new(PgNotificationRepo::new(db.clone()));
        let tenant_repo: Arc<dyn TenantRepository> = Arc::new(PgTenantRepo::new(db.clone()));

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

        // Circuit breaker for vLLM calls (configurable via VLLM_CB_FAILURE_THRESHOLD / VLLM_CB_RECOVERY_TIMEOUT_SECS)
        let vllm_circuit_breaker = CircuitBreaker::new(
            config.vllm_cb_failure_threshold,
            Duration::from_secs(config.vllm_cb_recovery_timeout_secs),
        );

        // Billing micro-batcher (10K channel capacity, flush every 5s or 1000 events)
        let billing_batcher = Arc::new(BillingBatcher::new(
            db.clone(),
            10_000,
            1_000,
            Duration::from_secs(5),
        ));

        tracing::info!("Infrastructure hardening initialized (circuit breaker + billing batcher)");

        Ok(Self {
            inner: Arc::new(AppStateInner {
                config,
                db,
                redis,
                storage,
                orchestrator,
                auth_chain,
                http_client,
                project_repo,
                document_repo,
                dataset_repo,
                training_job_repo,
                model_repo,
                evaluation_repo,
                export_repo,
                api_key_repo,
                billing_event_repo,
                audit_log_repo,
                team_member_repo,
                invitation_repo,
                notification_repo,
                tenant_repo,
                billing_provider,
                vllm_circuit_breaker,
                billing_batcher,
            }),
        })
    }

    pub fn config(&self) -> &Config {
        &self.inner.config
    }

    pub fn db(&self) -> &PgPool {
        &self.inner.db
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

    pub fn vllm_circuit_breaker(&self) -> &CircuitBreaker {
        &self.inner.vllm_circuit_breaker
    }

    pub fn billing_batcher(&self) -> &BillingBatcher {
        &self.inner.billing_batcher
    }

    /// Get a cloneable handle for explicit shutdown of the billing batcher.
    pub fn billing_batcher_handle(&self) -> Arc<BillingBatcher> {
        Arc::clone(&self.inner.billing_batcher)
    }
}
