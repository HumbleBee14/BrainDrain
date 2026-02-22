# BrainDrain — Improvements & Enhancement Roadmap

> Gaps identified during architecture review and end-to-end testing preparation.
> Philosophy: **end users only see the website** — all infrastructure complexity is hidden behind defaults with optional customization.

---

## Guiding Principles

1. **Zero-friction onboarding** — Upload a document, get a fine-tuned model. No CLI, no config files, no terminal commands.
2. **Defaults always** — Every setting has a sensible default. Users who don't care about details never have to touch anything.
3. **Customization where it matters** — Power users can override defaults (model, provider, hyperparams, GPU tier) but never have to.
4. **Infrastructure is our problem** — Users don't provision GPUs, manage containers, or download models. The platform handles compute orchestration end-to-end.
5. **Provider agnostic** — No lock-in to any single LLM provider, GPU vendor, or cloud. Pluggable backends everywhere.

---

## 1. Per-User LLM Provider Selection

**Current state:** Synthetic data generation uses a single worker-level LLM provider configured via environment variables (`APP_LLM_API_BASE_URL`, `APP_LLM_API_KEY`, `APP_LLM_MODEL`). All users share the same provider.

**Gap:** Users cannot choose their own LLM provider for data generation. A user with an Anthropic API key cannot use Claude; a user with a Groq key cannot use Groq.

**What exists that helps:** The data generation code (`generate_pairs.py`) already uses raw HTTP calls to an OpenAI-compatible `/chat/completions` endpoint — it is NOT locked to any SDK. Any provider exposing this interface (OpenAI, Anthropic via OpenRouter, Mistral, Together, Groq, local Ollama/vLLM) already works by changing three config values.

**Enhancement:**

| Item | Detail |
|------|--------|
| **Effort** | Medium |
| **DB change** | Add `llm_provider_config` table (tenant_id, provider_name, api_base_url, encrypted_api_key, model, is_default) |
| **API change** | New endpoints: `POST /api/v1/settings/llm-providers`, `GET /api/v1/settings/llm-providers`, `DELETE /api/v1/settings/llm-providers/{id}` |
| **Workflow change** | Pass provider config into Temporal workflow input → activity reads per-tenant config instead of global env |
| **Frontend** | Settings page with provider selector (dropdown: OpenAI, Anthropic, Mistral, Together, Groq, Custom), API key input, model selector, "Test Connection" button |
| **Defaults** | Platform provides a default provider (OpenAI gpt-4o-mini). Users override only if they want to. |
| **Security** | API keys encrypted at rest. Never returned in GET responses (masked: `sk-...xxxx`). |

**Suggested providers to support out of the box:**

| Provider | Base URL | Compatible |
|----------|----------|------------|
| OpenAI | `https://api.openai.com/v1` | Native |
| Anthropic (via OpenRouter) | `https://openrouter.ai/api/v1` | Yes |
| Mistral | `https://api.mistral.ai/v1` | Yes |
| Together | `https://api.together.xyz/v1` | Yes |
| Groq | `https://api.groq.com/openai/v1` | Yes |
| Fireworks | `https://api.fireworks.ai/inference/v1` | Yes |
| Ollama (local) | `http://localhost:11434/v1` | Yes |
| Custom | User-provided URL | Yes |

---

## 2. Base Model Catalog & Selection

**Current state:** `CreateTrainingJobRequest.base_model` is a free-text string field. Users must know the exact HuggingFace model ID (e.g., `unsloth/Llama-3.1-8B-Instruct`). The UnslothEngine auto-downloads the model from HuggingFace at training time.

**Gap:** Users need to know HuggingFace model naming conventions. No browsing, no validation, no size/VRAM guidance.

**Enhancement:**

| Item | Detail |
|------|--------|
| **Effort** | Small–Medium |
| **Approach** | Curated catalog of recommended models + custom model input |
| **API change** | New endpoint: `GET /api/v1/models/catalog` returning curated list with metadata |
| **Frontend** | Dropdown with recommended models grouped by size/task, plus "Custom HuggingFace ID" text input |
| **Validation** | Optional: HEAD request to HuggingFace API to verify model exists before queueing training |

**Suggested default catalog:**

| Model | Size | VRAM Needed | Best For |
|-------|------|-------------|----------|
| `unsloth/Llama-3.1-8B-Instruct` | 8B | 16GB+ (4bit: 6GB) | General purpose |
| `unsloth/Mistral-7B-Instruct-v0.3` | 7B | 16GB+ (4bit: 6GB) | Instruction following |
| `unsloth/Phi-3.5-mini-instruct` | 3.8B | 8GB+ (4bit: 4GB) | Fast/lightweight |
| `unsloth/gemma-2-9b-it` | 9B | 20GB+ (4bit: 8GB) | Multilingual |
| `unsloth/Qwen2.5-7B-Instruct` | 7B | 16GB+ (4bit: 6GB) | Code + reasoning |

- Users always see these defaults with clear descriptions (size, speed, quality tradeoffs)
- "Advanced" toggle reveals a text input for any HuggingFace model ID
- Gated models show a note: "Requires HuggingFace token — add in Settings"

---

## 3. Automated GPU Provisioning

**Current state:** Workers run on whatever machine they're deployed to. No automated provisioning. To train a model, someone must manually set up a GPU machine, install dependencies, configure env vars, and start the worker process.

**Gap:** This is the largest UX gap. End users should never think about GPUs. They click "Train" and the platform finds compute.

**Enhancement (phased):**

### Phase A — Single-Provider Integration (RunPod)

| Item | Detail |
|------|--------|
| **Effort** | Large |
| **Approach** | Platform auto-provisions GPU pods on RunPod when a training job is queued |
| **Flow** | User clicks Train → API creates training job → Temporal workflow calls `provision_gpu` activity → RunPod API creates pod with worker Docker image → pod connects to Temporal → trains → uploads adapter → pod auto-terminates |
| **Config** | Platform-level RunPod API key. Users select GPU tier (A40, A100, H100) or "Auto" (platform picks cheapest that fits). |
| **Cost** | Pass-through billing or bundled into platform pricing |
| **New code** | `apps/workers/src/activities/provision_gpu.py` — RunPod API client, pod lifecycle management |
| **Temporal** | New activity: `provision_gpu`, `teardown_gpu`. Training workflow wraps: provision → train → teardown |

### Phase B — Multi-Provider

| Item | Detail |
|------|--------|
| **Effort** | Large |
| **Providers** | RunPod, Modal, Lambda Labs, AWS (SageMaker/EC2), GCP (Vertex AI) |
| **Approach** | `GpuProvider` Protocol with provider-specific implementations |
| **User choice** | Settings page: "GPU Provider" dropdown + API key. Or "Platform Managed" (we pick cheapest). |
| **Smart routing** | Compare pricing across providers, pick cheapest available GPU that meets VRAM requirements |

### Phase C — Kubernetes / Self-Hosted

| Item | Detail |
|------|--------|
| **Effort** | Very Large |
| **Approach** | Kubernetes Job-based GPU scheduling for self-hosted deployments |
| **Use case** | Enterprise customers with their own GPU clusters |

**Priority:** Phase A (RunPod) is sufficient for launch. The `gpu_class` field already exists in `CreateTrainingJobRequest` — it just needs to be wired to actual provisioning logic.

---

## 4. Document Source Connectors

**Current state:** Documents can only be uploaded as files via the API (`POST /api/v1/projects/{id}/documents` with multipart form).

**Gap:** Users should be able to point at existing document sources (Google Drive, Notion, Confluence, URLs, GitHub repos) without manual download-and-upload.

**Enhancement:**

| Source | Effort | Approach |
|--------|--------|----------|
| URL / Web scraping | Small | Accept URL, fetch content, parse HTML |
| Google Drive | Medium | OAuth2 integration, folder picker |
| Notion | Medium | Notion API integration, database/page selector |
| GitHub repo | Medium | GitHub App integration, repo/path selector |
| Confluence | Medium | Atlassian API integration |
| S3 bucket | Small | User provides bucket + credentials |
| File upload (current) | Done | Already implemented |

**Default:** File upload always available. Connectors added incrementally. Each connector is a separate `DocumentSource` implementation that downloads content and feeds it into the existing parsing pipeline.

---

## 5. Training Progress & Cost Estimation

**Current state:** Training metrics stream via SSE (loss, learning rate, gradient norm, GPU stats). No time estimate, no cost estimate, no "you are X% done" indicator.

**Gap:** Users want to know: How long will this take? How much will it cost? Am I 20% done or 90% done?

**Enhancement:**

| Item | Detail |
|------|--------|
| **Progress bar** | Calculate `current_step / total_steps` from trainer state, send as percentage in SSE stream |
| **ETA** | Track steps/second, extrapolate remaining time |
| **Cost estimate** | `estimated_time × gpu_hourly_rate` — show before training starts and update live |
| **Historical data** | Store training duration per (model_size, dataset_size, gpu_class) tuple. Use for future estimates. |

---

## 6. Evaluation UX Improvements

**Current state:** Evaluation runs MMLU/HellaSwag benchmarks and returns JSON scores. No explanation, no comparison, no recommendation.

**Gap:** Users don't know what an MMLU score of 0.65 means or whether their fine-tune improved the model.

**Enhancement:**

| Item | Detail |
|------|--------|
| **Score interpretation** | "Your model scores 65% on MMLU (general knowledge). This is competitive with GPT-3.5-level performance." |
| **Before/after comparison** | Run eval on base model AND fine-tuned model, show delta |
| **Domain-specific eval** | Auto-generate eval questions from the user's own data (not just generic benchmarks) |
| **Visual report** | Radar chart of capabilities, not just numbers |
| **Recommendation** | "Your model improved 12% on Q&A tasks. Consider aligned training mode for further improvement." |

---

## 7. Deployment Simplification

**Current state:** Deployment requires a running vLLM server. Users must configure `APP_VLLM_API_URL`. The platform sends a "load model" request to vLLM but doesn't provision the vLLM instance itself.

**Gap:** Same problem as GPU provisioning — users shouldn't manage inference infrastructure.

**Enhancement:**

| Item | Detail |
|------|--------|
| **Serverless inference** | Auto-provision vLLM on RunPod Serverless / Modal / Replicate when user clicks "Deploy" |
| **Scale-to-zero** | Inference endpoint scales down when not in use (cost savings) |
| **Custom domains** | `{model-name}.api.braindrain.ai` — vanity URLs for deployed models |
| **Usage dashboard** | Requests/day, latency p50/p95, token usage, cost tracking |
| **One-click Ollama** | Export as GGUF + generate `ollama run` command (partially exists via exports) |

---

## 8. Data Quality Feedback Loop

**Current state:** Generated pairs go through length-based filtering and MD5 deduplication. No semantic quality check, no user review step, no feedback mechanism.

**Gap:** Users can't review or curate their training data. Low-quality pairs reduce model quality.

**Enhancement:**

| Item | Detail |
|------|--------|
| **Data preview** | Show sample pairs before training starts (partially exists via `/datasets/{id}/preview`) |
| **Thumbs up/down** | Let users rate individual pairs. Remove downvoted pairs from training set. |
| **Semantic dedup** | Embedding-based similarity dedup (not just exact MD5 match). Requires vector store (Qdrant). |
| **Quality scoring** | LLM-as-judge to score pair quality (coherence, accuracy, relevance) |
| **Active learning** | Identify low-confidence pairs and surface them for human review |

---

## 9. Multi-Language Support

**Current state:** Language detection exists (`langdetect` in `parse_document.py`) but is only stored as metadata. No language-specific processing.

**Gap:** Non-English documents are parsed and chunked the same way. Prompt templates are English-only.

**Enhancement:**

| Item | Detail |
|------|--------|
| **Localized prompts** | Prompt templates in multiple languages for data generation |
| **Language-aware chunking** | Respect language-specific sentence boundaries |
| **Multilingual models** | Surface multilingual base models in catalog for non-English data |
| **UI language** | Frontend i18n (lower priority) |

---

## 10. Security & Compliance

**Current state:** Clerk JWT auth, tenant isolation via RLS, API key auth for inference. No file scanning, no PII detection.

**Enhancement:**

| Item | Detail |
|------|--------|
| **File scanning** | ClamAV integration for malware scanning on upload |
| **PII detection** | Scan documents for PII before training (names, emails, SSNs). Warn or auto-redact. |
| **Audit trail** | Audit log exists. Add data lineage: which document → which chunks → which pairs → which model. |
| **Data retention** | Auto-delete training data after configurable period |
| **SOC 2 prep** | Encryption at rest, access logging, data classification |

---

## Priority Matrix

| # | Enhancement | Impact | Effort | Priority |
|---|------------|--------|--------|----------|
| 3A | GPU Provisioning (RunPod) | Critical | Large | P0 — Blocks real users |
| 2 | Base Model Catalog | High | Small | P0 — Quick win |
| 1 | Per-User LLM Provider | High | Medium | P1 — Differentiator |
| 5 | Training Progress/Cost | High | Small | P1 — UX essential |
| 7 | Deployment Simplification | High | Large | P1 — End-to-end story |
| 4 | Document Connectors (URL) | Medium | Small | P2 — Start with URL |
| 8 | Data Quality Feedback | Medium | Medium | P2 — Quality improvement |
| 6 | Evaluation UX | Medium | Medium | P2 — Comprehension |
| 9 | Multi-Language | Medium | Medium | P3 — Market expansion |
| 10 | Security/Compliance | High | Large | P3 — Enterprise readiness |
| 3B | Multi-Provider GPU | Medium | Large | P3 — After RunPod works |
| 4+ | More Connectors | Medium | Medium | P3 — Incremental |

---

## Implementation Notes

- All enhancements follow the existing architecture: Route → Service → Repository
- New protocols/traits for pluggable backends (GpuProvider, DocumentSource, QualityScorer)
- Database changes require new migrations in `crates/db/src/migrations/`
- Frontend changes in `apps/web/src/app/` following existing patterns
- Worker changes in `apps/workers/src/activities/` following existing Protocol patterns
- Every new API endpoint gets OpenAPI annotations (utoipa) and TypeScript type generation (ts-rs)

---

*This is a living document. Update as gaps are identified during testing and user feedback.*
