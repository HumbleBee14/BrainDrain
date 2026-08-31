mod app_state;
mod auth;
mod auth_api_key;
mod config;
mod dto;
mod error;
mod middleware;
mod rbac;
mod repositories;
mod routes;
mod services;
mod temporal;

use std::net::SocketAddr;

use opentelemetry::KeyValue;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::Resource;
use tokio::net::TcpListener;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

use app_state::AppState;
use config::Config;
use services::feature_flags::{
    BILLING_OUTBOX_ENABLED, DEPLOYMENTS_MULTI_INSTANCE_ENABLED, FlagContext, IDEMPOTENCY_ENFORCED,
    INFERENCE_BACKEND_TGI_ENABLED, NOTIFICATIONS_DELIVERY_WORKER_ENABLED,
};
use services::idle_backoff::IdleBackoff;
use services::inference_instance_service::InferenceInstanceService;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // rustls links both aws-lc-rs and ring (AWS SDK + redis TLS), so the provider
    // must be selected explicitly before any TLS handshake.
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .map_err(|_| anyhow::anyhow!("failed to install rustls CryptoProvider"))?;

    // Load configuration from environment
    let config = Config::from_env().map_err(|e| anyhow::anyhow!("Config error: {e}"))?;

    // Initialize structured logging with optional OTEL export
    init_tracing(&config)?;

    // Teacher policy and the hosted-model allowlist are deployment data. Loaded
    // before anything can read them, and a bad override fails startup rather
    // than silently leaving the defaults in place — an operator who edited the
    // catalog must not discover at request time that it was ignored.
    init_teacher_policy(&config)?;

    tracing::info!(
        app = %config.app_name,
        env = %config.environment,
        otel = config.otel_enabled,
        "Starting API server"
    );

    // Build application state (connects to DB, Redis, S3). Migrations run inside
    // AppState::new on the owner pool — before the RLS pool is created — because
    // migration 017 provisions the `app_rls` role that pool connects as.
    let state = AppState::new(config.clone()).await?;

    // Ensure billing partitions exist for the next 3 months.
    // Runs on every startup (idempotent, <1ms). Without this, inserts into
    // future months would fail on the partitioned billing_events table.
    if let Err(e) = platform_db::ensure_billing_partitions(state.db(), 3).await {
        tracing::warn!(
            "Failed to ensure billing partitions: {e} — billing inserts for future months may fail"
        );
    }

    // Spawn background task: cleanup expired + stale idempotency keys every hour.
    let (idempotency_shutdown_tx, mut idempotency_shutdown_rx) =
        tokio::sync::oneshot::channel::<()>();
    {
        let cleanup_db = state.db().clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        match services::idempotency::cleanup_expired_keys(&cleanup_db).await {
                            Ok(count) if count > 0 => {
                                tracing::info!(count, "Cleaned up expired idempotency keys");
                            }
                            Err(e) => {
                                tracing::warn!(error = %e, "Failed to cleanup idempotency keys");
                            }
                            _ => {}
                        }
                    }
                    _ = &mut idempotency_shutdown_rx => {
                        tracing::info!("Idempotency cleanup task shutting down");
                        return;
                    }
                }
            }
        });
    }

    // Spawn background task: inference instance health probes + capacity reconciliation.
    let (instance_control_shutdown_tx, mut instance_control_shutdown_rx) =
        tokio::sync::oneshot::channel::<()>();
    {
        let control_state = state.clone();
        let health_interval_secs = config.inference_instance_health_poll_interval_secs;
        let idle_backoff_max = std::time::Duration::from_secs(config.db_idle_backoff_max_secs);
        let reconcile_interval_secs = config.inference_instance_reconcile_interval_secs;
        tokio::spawn(async move {
            let mut health_backoff = IdleBackoff::new(
                std::time::Duration::from_secs(health_interval_secs),
                idle_backoff_max,
            );
            let mut reconcile_backoff = IdleBackoff::new(
                std::time::Duration::from_secs(reconcile_interval_secs),
                idle_backoff_max,
            );

            loop {
                tokio::select! {
                    _ = tokio::time::sleep(health_backoff.interval()) => {
                        match InferenceInstanceService::run_health_probes(&control_state).await {
                            Ok(0) => health_backoff.saw_idle(),
                            Ok(_) => health_backoff.saw_work(),
                            Err(e) => {
                                tracing::warn!(error = %e, "Inference instance health loop failed");
                                health_backoff.saw_idle();
                            }
                        }
                    }
                    _ = tokio::time::sleep(reconcile_backoff.interval()) => {
                        match control_state.inference_instance_repo().reconcile_adapter_counts().await {
                            Ok(repaired) if repaired > 0 => {
                                tracing::warn!(repaired, "Reconciled inference instance adapter counts");
                                reconcile_backoff.saw_work();
                            }
                            Ok(_) => reconcile_backoff.saw_idle(),
                            Err(e) => {
                                tracing::warn!(error = %e, "Inference instance reconciliation failed");
                                reconcile_backoff.saw_idle();
                            }
                        }
                    }
                    _ = &mut instance_control_shutdown_rx => {
                        tracing::info!("Inference instance control-plane task shutting down");
                        return;
                    }
                }
            }
        });
    }

    // Spawn background task: reap stuck training jobs + parsing documents.
    let (reaper_shutdown_tx, mut reaper_shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    {
        let reaper_state = state.clone();
        let poll_secs = config.reaper_poll_interval_secs;
        let reaper_idle_backoff_max =
            std::time::Duration::from_secs(config.db_idle_backoff_max_secs);
        let training_stuck = config.training_stuck_timeout_secs;
        let parsing_stuck = config.parsing_stuck_timeout_secs;
        let deploying_stuck = config.deploying_stuck_timeout_secs;
        let idle_instance_timeout = config.inference_instance_idle_timeout_secs;
        let orphan_sweep_secs = config.orphaned_document_sweep_secs;
        tokio::spawn(async move {
            let mut backoff = IdleBackoff::new(
                std::time::Duration::from_secs(poll_secs),
                reaper_idle_backoff_max,
            );
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(backoff.interval()) => {
                        let orch = reaper_state.orchestrator();
                        let mut reaped = 0usize;
                        match services::reaper::reap_stuck_training_jobs(
                            reaper_state.db(), orch, training_stuck,
                        ).await {
                            Ok(n) if n > 0 => { reaped += n; tracing::warn!(count = n, "Reaped stuck training jobs") }
                            Err(e) => tracing::warn!(error = %e, "Stuck-training reaper failed"),
                            _ => {}
                        }
                        match services::reaper::reap_stuck_parsing_documents(
                            reaper_state.db(), parsing_stuck,
                        ).await {
                            Ok(n) if n > 0 => { reaped += n as usize; tracing::warn!(count = n, "Reaped stuck parsing documents") }
                            Err(e) => tracing::warn!(error = %e, "Stuck-parsing reaper failed"),
                            _ => {}
                        }
                        match services::reaper::reap_stuck_deploying_models(
                            reaper_state.db(), deploying_stuck,
                        ).await {
                            Ok(n) if n > 0 => { reaped += n; tracing::warn!(count = n, "Reaped stuck deploying models") }
                            Err(e) => tracing::warn!(error = %e, "Stuck-deploy reaper failed"),
                            _ => {}
                        }
                        match services::reaper::reap_idle_instances(
                            reaper_state.db(), idle_instance_timeout,
                        ).await {
                            Ok(n) if n > 0 => { reaped += n; tracing::warn!(count = n, "Scaled idle serving instances to zero") }
                            Err(e) => tracing::warn!(error = %e, "Idle-instance reaper failed"),
                            _ => {}
                        }
                        match services::reaper::sweep_orphaned_document_objects(
                            reaper_state.db(), reaper_state.storage(), orphan_sweep_secs,
                        ).await {
                            Ok(n) if n > 0 => { reaped += n; tracing::info!(count = n, "Reclaimed orphaned document objects") }
                            Err(e) => tracing::warn!(error = %e, "Orphaned-object sweep failed"),
                            _ => {}
                        }
                        if reaped > 0 {
                            backoff.saw_work();
                        } else {
                            backoff.saw_idle();
                        }
                    }
                    _ = &mut reaper_shutdown_rx => {
                        tracing::info!("Stuck-job reaper shutting down");
                        return;
                    }
                }
            }
        });
    }

    let default_flag_context = FlagContext::default();
    tracing::info!(
        billing_outbox = state
            .feature_flags()
            .is_enabled(BILLING_OUTBOX_ENABLED, &default_flag_context),
        idempotency = state
            .feature_flags()
            .is_enabled(IDEMPOTENCY_ENFORCED, &default_flag_context),
        deployments_multi_instance = state
            .feature_flags()
            .is_enabled(DEPLOYMENTS_MULTI_INSTANCE_ENABLED, &default_flag_context),
        delivery_worker = state.feature_flags().bool_variation(
            NOTIFICATIONS_DELIVERY_WORKER_ENABLED,
            true,
            &default_flag_context,
        ),
        tgi_backend = state
            .feature_flags()
            .is_enabled(INFERENCE_BACKEND_TGI_ENABLED, &default_flag_context),
        "Feature flags initialized"
    );

    // Build middleware stack
    let cors_origins = config.cors_origins_list();
    let (set_request_id, propagate_request_id) = middleware::request_id_layers();
    let security_config = middleware::SecurityHeadersConfig::new(
        &config.security_csp_policy,
        config.security_hsts_max_age,
    );
    let http_metrics = middleware::HttpMetrics::new(&config);
    let ip_rate_limiter = middleware::IpRateLimiter::new(state.redis(), &config);

    // Grab background worker handles before moving state into the router.
    // These are Arc-cloned or extracted here because `state` is consumed by
    // the router builder and cannot be accessed after.
    let billing_batcher = state.billing_batcher_handle();
    let billing_outbox_relay = state.billing_outbox_relay_handle();
    let delivery_worker = state.delivery_worker_handle();
    let shutdown_state = state.clone();

    // Build router (layers applied outside-in: last .layer() is outermost)
    // Auth + idempotency middleware are applied inside v1_router (see routes/mod.rs).
    // Request flow: set_request_id → cors → security_headers → trace → ip_rate_limit → http_metrics → propagate_request_id → inject_request_id_into_errors → [v1: auth → idempotency →] handler
    let mut app = routes::router(state.clone());

    // Mount Swagger UI docs in non-production environments
    if let Some(docs) = routes::docs_router(&config) {
        tracing::info!("Swagger UI enabled at /docs");
        app = app.merge(docs);
    }

    let app = app
        .with_state(state)
        .layer(axum::middleware::from_fn(
            error::inject_request_id_into_errors,
        ))
        .layer(propagate_request_id)
        .layer(axum::middleware::from_fn_with_state(
            http_metrics,
            middleware::http_metrics,
        ))
        .layer(axum::middleware::from_fn_with_state(
            ip_rate_limiter,
            middleware::ip_rate_limit,
        ))
        .layer(middleware::trace_layer())
        .layer(axum::middleware::from_fn_with_state(
            security_config,
            middleware::security_headers,
        ))
        .layer(middleware::cors_layer(&cors_origins))
        .layer(set_request_id);

    // Start server
    let addr = SocketAddr::from(([0, 0, 0, 0], config.api_port));
    let listener = TcpListener::bind(addr).await?;

    tracing::info!(%addr, "Server listening");

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;

    // Shut down background workers after server stops accepting requests
    tracing::info!("Shutting down background workers...");
    let _ = idempotency_shutdown_tx.send(());
    let _ = instance_control_shutdown_tx.send(());
    let _ = reaper_shutdown_tx.send(());

    // Stop Unleash poller if running
    shutdown_state.shutdown_unleash_poller();

    if let Some(relay) = billing_outbox_relay {
        tokio::join!(
            billing_batcher.shutdown(),
            relay.shutdown(),
            delivery_worker.shutdown()
        );
    } else {
        tokio::join!(billing_batcher.shutdown(), delivery_worker.shutdown());
    }

    tracing::info!("Server shutdown complete");
    Ok(())
}

/// Initialize tracing subscriber with optional OpenTelemetry export.
///
/// Application code uses `tracing::info!()` / `tracing::span!()` exclusively.
/// When `otel_enabled=true`, spans are bridged to OTEL via `tracing-opentelemetry`.
/// Swapping OTEL for Datadog/New Relic means changing only this function.
/// Install operator overrides for the hosted-teacher allowlist and the
/// restricted-provider list.
fn init_teacher_policy(config: &Config) -> anyhow::Result<()> {
    if let Some(path) = &config.hosted_teacher_catalog_path {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("cannot read hosted teacher catalog {path}: {e}"))?;
        let entries = serde_json::from_str(&raw)
            .map_err(|e| anyhow::anyhow!("hosted teacher catalog {path} is not valid: {e}"))?;
        services::teacher::hosted::init_hosted_catalog(entries)
            .map_err(|e| anyhow::anyhow!("hosted teacher catalog {path} rejected: {e}"))?;
        tracing::info!(
            path = %path,
            teachers = services::teacher::hosted::hosted_catalog().len(),
            "Loaded operator hosted-teacher catalog"
        );
    }

    if let Some(hosts) = &config.restricted_teacher_hosts {
        let hosts: Vec<String> = hosts.split(',').map(str::to_string).collect();
        services::teacher::policy::init_restricted_hosts(hosts)
            .map_err(|_| anyhow::anyhow!("restricted teacher hosts already initialized"))?;
        tracing::info!(
            hosts = services::teacher::policy::restricted_hosts().len(),
            "Loaded operator restricted-provider list"
        );
    }

    Ok(())
}

fn init_tracing(config: &Config) -> anyhow::Result<()> {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&config.log_level));

    let fmt_layer = if config.is_dev() {
        tracing_subscriber::fmt::layer().with_target(true).boxed()
    } else {
        tracing_subscriber::fmt::layer().json().boxed()
    };

    let registry = tracing_subscriber::registry().with(filter).with(fmt_layer);

    if config.otel_enabled {
        let resource = Resource::builder()
            .with_attribute(KeyValue::new("service.name", config.app_name.clone()))
            .with_attribute(KeyValue::new(
                "deployment.environment",
                config.environment.clone(),
            ))
            .build();

        let exporter = opentelemetry_otlp::SpanExporter::builder()
            .with_tonic()
            .with_endpoint(&config.otel_endpoint)
            .build()
            .map_err(|e| anyhow::anyhow!("Failed to create OTLP exporter: {e}"))?;

        let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder()
            .with_resource(resource.clone())
            .with_batch_exporter(exporter)
            .build();

        let tracer = provider.tracer("platform-api");
        opentelemetry::global::set_tracer_provider(provider);

        let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);
        registry.with(otel_layer).init();

        // Initialize OTEL metrics pipeline (separate from traces)
        let metrics_exporter = opentelemetry_otlp::MetricExporter::builder()
            .with_tonic()
            .with_endpoint(&config.otel_endpoint)
            .build()
            .map_err(|e| anyhow::anyhow!("Failed to create OTLP metrics exporter: {e}"))?;

        let metrics_reader = opentelemetry_sdk::metrics::PeriodicReader::builder(metrics_exporter)
            .with_interval(std::time::Duration::from_secs(15))
            .build();

        let meter_provider = opentelemetry_sdk::metrics::SdkMeterProvider::builder()
            .with_resource(resource)
            .with_reader(metrics_reader)
            .build();

        opentelemetry::global::set_meter_provider(meter_provider);

        tracing::info!("OpenTelemetry export enabled → {}", config.otel_endpoint);
    } else {
        registry.init();
    }

    Ok(())
}

/// Listen for Ctrl+C or SIGTERM for graceful shutdown.
async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(e) = tokio::signal::ctrl_c().await {
            tracing::warn!(error = %e, "Failed to install Ctrl+C handler");
            std::future::pending::<()>().await;
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut stream) => {
                let _ = stream.recv().await;
            }
            Err(e) => {
                tracing::warn!(error = %e, "Failed to install SIGTERM handler");
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("Shutdown signal received");
}
