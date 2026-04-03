pub mod api_keys;
pub mod audit_logs;
pub mod billing;
pub mod catalog;
pub mod dashboard;
pub mod datasets;
pub mod deployments;
pub mod documents;
pub mod evaluations;
pub mod exports;
pub mod health;
pub mod inference;
pub mod notifications;
pub mod pipeline;
pub mod projects;
pub mod stripe_webhooks;
pub mod team;
pub mod tenant_settings;
pub mod training;
pub mod ws;

use axum::Router;
use utoipa::OpenApi;

use crate::app_state::AppState;
use crate::config::Config;

/// Build the complete API router with all versioned routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .nest("/api/v1", v1_router())
        .merge(health::router())
        // Inference routes at /v1/ (OpenAI-compatible, API key auth)
        .merge(inference::router())
        // Stripe webhooks — NOT behind Clerk auth, mounted at /api/webhooks/stripe
        .merge(stripe_webhooks::router())
}

/// V1 API routes.
fn v1_router() -> Router<AppState> {
    Router::new()
        .merge(projects::router())
        .merge(documents::router())
        .merge(pipeline::router())
        .merge(datasets::router())
        .merge(training::router())
        .merge(catalog::router())
        .merge(evaluations::router())
        .merge(exports::router())
        .merge(api_keys::router())
        .merge(deployments::router())
        .merge(billing::router())
        .merge(audit_logs::router())
        .merge(dashboard::router())
        .merge(notifications::router())
        .merge(tenant_settings::router())
        .merge(ws::router())
        .merge(team::router())
        .merge(team::public_router())
}

/// OpenAPI documentation spec.
#[derive(OpenApi)]
#[openapi(
    paths(
        // Health
        health::health,
        health::ready,
        // Projects
        projects::create_project,
        projects::list_projects,
        projects::get_project,
        projects::update_project,
        projects::delete_project,
        // Documents
        documents::upload_document,
        documents::list_documents,
        documents::get_document,
        // Pipeline
        pipeline::trigger_parse,
        pipeline::trigger_refine,
        pipeline::get_status,
        // Datasets
        datasets::list_datasets,
        datasets::get_dataset,
        datasets::preview_dataset,
        datasets::get_parsed_content,
        // Training
        training::create_training_job,
        training::list_training_jobs,
        training::get_training_job,
        training::cancel_training_job,
        training::stream_training_metrics,
        training::get_training_metrics,
        training::list_models,
        training::get_model,
        // Catalog
        catalog::get_catalog,
        // Evaluations
        evaluations::create_evaluation,
        evaluations::list_evaluations,
        evaluations::get_evaluation,
        // Exports
        exports::create_export,
        exports::list_exports,
        exports::download_export,
        // API Keys
        api_keys::create_api_key,
        api_keys::list_api_keys,
        api_keys::revoke_api_key,
        // Deployments
        deployments::deploy_model,
        deployments::undeploy_model,
        deployments::get_deployment_status,
        // Billing
        billing::list_billing_events,
        billing::get_usage_summary,
        billing::create_checkout,
        billing::create_portal_session,
        billing::get_subscription,
        billing::get_plan_limits,
        // Dashboard
        dashboard::get_stats,
        dashboard::get_usage,
        dashboard::get_inference_usage,
        dashboard::get_activity,
        // Team
        team::list_members,
        team::create_invitation,
        team::list_invitations,
        team::revoke_invitation,
        team::accept_invitation,
        team::update_role,
        team::remove_member,
        // Notifications
        notifications::list_preferences,
        notifications::update_preferences,
        notifications::list_deliveries,
        // Audit Logs
        audit_logs::list_audit_logs,
        // Settings
        tenant_settings::get_llm_settings,
        tenant_settings::update_llm_settings,
        tenant_settings::delete_llm_settings,
        // Inference
        inference::chat_completions,
        // Webhooks
        stripe_webhooks::handle_stripe_webhook,
    ),
    components(schemas(
        // Health
        health::HealthResponse,
        health::ReadyResponse,
        // Error
        crate::error::ErrorEnvelope,
        crate::error::ErrorBody,
        // Projects
        crate::dto::project::CreateProjectRequest,
        crate::dto::project::UpdateProjectRequest,
        crate::dto::project::ProjectResponse,
        // Documents
        crate::dto::document::DocumentResponse,
        crate::dto::document::UploadResponse,
        // Datasets
        crate::dto::dataset::DatasetResponse,
        datasets::ParsedContentResponse,
        // Pipeline
        crate::dto::pipeline::TriggerParseResponse,
        crate::dto::pipeline::TriggerRefineRequest,
        crate::dto::pipeline::TriggerRefineResponse,
        crate::dto::pipeline::ProjectPipelineStatus,
        crate::dto::pipeline::DocumentStatusCounts,
        crate::dto::pipeline::DatasetStatusCounts,
        crate::dto::pipeline::TrainingJobStatusCounts,
        crate::dto::pipeline::ModelStatusCounts,
        crate::dto::pipeline::EvaluationStatusCounts,
        // Training
        crate::dto::training_job::CreateTrainingJobRequest,
        crate::dto::training_job::TrainingJobResponse,
        // Models
        crate::dto::model::ModelResponse,
        // Evaluations
        crate::dto::evaluation::CreateEvaluationRequest,
        crate::dto::evaluation::EvaluationResponse,
        // Exports
        crate::dto::export::ExportRequest,
        crate::dto::export::ExportResponse,
        crate::dto::export::ExportDownloadResponse,
        // API Keys
        crate::dto::api_key::CreateApiKeyRequest,
        crate::dto::api_key::CreateApiKeyResponse,
        crate::dto::api_key::ApiKeyResponse,
        // Billing
        crate::dto::billing::BillingEventResponse,
        // Audit Logs
        crate::dto::audit_log::AuditLogResponse,
        // Dashboard
        crate::dto::dashboard::DashboardStats,
        crate::dto::dashboard::UsageSummary,
        crate::dto::dashboard::ActivityEntry,
        crate::repositories::billing_event_repo::InferenceUsageDay,
        // Team
        crate::dto::team::InviteRequest,
        crate::dto::team::UpdateRoleRequest,
        crate::dto::team::TeamMemberResponse,
        crate::dto::team::InvitationResponse,
        // Notifications
        crate::dto::notification::NotificationPreferenceResponse,
        crate::dto::notification::UpdatePreferencesRequest,
        crate::dto::notification::PreferenceUpdate,
        crate::dto::notification::NotificationDeliveryResponse,
        // Inference
        inference::ChatCompletionRequest,
        inference::ChatMessage,
        inference::ChatCompletionResponse,
        inference::ChatChoice,
        inference::ChatUsage,
        // Stripe
        crate::dto::stripe::CreateCheckoutRequest,
        crate::dto::stripe::CheckoutSessionResponse,
        crate::dto::stripe::CreatePortalRequest,
        crate::dto::stripe::PortalSessionResponse,
        crate::dto::stripe::SubscriptionResponse,
        // Deployments
        crate::services::deployment_service::DeploymentStatusResponse,
        // Plan limits
        crate::services::plan_service::PlanLimits,
        // Catalog
        catalog::CatalogModel,
        catalog::CatalogResponse,
        catalog::CatalogQuery,
        // Shared enums
        platform_shared::enums::DocumentStatus,
        platform_shared::enums::DatasetStatus,
        platform_shared::enums::TrainingJobStatus,
        platform_shared::enums::TrainingMethod,
        platform_shared::enums::TrainingMode,
        platform_shared::enums::DeploymentStatus,
        platform_shared::enums::EvaluationStatus,
        platform_shared::enums::PipelineStage,
        platform_shared::enums::TaskType,
        platform_shared::enums::Plan,
        platform_shared::enums::BillingOperation,
        platform_shared::enums::GpuClass,
        platform_shared::enums::ProjectStatus,
        platform_shared::enums::TeamRole,
        platform_shared::enums::InvitationStatus,
        // Tenant Settings
        crate::dto::tenant_settings::UpdateLlmSettingsRequest,
        crate::dto::tenant_settings::LlmSettingsResponse,
        // Shared types
        platform_shared::types::Hyperparams,
        platform_shared::types::TrainingMetrics,
        platform_shared::types::EvaluationScores,
        platform_shared::types::DomainScores,
        platform_shared::types::GeneralScores,
        platform_shared::types::ABComparisonScores,
        platform_shared::types::SafetyScores,
        platform_shared::types::DeploymentConfig,
    )),
    tags(
        (name = "Health", description = "Health and readiness checks"),
        (name = "Projects", description = "Project CRUD operations"),
        (name = "Documents", description = "Document upload and management"),
        (name = "Pipeline", description = "Pipeline trigger and status"),
        (name = "Datasets", description = "Dataset management and preview"),
        (name = "Training", description = "Training job management and models"),
        (name = "Evaluations", description = "Model evaluation"),
        (name = "Exports", description = "Dataset and model exports"),
        (name = "API Keys", description = "API key management for inference"),
        (name = "Deployments", description = "Model deployment management"),
        (name = "Billing", description = "Billing, subscriptions, and usage"),
        (name = "Dashboard", description = "Dashboard statistics and activity"),
        (name = "Team", description = "Team member and invitation management"),
        (name = "Notifications", description = "Notification preferences and deliveries"),
        (name = "Audit Logs", description = "Audit log entries"),
        (name = "Settings", description = "Per-tenant configuration (LLM providers, etc.)"),
        (name = "Inference", description = "OpenAI-compatible chat completions API"),
        (name = "Webhooks", description = "Stripe webhook handler"),
    ),
    modifiers(&SecurityAddon),
)]
pub struct ApiDoc;

/// Adds JWT and API key security schemes to the OpenAPI spec.
struct SecurityAddon;

impl utoipa::Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "jwt",
            utoipa::openapi::security::SecurityScheme::Http(utoipa::openapi::security::Http::new(
                utoipa::openapi::security::HttpAuthScheme::Bearer,
            )),
        );
        components.add_security_scheme(
            "api_key",
            utoipa::openapi::security::SecurityScheme::Http(utoipa::openapi::security::Http::new(
                utoipa::openapi::security::HttpAuthScheme::Bearer,
            )),
        );
    }
}

/// Build the Swagger UI router (only in non-production environments).
pub fn docs_router(config: &Config) -> Option<Router<AppState>> {
    if config.environment == "production" {
        return None;
    }

    let swagger_ui =
        utoipa_swagger_ui::SwaggerUi::new("/docs").url("/api-docs/openapi.json", ApiDoc::openapi());

    Some(Router::new().merge(swagger_ui))
}
