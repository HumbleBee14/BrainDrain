from pydantic import field_validator
from pydantic_settings import BaseSettings


class WorkerSettings(BaseSettings):
    """Worker configuration loaded from environment variables."""

    # Deployment environment: "development" | "production"
    environment: str = "development"

    # Fetch-time SSRF re-validation of tenant-supplied LLM base URLs
    # (see src/url_guard.py). None = enabled only when environment=production,
    # so dev keeps working against localhost endpoints.
    url_guard_enabled: bool | None = None

    # Temporal
    temporal_address: str = "localhost:7233"
    temporal_namespace: str = "default"
    temporal_task_queue: str = "ml-pipeline"
    # Per-worker activity concurrency. GPU activities share one physical GPU,
    # so the GPU queue defaults to strictly serial execution — two concurrent
    # training runs OOM each other. 0 = Temporal's default (no explicit cap).
    max_concurrent_activities: int = 0
    gpu_max_concurrent_activities: int = 1

    # Database (required — no insecure default)
    database_url: str
    db_pool_min: int = 2
    db_pool_max: int = 10

    # Redis
    redis_url: str = "redis://localhost:6379"

    # S3 / Object storage (required — no insecure defaults for credentials)
    s3_endpoint: str = "http://localhost:9000"
    s3_access_key: str
    s3_secret_key: str
    s3_bucket: str = "platform"
    s3_region: str = "us-east-1"

    # Platform API (for internal worker → API calls like deploy)
    platform_api_url: str = "http://localhost:8000"
    platform_internal_token: str = ""

    # Decrypts tenant LLM API keys stored as enc:v1:... in tenants.settings
    # (AES-256-GCM, base64-encoded 32 bytes). Must match the API's
    # SETTINGS_ENCRYPTION_KEY. Empty + encrypted value = the activity fails loud.
    settings_encryption_key: str = ""

    # LLM API (OpenAI-compatible — works with any provider)
    llm_api_base_url: str = "https://api.openai.com/v1"
    llm_api_key: str = ""
    llm_model: str = "gpt-4o-mini"
    llm_max_tokens: int = 2000

    # Training / ML
    hf_token: str = ""
    model_cache_dir: str = "/tmp/model_cache"
    worker_mode: str = "all"  # "all" | "main" | "gpu"

    # GPU provider: "local" (default, worker's own GPU) | "modal" (serverless)
    gpu_provider: str = "local"

    # Cloud GPU (Modal serverless) — used when gpu_provider="modal".
    # One deployed function per GPU-bound activity (see apps/workers/modal_app.py).
    modal_app_name: str = "platform-training"
    # Per-function app-name overrides, e.g. {"train_on_policy": "other-app"}.
    # Lets a subset of functions ship on a separate app when the main app's
    # deploy is blocked (image rebuilds) without forking the worker config.
    modal_function_apps: dict[str, str] = {}
    modal_function_name: str = "train"
    # On-policy distillation cannot run on the training image: it needs vLLM
    # in-process to reach its teacher sidecar, and unsloth cannot resolve beside
    # vLLM. Same core, different image, therefore a different deployed function.
    modal_on_policy_function_name: str = "train_on_policy"
    modal_sft_round_function_name: str = "train_sft_round"
    modal_evaluate_holdout_function_name: str = "evaluate_holdout"
    modal_evaluation_function_name: str = "run_evaluation"
    modal_export_function_name: str = "export_gguf"
    modal_secret_name: str = "platform-training-secrets"
    modal_poll_interval_secs: int = 15
    # How often to sweep for orphaned Modal calls — remote GPU calls whose job
    # was cancelled or reaped (terminated workflow / dead worker) while the call
    # kept running and billing. Set <= 0 to disable the sweep.
    modal_orphan_sweep_interval_secs: int = 300

    # Cloud GPU (Beam serverless) — used when gpu_provider="beam".
    # Beam assigns each deployed task queue a generated invoke URL (printed by
    # `beam deploy apps/workers/beam_app.py:<name>`), so queues are configured
    # as {logical function name: invoke URL} rather than derived from a name.
    # Logical names match the queues in apps/workers/beam_app.py:
    # "train", "train_sft_round", "evaluate_holdout", "run_evaluation".
    beam_token: str = ""
    beam_workspace_id: str = ""
    beam_queue_urls: dict[str, str] = {}
    beam_task_api_base: str = "https://app.beam.cloud/api/v1/task"
    beam_poll_interval_secs: int = 15

    # Backend selection — swap any processing layer without code changes
    pdf_backend: str = "pymupdf"  # "pymupdf" | "docling"
    language_detector_backend: str = "langdetect"  # "langdetect" | "null"
    training_engine: str = "unsloth"  # "unsloth"
    metrics_backend: str = "redis"  # "redis" | "log" | "null"
    eval_model_loader: str = "unsloth"  # "unsloth"
    chunking_backend: str = "recursive"  # "recursive" | "sliding"
    llm_provider_backend: str = "openai"  # "openai" (any OpenAI-compatible)
    dataset_filter_backend: str = "heuristic"  # "heuristic"
    dedup_backend: str = "hash"  # "hash" (exact) | "near" (token-Jaccard near-dup)
    judge_backend: str = "openai"  # "openai"
    # Judge resilience: retry transient errors, then fail loudly by default so a
    # broken judge never silently poisons rewards/scores with fabricated numbers.
    judge_max_retries: int = 3
    judge_on_failure: str = "error"  # "error" (fail loud) | "heuristic" (advanced opt-in)
    datagen_facet_backend: str = "llm"  # "llm"
    datagen_pair_backend: str = "llm"  # "llm"
    datagen_refiner_backend: str = "llm"  # "llm"
    datagen_faithfulness_backend: str = "llm"  # "llm"
    # Per-call HTTP timeout for datagen LLM requests. Sized for a self-hosted
    # reasoning teacher writing a full chunk's pairs (plus its think block) in
    # one completion, which routinely exceeds the ~2 minutes a hosted-API
    # default would allow. The Temporal activity timeout still bounds the run.
    datagen_llm_timeout_seconds: float = 600.0
    # Per-call HTTP timeout for judge LLM requests, sized like the datagen one:
    # a reasoning judge thinks for tens of seconds per verdict, and a
    # scale-to-zero judge endpoint can take minutes to cold-start.
    judge_timeout_seconds: float = 600.0
    # Reasoning judges deliberate 30-60s per verdict; off turns judge-bound
    # stages (eval, DPO filtering, GRPO rewards) from hours into minutes.
    judge_enable_thinking: bool = False
    # Concurrent judge verdicts per evaluation suite. Generation stays serial
    # (one model, one GPU); only the HTTP judge calls fan out.
    eval_judge_concurrency: int = 4
    # Caps how many items each evaluation suite scores (0 = full suite). Every
    # item costs a judge round-trip, so the full ~260-item run with a reasoning
    # judge exceeds the evaluation activity timeout; a small cap turns it into
    # a minutes-long smoke check.
    eval_max_items_per_suite: int = 0

    # Data Studio synthetic data-generation
    faithfulness_gate_enabled: bool = True
    # Sampling temperatures. Generation is deliberately creative; the
    # faithfulness judge is scored near-deterministically so the same
    # (pair, source) inputs yield a stable verdict instead of drifting.
    generation_temperature: float = 0.7
    judge_temperature: float = 0.0
    # Persist each chunk's finished pairs to S3 as it completes so a mid-run
    # failure resumes from the last checkpoint instead of regenerating every
    # chunk. Disable only if the extra per-chunk writes are undesirable.
    pair_checkpoint_enabled: bool = True

    # Logging
    log_level: str = "INFO"
    log_format: str = "json"  # "json" (production) | "text" (local dev)

    # Observability (OTEL)
    otel_enabled: bool = False
    otel_endpoint: str = "http://localhost:4317"

    # Circuit breaker (LLM API resilience)
    circuit_breaker_enabled: bool = True
    circuit_breaker_fail_max: int = 5
    circuit_breaker_reset_timeout: int = 30

    # Activity timeouts — override for slower hardware or very large models
    # Set via APP_TIMEOUT_* env vars
    timeout_parse_minutes: int = 10  # per-document parse
    timeout_chunk_minutes: int = 10  # text chunking
    timeout_generate_pairs_minutes: int = 30  # synthetic pair generation
    timeout_build_dataset_minutes: int = 15  # dataset assembly
    timeout_train_hours: int = 6  # single training run (SFT/DPO/GRPO)
    timeout_teacher_extraction_hours: int = 6  # one teacher scoring pass over a dataset
    timeout_train_iterative_hours: int = 4  # one round of iterative training
    timeout_holdout_eval_hours: int = 1  # holdout validation during training
    timeout_eval_hours: int = 1  # full evaluation suite
    timeout_export_hours: int = 2  # GGUF export + quantize
    timeout_datagen_interactive_minutes: int = 5  # Data Studio facet/preview/refine (LLM-backed)

    # Billing
    min_billable_seconds: int = 300  # 5 min — failed jobs shorter than this get voided

    model_config = {"env_prefix": "APP_", "env_file": ".env"}

    @field_validator("temporal_address")
    @classmethod
    def temporal_address_not_empty(cls, v: str) -> str:
        if not v.strip():
            raise ValueError("temporal_address must not be empty")
        return v

    @field_validator("database_url")
    @classmethod
    def database_url_must_be_postgresql(cls, v: str) -> str:
        if not v.startswith("postgresql://"):
            raise ValueError("database_url must start with 'postgresql://'")
        return v

    @field_validator("s3_endpoint")
    @classmethod
    def s3_endpoint_must_be_http(cls, v: str) -> str:
        if not v.startswith("http://") and not v.startswith("https://"):
            raise ValueError("s3_endpoint must start with 'http://' or 'https://'")
        return v

    @field_validator("llm_api_base_url")
    @classmethod
    def llm_api_base_url_must_be_http(cls, v: str) -> str:
        if not v.startswith("http://") and not v.startswith("https://"):
            raise ValueError("llm_api_base_url must start with 'http://' or 'https://'")
        return v

    @field_validator("s3_bucket")
    @classmethod
    def s3_bucket_not_empty(cls, v: str) -> str:
        if not v.strip():
            raise ValueError("s3_bucket must not be empty")
        return v
