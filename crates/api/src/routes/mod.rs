pub mod admin_instances;
pub mod admin_tenants;
pub mod api_keys;
pub mod audit_logs;
pub mod billing;
pub mod catalog;
pub mod dashboard;
pub mod datagen;
pub mod datasets;
pub mod deployments;
pub mod documents;
pub mod evaluations;
pub mod exports;
pub mod feedback;
pub mod health;
pub mod inference;
pub mod notifications;
pub mod pipeline;
pub mod projects;
pub mod stripe_webhooks;
pub mod teacher;
pub mod team;
pub mod tenant_settings;
pub mod training;
pub mod ws;

use axum::Router;
use utoipa::OpenApi;

use crate::app_state::AppState;
use crate::config::Config;

/// Build the complete API router with all versioned routes.
///
/// Auth + idempotency middleware are applied to the v1 sub-router only.
/// Health, inference (API key auth), and webhook (HMAC auth) routes are excluded.
///
/// Layer order (outside-in): auth runs first → idempotency reads from
/// extensions → handler reads from extensions. Single auth execution path.
pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .nest(
            "/api/v1",
            v1_router()
                // Axum layers are outside-in: last .layer() runs first.
                // 1. auth_middleware (outermost, runs first → inserts user into extensions)
                // 2. idempotency (inner, runs second → reads auth from extensions)
                .layer(axum::middleware::from_fn_with_state(
                    state.clone(),
                    crate::services::idempotency::idempotency_middleware,
                ))
                .layer(axum::middleware::from_fn_with_state(
                    state,
                    crate::auth::auth_middleware,
                )),
        )
        .merge(health::router())
        // Inference + feedback routes at /v1/ (OpenAI-compatible, API key auth)
        .merge(inference::router())
        .merge(feedback::api_router())
        // Stripe webhooks — NOT behind Clerk auth, mounted at /api/webhooks/stripe
        .merge(stripe_webhooks::router())
}

/// V1 API routes.
fn v1_router() -> Router<AppState> {
    Router::new()
        .merge(admin_instances::router())
        .merge(admin_tenants::router())
        .merge(projects::router())
        .merge(documents::router())
        .merge(pipeline::router())
        .merge(datagen::router())
        .merge(datasets::router())
        .merge(training::router())
        .merge(catalog::router())
        .merge(teacher::router())
        .merge(evaluations::router())
        .merge(exports::router())
        .merge(feedback::router())
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
        documents::delete_document,
        // Pipeline
        pipeline::trigger_parse,
        pipeline::trigger_refine,
        pipeline::get_status,
        // Datasets
        datasets::list_datasets,
        datasets::get_dataset,
        datasets::preview_dataset,
        datasets::import_dataset,
        datasets::get_parsed_content,
        // Data Studio
        datagen::create_data_guide,
        datagen::reset_data_guide,
        datagen::get_data_guide,
        datagen::start_facets,
        datagen::update_facets,
        datagen::start_preview,
        datagen::rate_samples,
        datagen::refine_guidance,
        datagen::update_guidance,
        datagen::generate_dataset,
        // Training
        training::create_training_job,
        training::list_training_jobs,
        training::get_training_job,
        training::cancel_training_job,
        training::stream_training_metrics,
        training::get_training_metrics,
        training::list_models,
        training::get_model,
        training::download_adapter,
        // Catalog
        catalog::get_catalog,
        // Teacher picker
        teacher::get_teacher_catalog,
        teacher::classify_teacher,
        // Evaluations
        evaluations::create_evaluation,
        evaluations::list_evaluations,
        evaluations::get_evaluation,
        // Exports
        exports::create_export,
        exports::list_exports,
        exports::download_export,
        exports::ollama_export_recipe,
        // API Keys
        api_keys::create_api_key,
        api_keys::list_api_keys,
        api_keys::revoke_api_key,
        // Admin inference instances
        admin_instances::list_instances,
        admin_instances::register_instance,
        admin_instances::update_lifecycle,
        admin_instances::delete_instance,
        // Admin tenant erasure
        admin_tenants::erase_tenant,
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
        notifications::list_in_app,
        notifications::mark_in_app_read,
        notifications::mark_all_in_app_read,
        // Audit Logs
        audit_logs::list_audit_logs,
        // Settings
        tenant_settings::get_llm_settings,
        tenant_settings::test_llm_settings,
        tenant_settings::update_llm_settings,
        tenant_settings::delete_llm_settings,
        // Inference
        inference::chat_completions,
        // Feedback (data flywheel)
        feedback::list_samples,
        feedback::promote_samples,
        feedback::set_capture,
        feedback::submit_feedback,
        feedback::submit_api_feedback,
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
        crate::dto::dataset::DatasetImportResponse,
        crate::dto::dataset::DatasetImportRowError,
        datasets::ParsedContentResponse,
        // Data Studio
        crate::dto::datagen::Facet,
        crate::dto::datagen::PreviewSample,
        crate::dto::datagen::DataGuideResponse,
        crate::dto::datagen::CreateDataGuideRequest,
        crate::dto::datagen::GenerateFacetsRequest,
        crate::dto::datagen::UpdateFacetsRequest,
        crate::dto::datagen::GeneratePreviewRequest,
        crate::dto::datagen::SampleRatingItem,
        crate::dto::datagen::RateSamplesRequest,
        crate::dto::datagen::UpdateGuidanceRequest,
        crate::dto::datagen::GenerateDatasetRequest,
        // Teacher picker
        crate::services::teacher::config::TeacherConfigDto,
        crate::services::teacher::config::TeacherProvenance,
        crate::services::teacher::policy::ProviderPolicy,
        crate::services::teacher::policy::TeacherCatalogEntry,
        teacher::ClassifyTeacherRequest,
        teacher::ClassifyTeacherResponse,
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
        crate::dto::export::OllamaExportResponse,
        // API Keys
        crate::dto::api_key::CreateApiKeyRequest,
        crate::dto::api_key::CreateApiKeyResponse,
        crate::dto::api_key::ApiKeyResponse,
        // Admin inference instances
        crate::dto::inference_instance::CreateInferenceInstanceRequest,
        crate::dto::inference_instance::UpdateInferenceInstanceLifecycleRequest,
        crate::dto::inference_instance::InferenceInstanceResponse,
        // Admin tenant erasure
        crate::dto::admin::TenantErasureSummary,
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
        crate::dto::notification::InAppNotificationResponse,
        crate::dto::notification::InAppNotificationsResponse,
        // Inference
        inference::ChatCompletionRequest,
        inference::ChatMessage,
        inference::ChatCompletionResponse,
        inference::ChatChoice,
        inference::ChatUsage,
        // Feedback (data flywheel)
        crate::dto::feedback::SampleMessage,
        crate::dto::feedback::InferenceSampleResponse,
        crate::dto::feedback::SubmitFeedbackRequest,
        crate::dto::feedback::ApiFeedbackRequest,
        crate::dto::feedback::SetCaptureRequest,
        crate::dto::feedback::PromoteSampleItem,
        crate::dto::feedback::PromoteSamplesRequest,
        crate::dto::feedback::PromoteSamplesResponse,
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
        platform_shared::enums::InferenceInstanceHealthStatus,
        platform_shared::enums::InferenceInstanceLifecycleState,
        platform_shared::enums::EvaluationStatus,
        platform_shared::enums::PipelineStage,
        platform_shared::enums::TaskType,
        platform_shared::enums::DataGuideStatus,
        platform_shared::enums::SampleRating,
        platform_shared::enums::FeedbackRating,
        platform_shared::enums::Plan,
        platform_shared::enums::BillingOperation,
        platform_shared::enums::GpuClass,
        platform_shared::enums::ProjectStatus,
        platform_shared::enums::TeamRole,
        platform_shared::enums::InvitationStatus,
        // Tenant Settings
        crate::dto::tenant_settings::UpdateLlmSettingsRequest,
        crate::dto::tenant_settings::LlmSettingsResponse,
        crate::dto::tenant_settings::LlmTestResponse,
        // Shared types
        platform_shared::types::Hyperparams,
        platform_shared::types::TrainingMetrics,
        platform_shared::types::EvaluationScores,
        platform_shared::types::DomainScores,
        platform_shared::types::GeneralScores,
        platform_shared::types::ABComparisonScores,
        platform_shared::types::SafetyScores,
        platform_shared::types::DocKnowledgeScores,
        platform_shared::types::DeploymentConfig,
    )),
    tags(
        (name = "Health", description = "Health and readiness checks"),
        (name = "Projects", description = "Project CRUD operations"),
        (name = "Documents", description = "Document upload and management"),
        (name = "Pipeline", description = "Pipeline trigger and status"),
        (name = "Datasets", description = "Dataset management and preview"),
        (name = "Data Studio", description = "Guided synthetic-data generation sessions"),
        (name = "Training", description = "Training job management and models"),
        (name = "Evaluations", description = "Model evaluation"),
        (name = "Exports", description = "Dataset and model exports"),
        (name = "API Keys", description = "API key management for inference"),
        (name = "Admin", description = "Platform infrastructure administration"),
        (name = "Deployments", description = "Model deployment management"),
        (name = "Billing", description = "Billing, subscriptions, and usage"),
        (name = "Dashboard", description = "Dashboard statistics and activity"),
        (name = "Team", description = "Team member and invitation management"),
        (name = "Notifications", description = "Notification preferences and deliveries"),
        (name = "Audit Logs", description = "Audit log entries"),
        (name = "Settings", description = "Per-tenant configuration (LLM providers, etc.)"),
        (name = "Inference", description = "OpenAI-compatible chat completions API"),
        (name = "Feedback", description = "Captured inference traffic and response feedback"),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Axum panics at ROUTE-REGISTRATION time on a path-shape conflict between
    /// merged routers or on invalid segment syntax (e.g. pre-0.8 `:id`). Both
    /// classes shipped as boot blockers once (#94) because nothing constructed
    /// the full router in CI. This composes the exact same route tree as
    /// `router()` minus the state-bound middleware layers, so any registration
    /// panic fails this test instead of production startup.
    #[test]
    fn full_route_tree_composes_without_panicking() {
        let _app: Router<AppState> = Router::new()
            .nest("/api/v1", v1_router())
            .merge(health::router())
            .merge(inference::router())
            .merge(feedback::api_router())
            .merge(stripe_webhooks::router());
    }
}
