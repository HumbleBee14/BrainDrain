# Platform — System Architecture & Learning Notes (February 2026)

> **This is the original, aspirational design document written at project
> inception (February 2026).** It describes the intended architecture, not
> necessarily what was built. Notable divergences: parsing shipped with
> PyMuPDF/Docling (no MinerU, no Nougat, no OCR for scanned PDFs); synthetic
> data generation uses raw calls to an OpenAI-compatible endpoint (no
> distilabel); cloud GPU training uses Modal, validated for deploy/smoke
> only (no RunPod, no full train→S3 cloud proof yet); serving/CD are
> implemented but not proven end-to-end. For the as-built system, see
> [SYSTEM_ARCHITECTURE.md](./SYSTEM_ARCHITECTURE.md),
> [PROJECT_FLOW.md](./PROJECT_FLOW.md), [DATA_PIPELINE.md](./DATA_PIPELINE.md),
> and [CLOUD_GPU_TRAINING.md](./CLOUD_GPU_TRAINING.md).

## Table of Contents

1. [System Overview](#1-system-overview)
2. [Architecture Principles](#2-architecture-principles)
3. [High-Level Architecture](#3-high-level-architecture)
4. [Layer 1: Ingestion Engine](#4-layer-1-ingestion-engine)
5. [Layer 2: Data Refinery](#5-layer-2-data-refinery)
6. [Layer 3: Training Core](#6-layer-3-training-core)
7. [Layer 4: Evaluator Arena](#7-layer-4-evaluator-arena)
8. [Layer 5: Serving & Deployment](#8-layer-5-serving--deployment)
9. [Layer 6: Orchestration & Control Plane](#9-layer-6-orchestration--control-plane)
10. [Layer 7: API Gateway & User Interface](#10-layer-7-api-gateway--user-interface)
11. [Data Flow Diagrams](#11-data-flow-diagrams)
12. [Storage Architecture](#12-storage-architecture)
13. [Multi-Tenancy & Isolation](#13-multi-tenancy--isolation)
14. [Performance Metrics & SLAs](#14-performance-metrics--slas)
15. [Cost Model](#15-cost-model)
16. [Security Architecture](#16-security-architecture)
17. [Phased Build Roadmap](#17-phased-build-roadmap)
18. [Learning Map](#18-learning-map)
19. [Technology Decision Records](#19-technology-decision-records)

---

## 1. System Overview

Platform is a personal learning project that explores the full pipeline of transforming raw documents into deployed, fine-tuned LLMs. The goal is to learn production Rust, ML training pipelines, and systems engineering by building every stage end-to-end.

### Current Production Snapshot

The current codebase is no longer just a conceptual architecture. The implemented production shape is:
- Rust control plane with auth middleware, idempotency, durable billing outbox, feature flags, and inference routing
- Python Temporal workers for parsing, synthesis, training, evaluation, and export
- Next.js frontend
- PostgreSQL + PgBouncer + Redis + S3/MinIO + Temporal
- pluggable inference backends (`vllm`, `tgi`, `sglang`)
- multi-instance inference control plane with instance registry, health checks, draining, and per-deployment instance binding

This document keeps some longer-term notes and learning context, but the current operational system is best summarized by:
- [SYSTEM_ARCHITECTURE.md](./SYSTEM_ARCHITECTURE.md)
- [DEPLOYMENT.md](./DEPLOYMENT.md)
- [PRODUCTION_OPS.md](./PRODUCTION_OPS.md)

### The Goal

```
User uploads documents → Answers 3-5 questions → Gets a trained, deployed model
```

### What Happens Under the Hood

```
┌─────────────────────────────────────────────────────────────────────────┐
│                        USER INTERACTION LAYER                          │
│   Upload docs  →  Answer questions  →  Review samples  →  Use model   │
└────────┬───────────────┬──────────────────┬──────────────────┬─────────┘
         │               │                  │                  │
         ▼               ▼                  ▼                  ▼
┌──────────────┐ ┌──────────────┐ ┌──────────────────┐ ┌─────────────┐
│  INGESTION   │ │    DATA      │ │    TRAINING      │ │   SERVING   │
│   ENGINE     │ │  REFINERY    │ │     CORE         │ │  & DEPLOY   │
│              │ │              │ │                  │ │             │
│ Parse docs   │ │ Chunk        │ │ SFT (LoRA)      │ │ vLLM        │
│ Detect type  │ │ Synthesize   │ │ DPO alignment   │ │ S-LoRA      │
│ Extract text │ │ Filter       │ │ GRPO reasoning  │ │ GGUF export │
│ Structure    │ │ Deduplicate  │ │ Auto-config     │ │ API         │
└──────┬───────┘ └──────┬───────┘ └────────┬─────────┘ └──────┬──────┘
       │                │                  │                   │
       └────────────────┴──────────────────┴───────────────────┘
                                   │
                    ┌──────────────┴──────────────┐
                    │    ORCHESTRATION LAYER      │
                    │    Temporal.io              │
                    │    Event-driven pipelines   │
                    │    Crash recovery           │
                    └─────────────────────────────┘
```

---

## 2. Architecture Principles

| # | Principle | Implication |
|---|-----------|-------------|
| 1 | **Modularity** | Every layer is independently deployable, testable, replaceable |
| 2 | **Event-Driven** | Layers communicate via events/messages, not direct calls |
| 3 | **GPU-Ephemeral** | GPUs are rented per-job, never always-on (except shared inference) |
| 4 | **Data-First** | Data quality is the product. Training is just the execution |
| 5 | **Multi-Tenant by Default** | Every component supports user isolation from day one |
| 6 | **Fail-Forward** | Every pipeline step saves checkpoints. Crashes resume, not restart |
| 7 | **Observable** | Every operation is logged, metered, and traceable |
| 8 | **Cost-Transparent** | Users see estimated cost before every operation |

---

## 3. High-Level Architecture

### Complete System Diagram

```
                            ┌─────────────────────────────┐
                            │      LOAD BALANCER          │
                            │      (Cloudflare / Nginx)   │
                            └─────────────┬───────────────┘
                                          │
                            ┌─────────────▼───────────────┐
                            │      API GATEWAY            │
                            │      Rust (Axum) + Auth      │
                            │      Rate Limiting           │
                            │      Request Routing         │
                            └─────────────┬───────────────┘
                                          │
                 ┌────────────────────────┼────────────────────────┐
                 │                        │                        │
    ┌────────────▼─────────┐  ┌──────────▼──────────┐  ┌─────────▼──────────┐
    │   PROJECT SERVICE    │  │  PIPELINE SERVICE   │  │  INFERENCE SERVICE  │
    │                      │  │                      │  │                     │
    │ - Create projects    │  │ - Trigger pipelines  │  │ - Chat/completions  │
    │ - Upload documents   │  │ - Monitor status     │  │ - Model playground  │
    │ - Manage datasets    │  │ - View logs          │  │ - API keys          │
    │ - Configure training │  │ - Cancel/retry       │  │ - Usage metering    │
    └──────────┬───────────┘  └──────────┬───────────┘  └─────────┬──────────┘
               │                         │                        │
               ▼                         ▼                        ▼
    ┌─────────────────────────────────────────────────────────────────────┐
    │                        MESSAGE BUS                                  │
    │                     (Redis Streams / NATS)                          │
    └──┬──────────┬──────────┬──────────┬──────────┬──────────┬──────────┘
       │          │          │          │          │          │
       ▼          ▼          ▼          ▼          ▼          ▼
   ┌────────┐┌────────┐┌─────────┐┌────────┐┌────────┐┌──────────┐
   │INGEST  ││CHUNK   ││SYNTH    ││QUALITY ││TRAIN   ││EVALUATE  │
   │WORKER  ││WORKER  ││WORKER   ││WORKER  ││WORKER  ││WORKER    │
   │        ││        ││         ││        ││        ││          │
   │MinerU  ││Struct  ││Agent    ││LLM     ││Unsloth ││LLM-Judge │
   │Docling ││Semantic││Instruct ││Judge   ││TRL     ││Benchmark │
   │Nougat  ││        ││distilabel│Dedup  ││Modal   ││A/B Test  │
   └───┬────┘└───┬────┘└────┬────┘└───┬────┘└───┬────┘└────┬─────┘
       │         │          │         │         │          │
       ▼         ▼          ▼         ▼         ▼          ▼
    ┌─────────────────────────────────────────────────────────────┐
    │                     STORAGE LAYER                           │
    │                                                             │
    │  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌───────────┐  │
    │  │  S3/R2   │  │ Postgres │  │ Qdrant   │  │  Redis    │  │
    │  │ (Objects)│  │(Metadata)│  │(Vectors) │  │ (Cache)   │  │
    │  └──────────┘  └──────────┘  └──────────┘  └───────────┘  │
    └─────────────────────────────────────────────────────────────┘
```

### Language Decision: Performance-First Architecture

We use **Rust for all infrastructure** and **Python only where ML libraries force it**. When scaling to 100s of instances, cold starts, memory footprint, and throughput compound — Rust wins everywhere it's applicable.

#### Language Comparison (Why Rust Over Python/Go for Infrastructure)

| Metric | Rust (Tokio + Axum) | Go (net/http) | Python (FastAPI) |
|--------|---------------------|---------------|------------------|
| **HTTP throughput** | #1 in TechEmpower benchmarks | ~30-50% behind Rust | ~10-20x behind Rust |
| **Memory per instance** | ~5-15 MB baseline | ~25-50 MB baseline | ~80-200 MB baseline |
| **Cold start time** | ~2-5 ms | ~10-20 ms | ~500-2000 ms |
| **Concurrency model** | Zero-cost futures, millions of tasks | Goroutines, good but higher mem/task | asyncio, GIL limits true parallelism |
| **S3/DB/Redis clients** | Mature async: `aws-sdk-rust`, `sqlx`, `redis-rs` | Mature | Mature |
| **Type safety** | Compile-time, zero runtime errors | Good (static types) | Runtime only (Pydantic helps) |
| **Binary deployment** | Single static binary, no runtime | Single binary, no runtime | Requires Python runtime + venv |
| **100 instances cost** | ~1.5 GB total memory | ~5 GB total memory | ~20 GB total memory |

**Verdict:** Go's only advantage is faster development speed. No unique capability Rust lacks. For a system that could scale to hundreds of instances, Rust's memory and cold start advantages compound into real savings. Plus, learning Rust deeply is a primary goal of this project.

#### Language Split Across System

| Component | Language | Rationale |
|-----------|----------|-----------|
| API Gateway | **Rust (Axum)** | Fastest HTTP, minimal memory, instant cold starts |
| File upload/streaming | **Rust** | Zero-copy streaming to S3, backpressure |
| Database layer | **Rust (SQLx)** | Compile-time checked SQL, async, connection pooling |
| Redis/cache | **Rust (redis-rs)** | Async, connection pooling |
| S3/storage client | **Rust (aws-sdk-rust)** | Official AWS SDK, streaming |
| Auth (Clerk JWT) | **Rust (jsonwebtoken)** | JWT verification, JWKS |
| Message bus | **Rust** | Redis Streams pub/sub |
| ML Training | **Python (Unsloth/TRL)** | ML ecosystem is Python-only |
| Synthetic data gen | **Python (distilabel)** | ML pipeline, LLM API calls |
| Document parsing | **Python (MinerU/Docling)** | Parsers are Python-native |
| Temporal ML workers | **Python (temporalio)** | Workers run ML code, must be Python |
| Frontend | **TypeScript (Next.js)** | React ecosystem |

### Component Registry

| Component | Technology | Why This Choice |
|-----------|-----------|-----------------|
| API Gateway | **Rust (Axum)** | #1 performance, zero-cost async, single-binary deployment |
| Message Bus | Redis Streams (MVP) → NATS JetStream (scale) | Redis already needed for cache; NATS for scale |
| Orchestration | Temporal.io | Durable execution, built for long-running workflows |
| Object Storage | Cloudflare R2 (MVP) → S3 (scale) | Zero egress fees; S3 API compatible |
| Metadata DB | PostgreSQL 16 + SQLx | Compile-time checked queries, JSONB flexibility |
| Vector DB | Qdrant | Rust-native, filtering, multi-tenancy support |
| Cache | Redis (redis-rs) | Sub-ms latency, async Rust client |
| GPU Compute | Modal (MVP) → RunPod (scale) | Modal: best DX; RunPod: cheapest at scale |
| Serving | Pluggable inference backend + instance-aware routing | `vllm`, `tgi`, `sglang`, OpenAI-compatible inference, multi-instance control plane |

---

## 4. Layer 1: Ingestion Engine

### Purpose
Transform any document format into clean, structured text with preserved layout information.

### Architecture

```
                    ┌───────────────────────────────────┐
                    │         UPLOAD ENDPOINT            │
                    │  Accept: PDF, DOCX, TXT, HTML,    │
                    │  EPUB, Markdown, CSV, Images       │
                    │  Max: 500MB per file, 10GB batch   │
                    └───────────────┬───────────────────┘
                                    │
                                    ▼
                    ┌───────────────────────────────────┐
                    │       DOCUMENT CLASSIFIER          │
                    │                                    │
                    │  1. File type detection (magic)    │
                    │  2. Language detection (lingua-py) │
                    │  3. Domain classification          │
                    │     (academic/legal/technical/     │
                    │      business/general)             │
                    │  4. Complexity scoring             │
                    │     (tables? math? scanned?)       │
                    └───────────────┬───────────────────┘
                                    │
                     ┌──────────────┼──────────────┐
                     │              │              │
                     ▼              ▼              ▼
              ┌────────────┐┌────────────┐┌────────────┐
              │  MinerU    ││  Docling   ││  Nougat    │
              │  2.5       ││  (IBM)     ││  (Meta)    │
              │            ││            ││            │
              │ Default    ││ CPU-only   ││ Academic   │
              │ Best acc.  ││ fallback   ││ Math/sci   │
              │ GPU req.   ││ No GPU     ││ LaTeX      │
              └─────┬──────┘└─────┬──────┘└─────┬──────┘
                    │             │              │
                    └──────┬──────┘──────────────┘
                           ▼
              ┌───────────────────────────────────┐
              │     PARSE QUALITY CHECKER          │
              │                                    │
              │ - Table extraction verification    │
              │ - Math/LaTeX rendering check       │
              │ - Character encoding validation    │
              │ - Layout structure integrity       │
              │ - Completeness score (0-100)       │
              │                                    │
              │ If score < 70: try alternate       │
              │ parser and pick best result        │
              └───────────────┬───────────────────┘
                              │
                              ▼
              ┌───────────────────────────────────┐
              │     STRUCTURED OUTPUT              │
              │                                    │
              │  {                                 │
              │    "doc_id": "uuid",               │
              │    "pages": [...],                 │
              │    "sections": [                   │
              │      {                             │
              │        "type": "heading|para|      │
              │                table|code|math",   │
              │        "content": "...",           │
              │        "metadata": {               │
              │          "page": 3,                │
              │          "bbox": [x,y,w,h],        │
              │          "confidence": 0.94        │
              │        }                           │
              │      }                             │
              │    ],                              │
              │    "tables": [...],                │
              │    "images": [...],                │
              │    "parse_quality": 87             │
              │  }                                 │
              └───────────────────────────────────┘
```

### Tech Stack

| Component | Technology | Notes |
|-----------|-----------|-------|
| Upload handling | `python-multipart` + `aiofiles` | Streaming upload, chunked for large files |
| File detection | `python-magic` + custom heuristics | MIME type + content analysis |
| Language detection | `lingua-py` | 75 languages, no GPU needed |
| Primary parser | MinerU 2.5 | Requires GPU (A10G minimum for speed) |
| CPU fallback | Docling | No GPU, ~1.3 sec/page on modern CPU |
| Academic parser | Nougat | Best for academic PDFs with math/equations |
| Virus scanning | ClamAV | Scan all uploads before processing |
| Storage | Cloudflare R2 / S3 | Raw files + parsed output |

### Parser Selection Logic

```python
def select_parser(doc_metadata):
    if doc_metadata.domain == "academic" and doc_metadata.has_math:
        return [Nougat, MinerU]  # try Nougat first, MinerU fallback

    if not gpu_available():
        return [Docling]  # CPU-only fallback

    if doc_metadata.is_scanned:
        return [MinerU]  # best OCR pipeline

    return [MinerU, Docling]  # MinerU primary, Docling verify
```

### Performance Targets

| Metric | Target | Notes |
|--------|--------|-------|
| Throughput | 100 pages/min (GPU), 20 pages/min (CPU) | Per worker instance |
| Parse accuracy | >85% on OmniDocBench | Measured via automated quality checker |
| Table extraction | >90% structural accuracy | Validated against ground truth samples |
| Upload-to-parsed latency | <5 min for 100 pages | Including queue time |
| Supported formats | PDF, DOCX, TXT, HTML, EPUB, MD, CSV, images | Phase 1 |

---

## 5. Layer 2: Data Refinery

### Purpose
Transform parsed documents into high-quality training datasets through chunking, synthetic data generation, quality filtering, and deduplication.

### Architecture

```
┌─────────────────────────────────────────────────────────────────────────┐
│                           DATA REFINERY                                │
│                                                                         │
│  ┌──────────┐   ┌──────────────┐   ┌─────────────┐   ┌─────────────┐  │
│  │ CHUNKER  │──▶│  SYNTHESIZER │──▶│   QUALITY   │──▶│  FORMATTER  │  │
│  │          │   │              │   │   GATE      │   │             │  │
│  └──────────┘   └──────────────┘   └─────────────┘   └─────────────┘  │
│                                                                         │
│  ┌────────────────────────────────────────────────────────────────────┐ │
│  │                    USER FEEDBACK LOOP                              │ │
│  │  Review samples → Accept/Reject/Edit → Retrain quality filter     │ │
│  └────────────────────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────────────────┘
```

### 5.1 Chunking Engine

```
Parsed Document
       │
       ▼
┌──────────────────────────────┐
│    CHUNKING STRATEGY ROUTER  │
│                              │
│  Document type → Strategy    │
│  ─────────────────────────   │
│  Legal docs  → Section-based │
│  Academic    → Section-based │
│  Technical   → Section-based │
│  Unstructured → Semantic     │
│  Simple text → Recursive     │
└──────────────┬───────────────┘
               │
       ┌───────┼───────┐
       ▼       ▼       ▼
┌──────────┐┌──────┐┌──────────┐
│Structure ││Seman-││Recursive │
│-Aware    ││tic   ││Character │
│          ││      ││          │
│ Uses doc ││Embed ││Split by  │
│ headings,││sente-││paragraph │
│ sections,││nces, ││then sent-│
│ tables   ││merge ││ence      │
│ as bound-││by    ││          │
│ aries    ││simil-││          │
│          ││arity ││          │
└────┬─────┘└──┬───┘└────┬─────┘
     │         │         │
     └─────────┼─────────┘
               ▼
┌──────────────────────────────┐
│   CHUNK METADATA ENRICHMENT  │
│                              │
│  - Source document reference │
│  - Page numbers & positions  │
│  - Section hierarchy path    │
│  - Topic/domain tags         │
│  - Token count               │
│  - Overlap with neighbors    │
└──────────────┬───────────────┘
               │
               ▼
        Chunked Corpus
    (1,500-2,000 tokens each)
```

**Tech Stack:** Custom Python chunking library built on `tiktoken` (tokenizer), `sentence-transformers` (semantic similarity), MinerU/Docling section metadata.

### 5.2 Synthesis Engine (The Hardest Part)

This is the most complex component — and the most interesting to build. It transforms document chunks into training data.

```
┌──────────────────────────────────────────────────────────────────────────┐
│                      SYNTHESIS ENGINE                                    │
│                                                                          │
│  ┌──────────────────────────────────────────────────────────────┐       │
│  │                 TASK TYPE ROUTER                              │       │
│  │                                                               │       │
│  │  User answers: "What should the model do?"                   │       │
│  │                                                               │       │
│  │  ┌─────────────┐ ┌───────────────┐ ┌──────────────────────┐  │       │
│  │  │ Q&A /       │ │ Instruction   │ │ Reasoning /          │  │       │
│  │  │ Knowledge   │ │ Following     │ │ Analysis             │  │       │
│  │  │             │ │               │ │                      │  │       │
│  │  │"Answer      │ │"Write like    │ │"Analyze and          │  │       │
│  │  │ questions   │ │ this, follow  │ │ reason about         │  │       │
│  │  │ about my    │ │ these rules"  │ │ complex problems"    │  │       │
│  │  │ domain"     │ │               │ │                      │  │       │
│  │  └──────┬──────┘ └──────┬────────┘ └──────────┬───────────┘  │       │
│  │         │               │                     │               │       │
│  └─────────┼───────────────┼─────────────────────┼───────────────┘       │
│            │               │                     │                       │
│            ▼               ▼                     ▼                       │
│  ┌──────────────────────────────────────────────────────────────┐       │
│  │              AGENT PIPELINE (distilabel)                      │       │
│  │                                                               │       │
│  │  Agent 1: CONTENT ANALYZER                                   │       │
│  │  ├─ Reads chunk, identifies key concepts, facts, procedures  │       │
│  │  ├─ Classifies complexity level                              │       │
│  │  └─ Outputs: concept list, complexity score                  │       │
│  │                                                               │       │
│  │  Agent 2: QUESTION GENERATOR                                 │       │
│  │  ├─ Generates diverse questions per task type                │       │
│  │  ├─ Evol-Instruct: creates simple → complex variants        │       │
│  │  ├─ Covers: factual, inferential, comparative, procedural   │       │
│  │  └─ Outputs: 5-15 questions per chunk                       │       │
│  │                                                               │       │
│  │  Agent 3: ANSWER GENERATOR                                   │       │
│  │  ├─ Generates detailed answers grounded in source chunk      │       │
│  │  ├─ Includes reasoning traces (Orca-style) if reasoning task│       │
│  │  └─ Outputs: (question, answer, source_span) triples        │       │
│  │                                                               │       │
│  │  Agent 4: VERIFIER                                           │       │
│  │  ├─ Checks answers against source document                  │       │
│  │  ├─ Flags hallucinated content                              │       │
│  │  ├─ Validates source grounding (LangExtract-style)          │       │
│  │  └─ Outputs: verified pairs with confidence scores          │       │
│  │                                                               │       │
│  └──────────────────────────────────────────────────────────────┘       │
│                              │                                           │
│                              ▼                                           │
│  ┌──────────────────────────────────────────────────────────────┐       │
│  │              RAW SYNTHETIC DATASET                            │       │
│  │                                                               │       │
│  │  Format: ChatML/JSONL                                        │       │
│  │  {                                                            │       │
│  │    "messages": [                                              │       │
│  │      {"role": "system", "content": "<user's system prompt>"} │       │
│  │      {"role": "user", "content": "<generated question>"},    │       │
│  │      {"role": "assistant", "content": "<generated answer>"}  │       │
│  │    ],                                                         │       │
│  │    "metadata": {                                              │       │
│  │      "source_doc": "doc_id",                                 │       │
│  │      "source_chunk": "chunk_id",                             │       │
│  │      "source_span": [start, end],                            │       │
│  │      "confidence": 0.92,                                     │       │
│  │      "task_type": "qa",                                      │       │
│  │      "complexity": "intermediate"                            │       │
│  │    }                                                          │       │
│  │  }                                                            │       │
│  └──────────────────────────────────────────────────────────────┘       │
└──────────────────────────────────────────────────────────────────────────┘
```

**Tech Stack:**

| Component | Technology | Notes |
|-----------|-----------|-------|
| Agent orchestration | `distilabel` (Argilla/HF) | Production pipeline framework |
| LLM backend (synthesis) | Claude Sonnet / GPT-4o-mini / Qwen-72B | Cost-optimized per quality tier |
| LLM backend (verification) | Same as above | Can use smaller model for verification |
| Prompt templates | Jinja2 templates, version-controlled | Per task type, per domain |
| Evol-Instruct | distilabel built-in | Complexity evolution |
| Source grounding | Custom + LangExtract patterns | Character-level source verification |

**LLM Cost Optimization:**

```
Tier 1 (Quality):   Claude Sonnet / GPT-4o    → $3-15/M tokens  → Complex domains
Tier 2 (Balanced):  GPT-4o-mini / Qwen-72B    → $0.15-0.60/M   → Standard use
Tier 3 (Volume):    Llama-3-70B / Qwen-32B    → Self-hosted     → High volume
```

Users pick quality tier (or auto-select based on domain complexity). Default: Tier 2.

### 5.3 Quality Gate

```
Raw Synthetic Dataset
        │
        ▼
┌───────────────────────────────────────────┐
│             QUALITY GATE                   │
│                                            │
│  Stage 1: DEDUPLICATION                   │
│  ├─ Exact hash (MD5)        → remove 100% │
│  ├─ MinHash + LSH           → remove ~95% │
│  └─ Semantic dedup (cosine) → remove ~80% │
│                                            │
│  Stage 2: QUALITY SCORING                 │
│  ├─ LLM-as-Judge (1-5 score)             │
│  │   Criteria:                             │
│  │   - Instruction clarity                │
│  │   - Response accuracy                  │
│  │   - Source faithfulness                │
│  │   - Completeness                       │
│  │   - Language quality                   │
│  ├─ Perplexity filter                     │
│  │   (remove if perplexity > threshold)   │
│  └─ IFD score (instructional difficulty)  │
│                                            │
│  Stage 3: DIVERSITY CHECK                 │
│  ├─ Cluster embedding space              │
│  ├─ Ensure coverage across topics        │
│  └─ Flag over-represented clusters       │
│                                            │
│  Stage 4: SOURCE GROUNDING               │
│  ├─ Verify answer traces to source doc   │
│  ├─ Flag hallucinated claims             │
│  └─ Compute grounding score (0-1)        │
│                                            │
│  OUTPUT: Scored, filtered, deduplicated   │
│  dataset with quality metadata            │
│                                            │
│  Default thresholds:                      │
│  - LLM-Judge score >= 3.5/5              │
│  - Grounding score >= 0.7                │
│  - Perplexity within 2σ of mean          │
└───────────────────┬───────────────────────┘
                    │
                    ▼
        ┌───────────────────────┐
        │  USER REVIEW SAMPLE   │
        │                       │
        │  Show 10-20 examples  │
        │  User: Accept/Reject  │
        │  Adjust thresholds    │
        │  if needed            │
        └───────────────────────┘
```

### 5.4 Anti-Forgetting Mixer

Before sending data to training, automatically mix domain data with general instruction data to prevent catastrophic forgetting.

```
┌─────────────────────────────────────────┐
│         ANTI-FORGETTING MIXER           │
│                                          │
│  Domain Data (user's)        70-80%     │
│  General Instruction Data    20-30%     │
│  (curated open-source subset)           │
│                                          │
│  Mixing ratio auto-adjusted based on:   │
│  - Dataset size (smaller = more mixing) │
│  - Domain specificity (niche = more)    │
│  - Model size (larger = more mixing)    │
│                                          │
│  General data sources:                  │
│  - OpenHermes 2.5 (subset)             │
│  - SlimOrca (subset)                   │
│  - Curated from open-instruct          │
└─────────────────────────────────────────┘
```

### Performance Targets

| Metric | Target |
|--------|--------|
| Synthesis throughput | 500-1,000 pairs/hour (Tier 2 LLM) |
| Quality gate pass rate | 60-80% of raw synthetic data |
| Dedup reduction | 10-30% of pairs removed |
| Grounding accuracy | >85% of pairs trace to source |
| Time for 10K examples | 10-20 hours (Tier 2) |
| Time for 1K examples | 1-2 hours (Tier 2) |

---

## 6. Layer 3: Training Core

### Purpose
Execute model fine-tuning jobs with automatic configuration, monitoring, and checkpointing.

### Architecture

```
┌──────────────────────────────────────────────────────────────────────────┐
│                         TRAINING CORE                                    │
│                                                                          │
│  ┌──────────────────────────────────────────────────────────┐           │
│  │                   AUTO-CONFIGURATOR                       │           │
│  │                                                           │           │
│  │  Inputs:                                                 │           │
│  │  - Model family + size (e.g., Llama-3-8B)               │           │
│  │  - Dataset size (e.g., 10K examples)                     │           │
│  │  - Task type (QA / instruction / reasoning)              │           │
│  │  - User quality preference (fast/balanced/best)          │           │
│  │                                                           │           │
│  │  Auto-selects:                                           │           │
│  │  ┌─────────────────────────────────────────────────────┐ │           │
│  │  │ Hyperparameter    │ Logic                           │ │           │
│  │  │───────────────────│─────────────────────────────────│ │           │
│  │  │ Method            │ QLoRA (default), LoRA if >48GB  │ │           │
│  │  │ LoRA rank         │ 16 (small data), 64 (large)    │ │           │
│  │  │ LoRA alpha        │ 2x rank                        │ │           │
│  │  │ Target modules    │ All linear (default for 2025+)  │ │           │
│  │  │ Learning rate     │ 2e-4 (7B), 1e-4 (13B+)        │ │           │
│  │  │ Batch size        │ Max that fits VRAM             │ │           │
│  │  │ Grad accumulation │ Effective batch 32-128         │ │           │
│  │  │ Epochs            │ 3 (small data), 1 (large data) │ │           │
│  │  │ Warmup ratio      │ 0.03-0.1                       │ │           │
│  │  │ Scheduler         │ Cosine                         │ │           │
│  │  │ Max seq length    │ From dataset analysis          │ │           │
│  │  │ GPU class         │ From model size (see table)    │ │           │
│  │  └─────────────────────────────────────────────────────┘ │           │
│  └──────────────────────────────────────────────────────────┘           │
│                              │                                           │
│                              ▼                                           │
│  ┌──────────────────────────────────────────────────────────┐           │
│  │                  GPU PROVISIONER                          │           │
│  │                                                           │           │
│  │  Phase 1 (MVP):  Modal.com                               │           │
│  │  ├─ @modal.function(gpu="A10G")   for 7B               │           │
│  │  ├─ @modal.function(gpu="A100")   for 13B-70B          │           │
│  │  └─ Per-second billing, auto-scaling                    │           │
│  │                                                           │           │
│  │  Phase 2 (Scale): RunPod + Custom K8s                   │           │
│  │  ├─ RunPod Pods for sustained training                  │           │
│  │  ├─ K8s GPU Operator for multi-node                     │           │
│  │  └─ Spot instances with checkpoint resume               │           │
│  └──────────────────────────────────────────────────────────┘           │
│                              │                                           │
│                              ▼                                           │
│  ┌──────────────────────────────────────────────────────────┐           │
│  │                  TRAINING EXECUTOR                        │           │
│  │                                                           │           │
│  │  Engine: Unsloth (primary) + TRL (fallback)              │           │
│  │                                                           │           │
│  │  Training Pipeline:                                      │           │
│  │  ┌─────────┐   ┌─────────┐   ┌─────────┐   ┌────────┐  │           │
│  │  │ Dataset │──▶│  SFT    │──▶│  DPO    │──▶│ Merge  │  │           │
│  │  │ Load    │   │ Train   │   │(optional│   │ & Save │  │           │
│  │  │         │   │         │   │ align)  │   │        │  │           │
│  │  └─────────┘   └─────────┘   └─────────┘   └────────┘  │           │
│  │                                                           │           │
│  │  Features:                                               │           │
│  │  - Checkpoint every N steps (S3/R2)                     │           │
│  │  - Early stopping on eval loss plateau                  │           │
│  │  - Gradient checkpointing (save VRAM)                   │           │
│  │  - Flash Attention 2 (automatic if supported)           │           │
│  │  - BF16/FP16 mixed precision                            │           │
│  │  - W&B / MLflow logging (metrics + artifacts)           │           │
│  └──────────────────────────────────────────────────────────┘           │
│                              │                                           │
│                              ▼                                           │
│  ┌──────────────────────────────────────────────────────────┐           │
│  │                  TRAINING MONITOR                         │           │
│  │                                                           │           │
│  │  Real-time metrics streamed to user:                     │           │
│  │  - Training loss curve                                   │           │
│  │  - Eval loss curve                                       │           │
│  │  - Learning rate schedule                                │           │
│  │  - GPU utilization & VRAM usage                          │           │
│  │  - Estimated time remaining                              │           │
│  │  - Cost accumulator                                      │           │
│  │                                                           │           │
│  │  Anomaly detection:                                      │           │
│  │  - Loss spike > 3σ → alert user                         │           │
│  │  - Loss plateau → suggest early stopping                │           │
│  │  - NaN/Inf → auto-restart with lower LR                 │           │
│  │  - OOM → auto-reduce batch size, restart                │           │
│  └──────────────────────────────────────────────────────────┘           │
└──────────────────────────────────────────────────────────────────────────┘
```

### GPU Selection Matrix

| Model Size | Method | GPU | VRAM Used | Cost/hr | Est. Time (10K examples) |
|-----------|--------|-----|-----------|---------|--------------------------|
| 1-3B | QLoRA | T4 16GB | ~8GB | $0.59 | 30-60 min |
| 7-8B | QLoRA | A10G 24GB | ~14GB | $1.10 | 1-3 hrs |
| 13-14B | QLoRA | A100 40GB | ~28GB | $2.50 | 3-6 hrs |
| 30-34B | QLoRA | A100 80GB | ~45GB | $2.50 | 6-12 hrs |
| 70B | QLoRA | A100 80GB | ~48GB | $2.50 | 12-24 hrs |

### Training Modes

```
┌─────────────────────────────────────────────────────────────────┐
│                    TRAINING MODES                                │
│                                                                  │
│  MODE 1: QUICK TRAIN (SFT only)                                │
│  ├─ One-click, fastest                                          │
│  ├─ Auto hyperparameters                                        │
│  ├─ Best for: first iteration, testing                          │
│  └─ Cost: $1-5 (7B)                                            │
│                                                                  │
│  MODE 2: ALIGNED TRAIN (SFT → DPO)                             │
│  ├─ Two-phase training                                          │
│  ├─ Requires preference data (from user feedback or generated)  │
│  ├─ Best for: production models                                 │
│  └─ Cost: $2-10 (7B)                                           │
│                                                                  │
│  MODE 3: REASONING TRAIN (SFT → GRPO)                          │
│  ├─ Optimized for reasoning tasks                               │
│  ├─ Uses verifiable rewards                                     │
│  ├─ Best for: analysis, coding, math                            │
│  └─ Cost: $3-15 (7B)                                           │
│                                                                  │
│  MODE 4: ITERATIVE TRAIN (Active Learning Loop)                 │
│  ├─ Train → Evaluate → Find weaknesses → Generate more data    │
│  ├─ Multiple iterations until quality target met                │
│  ├─ Best for: maximum quality                                   │
│  └─ Cost: $5-50 (7B, 2-5 iterations)                           │
└─────────────────────────────────────────────────────────────────┘
```

### Performance Targets

| Metric | Target |
|--------|--------|
| Job start latency (Modal) | <60 seconds from trigger to first step |
| Training throughput (7B QLoRA) | ~1,000 examples/min on A10G |
| Checkpoint save time | <30 seconds per checkpoint |
| Auto-recovery from OOM | <2 minutes (reduce batch, restart) |
| Cost overhead vs raw GPU | <15% (orchestration + storage) |

---

## 7. Layer 4: Evaluator Arena

### Purpose
Automatically evaluate fine-tuned models against base models, detect regressions, and provide quality scores users can understand.

### Architecture

```
┌──────────────────────────────────────────────────────────────────────────┐
│                        EVALUATOR ARENA                                   │
│                                                                          │
│  ┌─────────────────────────────────────────────────────────────┐        │
│  │                 EVALUATION SUITE                             │        │
│  │                                                              │        │
│  │  1. DOMAIN EVALUATION (from user's data)                    │        │
│  │     ├─ Hold-out test set (10% of training data)             │        │
│  │     ├─ Auto-generated evaluation questions                  │        │
│  │     ├─ LLM-as-Judge scoring (1-5, detailed rubric)         │        │
│  │     └─ Metrics: accuracy, completeness, faithfulness        │        │
│  │                                                              │        │
│  │  2. GENERAL CAPABILITY CHECK (forgetting detection)         │        │
│  │     ├─ Mini-benchmark suite (196 questions)                 │        │
│  │     ├─ Covers: reasoning, math, coding, general knowledge  │        │
│  │     ├─ Compare: base model vs fine-tuned                    │        │
│  │     └─ Alert if general score drops >10%                    │        │
│  │                                                              │        │
│  │  3. A/B COMPARISON                                          │        │
│  │     ├─ Side-by-side: base model vs fine-tuned              │        │
│  │     ├─ Same prompts, blind comparison                       │        │
│  │     ├─ LLM-as-Judge picks winner per prompt                │        │
│  │     └─ Win rate + confidence interval                       │        │
│  │                                                              │        │
│  │  4. SAFETY CHECK                                            │        │
│  │     ├─ Standard safety prompts (ToxiGen, BBQ bias)         │        │
│  │     ├─ Refusal rate on harmful prompts                     │        │
│  │     └─ Flag if safety degraded vs base model               │        │
│  │                                                              │        │
│  └─────────────────────────────────────────────────────────────┘        │
│                              │                                           │
│                              ▼                                           │
│  ┌─────────────────────────────────────────────────────────────┐        │
│  │                 EVALUATION REPORT                            │        │
│  │                                                              │        │
│  │  ┌─────────────────────────────────────────────────────┐    │        │
│  │  │  OVERALL SCORE: 82/100  ✓ Ready for deployment      │    │        │
│  │  └─────────────────────────────────────────────────────┘    │        │
│  │                                                              │        │
│  │  Domain Knowledge:     ████████████████████░░  85%          │        │
│  │  Response Quality:     ███████████████████░░░  80%          │        │
│  │  Source Faithfulness:  ██████████████████████░  90%          │        │
│  │  General Capability:   ████████████████░░░░░░  72%          │        │
│  │  Safety:               █████████████████████░  95%          │        │
│  │                                                              │        │
│  │  ⚠ General capability dropped 8% (acceptable)              │        │
│  │  ✓ No safety regression detected                            │        │
│  │  ✓ Wins A/B comparison 73% of the time                     │        │
│  │                                                              │        │
│  │  Recommendation: Deploy with confidence                     │        │
│  └─────────────────────────────────────────────────────────────┘        │
└──────────────────────────────────────────────────────────────────────────┘
```

### Tech Stack

| Component | Technology | Notes |
|-----------|-----------|-------|
| LLM-as-Judge | Claude Sonnet / GPT-4o | Best correlation with human judgment |
| Benchmark runner | `lm-eval-harness` (EleutherAI) | Standard benchmark framework |
| Safety evaluation | `ToxiGen`, `BBQ` datasets | Standard safety benchmarks |
| A/B test engine | Custom (vLLM inference) | Run both models, judge compares |
| Report generation | Custom templates | User-friendly, non-technical language |

### Performance Targets

| Metric | Target |
|--------|--------|
| Full evaluation time | <30 minutes (7B model) |
| Domain eval coverage | 100% of hold-out test set |
| General capability tests | 196 questions (bundled general_benchmark.json) |
| Judge agreement rate | >80% with human preference |
| Report generation | <1 minute after eval completes |

---

## 8. Layer 5: Serving & Deployment

### Purpose
Deploy fine-tuned models for inference with minimal latency and maximum cost efficiency.

### Current Implemented Architecture

```text
admin registers inference instances
  -> control plane tracks backend_type, base_model, health, lifecycle, capacity
  -> deploy claims a compatible healthy ready instance
  -> model stores inference_instance_id
  -> adapter loads on that assigned instance
  -> inference resolves model -> assigned instance -> backend
  -> undeploy unloads from the assigned instance and frees slot count
```

Key properties of the current system:
- backend abstraction supports `vllm`, `tgi`, and `sglang`
- single-instance mode still works through `INFERENCE_SERVER_URL`
- multi-instance mode is enabled by feature flag
- deploy/inference/undeploy are all instance-aware once enabled
- health probes and reconciliation repair stale instance state
- API keys remain scoped per model
- billing is routed through the durable outbox path

### Current Serving Layers

| Layer | Responsibility |
|------|----------------|
| Inference backend abstraction | Build engine-specific clients without hard-coupling routes/services to one engine |
| Inference instance registry | Track backend type, URL, base model, lifecycle, health, and adapter capacity |
| Deployment service | Claim capacity, bind model to instance, load adapter, persist routing state |
| Inference route | Resolve model binding, pick assigned backend instance, stream or batch requests |
| Reconciler | Health probes, stale-state detection, adapter count reconciliation |

### Current Constraints

The implemented control plane is intentionally scoped to:
- explicit admin registration of instances
- DB-backed capacity accounting
- health probing from the API control plane
- manual fleet growth

It does not yet attempt:
- auto-provisioning
- Kubernetes operator logic
- cross-region scheduling
- autoscaling policies

Those are future operational layers, not missing core architecture.

### Performance Targets

| Metric | Managed API | Dedicated | Edge |
|--------|------------|-----------|------|
| TTFT (time to first token) | <200ms | <500ms (warm), <2s (cold) | N/A |
| Throughput | 30-50 tokens/sec | 30-50 tokens/sec | Device-dependent |
| Availability | 99.9% | 99.5% | N/A |
| Cold start | N/A | <2 seconds | N/A |
| Adapter swap time | <10ms | N/A | N/A |

---

## 9. Layer 6: Orchestration & Control Plane

### Purpose
Coordinate all pipeline stages, handle failures, manage state, and provide durable execution guarantees.

### Architecture

```
┌──────────────────────────────────────────────────────────────────────────┐
│                    ORCHESTRATION (Temporal.io)                            │
│                                                                          │
│  ┌──────────────────────────────────────────────────────────────┐       │
│  │              WORKFLOW: FullPipeline                           │       │
│  │                                                               │       │
│  │  ┌─────────┐  ┌─────────┐  ┌─────────┐  ┌─────────┐        │       │
│  │  │ Ingest  │─▶│ Refine  │─▶│ Train   │─▶│ Evaluate│        │       │
│  │  │ Workflow│  │ Workflow│  │ Workflow│  │ Workflow│        │       │
│  │  └────┬────┘  └────┬────┘  └────┬────┘  └────┬────┘        │       │
│  │       │            │            │            │               │       │
│  │       │            │            │            ▼               │       │
│  │       │            │            │       ┌─────────┐         │       │
│  │       │            │            │       │ Deploy  │         │       │
│  │       │            │            │       │ Workflow│         │       │
│  │       │            │            │       └─────────┘         │       │
│  │       │            │            │                            │       │
│  │  Each workflow is independently retryable with              │       │
│  │  checkpoint/resume semantics                                │       │
│  └──────────────────────────────────────────────────────────────┘       │
│                                                                          │
│  ┌──────────────────────────────────────────────────────────────┐       │
│  │              WORKFLOW: IngestWorkflow                         │       │
│  │                                                               │       │
│  │  Activities:                                                 │       │
│  │  1. validate_upload(files) → file_ids                        │       │
│  │  2. scan_virus(file_ids) → clean_file_ids                   │       │
│  │  3. classify_documents(clean_file_ids) → doc_metadata       │       │
│  │  4. parse_documents(doc_metadata) → parsed_docs             │       │
│  │  5. verify_parse_quality(parsed_docs) → quality_report      │       │
│  │  6. retry_failed_parses(quality_report) → final_parsed      │       │
│  │                                                               │       │
│  │  Retry policy: 3 attempts, exponential backoff              │       │
│  │  Timeout: 30 min per activity                               │       │
│  │  Heartbeat: every 30 seconds                                │       │
│  └──────────────────────────────────────────────────────────────┘       │
│                                                                          │
│  ┌──────────────────────────────────────────────────────────────┐       │
│  │              WORKFLOW: RefineWorkflow                         │       │
│  │                                                               │       │
│  │  Activities:                                                 │       │
│  │  1. chunk_documents(parsed_docs) → chunks                   │       │
│  │  2. synthesize_data(chunks, task_config) → raw_pairs        │       │
│  │  3. deduplicate(raw_pairs) → unique_pairs                   │       │
│  │  4. quality_score(unique_pairs) → scored_pairs              │       │
│  │  5. filter_by_threshold(scored_pairs) → filtered_pairs      │       │
│  │  6. check_diversity(filtered_pairs) → diverse_pairs         │       │
│  │  7. mix_general_data(diverse_pairs) → final_dataset         │       │
│  │  8. present_samples_to_user(final_dataset) → [WAIT]        │       │
│  │  9. apply_user_feedback(user_edits) → approved_dataset      │       │
│  │                                                               │       │
│  │  Step 8 is a SIGNAL — workflow pauses until user approves   │       │
│  │  Timeout: 7 days (user can take their time)                 │       │
│  └──────────────────────────────────────────────────────────────┘       │
│                                                                          │
│  ┌──────────────────────────────────────────────────────────────┐       │
│  │              WORKFLOW: TrainWorkflow                          │       │
│  │                                                               │       │
│  │  Activities:                                                 │       │
│  │  1. auto_configure(model, dataset) → hyperparams            │       │
│  │  2. estimate_cost(hyperparams) → cost_estimate              │       │
│  │  3. request_user_approval(cost_estimate) → [WAIT]           │       │
│  │  4. provision_gpu(hyperparams.gpu_class) → gpu_session      │       │
│  │  5. load_model(gpu_session, model_id) → loaded_model        │       │
│  │  6. run_training(loaded_model, dataset, hyperparams) → ckpt │       │
│  │  7. save_adapter(ckpt) → adapter_path                       │       │
│  │  8. [optional] run_dpo(adapter_path, pref_data) → aligned   │       │
│  │  9. release_gpu(gpu_session)                                │       │
│  │                                                               │       │
│  │  Step 6 sends heartbeats with training metrics              │       │
│  │  Checkpoint saves go to S3 every N steps                    │       │
│  │  On failure: resume from last checkpoint                    │       │
│  └──────────────────────────────────────────────────────────────┘       │
│                                                                          │
│  ┌──────────────────────────────────────────────────────────────┐       │
│  │              WORKFLOW: EvalWorkflow                           │       │
│  │                                                               │       │
│  │  Activities:                                                 │       │
│  │  1. load_models(base_model, fine_tuned) → loaded            │       │
│  │  2. run_domain_eval(loaded, test_set) → domain_scores       │       │
│  │  3. run_general_eval(loaded, benchmark) → general_scores    │       │
│  │  4. run_ab_test(loaded, eval_prompts) → ab_results          │       │
│  │  5. run_safety_check(loaded) → safety_scores                │       │
│  │  6. generate_report(all_scores) → eval_report               │       │
│  │  7. present_report_to_user(eval_report) → [WAIT]           │       │
│  └──────────────────────────────────────────────────────────────┘       │
│                                                                          │
│  ┌──────────────────────────────────────────────────────────────┐       │
│  │              WORKFLOW: DeployWorkflow                         │       │
│  │                                                               │       │
│  │  Activities:                                                 │       │
│  │  1. select_deployment_type(user_choice) → deploy_config     │       │
│  │  2. [if managed] register_adapter(adapter, base_model)      │       │
│  │  3. [if dedicated] provision_endpoint(model)                │       │
│  │  4. [if edge] export_model(adapter, format, quantization)   │       │
│  │  5. generate_api_keys(deployment) → api_key                 │       │
│  │  6. run_smoke_test(deployment) → health_check               │       │
│  │  7. notify_user(api_key, endpoint) → DONE                  │       │
│  └──────────────────────────────────────────────────────────────┘       │
└──────────────────────────────────────────────────────────────────────────┘
```

### Why Temporal.io

| Requirement | Temporal Feature |
|-------------|-----------------|
| Long-running pipelines (hours-days) | Durable execution, survives crashes |
| Human-in-the-loop pauses | Signals & queries, workflow waits for user |
| Retry with backoff | Built-in retry policies per activity |
| Resume from checkpoint | Workflow history replay |
| Observability | Built-in web UI, metrics, tracing |
| Exactly-once semantics | Activity idempotency guarantees |
| Multi-step pipelines | Workflow composition, child workflows |

### Tech Stack

| Component | Technology | Notes |
|-----------|-----------|-------|
| Workflow engine | Temporal.io (self-hosted or cloud) | Python SDK for workers |
| Task queues | Temporal task queues | Separate queues per GPU class |
| Monitoring | Temporal web UI + Grafana | Pipeline visibility |
| Alerting | PagerDuty / Slack webhooks | Failure notifications |

---

## 10. Layer 7: API Gateway & User Interface

### API Architecture

```
┌──────────────────────────────────────────────────────────────────────────┐
│                         API GATEWAY                                      │
│                         Rust (Axum)                                      │
│                                                                          │
│  Authentication:  Clerk / Auth.js (JWT)                                 │
│  Rate Limiting:   Redis-based sliding window                            │
│  CORS:            Configurable per deployment                           │
│                                                                          │
│  ┌──────────────────────────────────────────────────────────────┐       │
│  │                    API ENDPOINTS                              │       │
│  │                                                               │       │
│  │  PROJECTS                                                    │       │
│  │  POST   /api/v1/projects                    Create project   │       │
│  │  GET    /api/v1/projects                    List projects    │       │
│  │  GET    /api/v1/projects/:id                Get project      │       │
│  │  DELETE /api/v1/projects/:id                Delete project   │       │
│  │                                                               │       │
│  │  DOCUMENTS                                                   │       │
│  │  POST   /api/v1/projects/:id/documents      Upload docs     │       │
│  │  GET    /api/v1/projects/:id/documents      List docs       │       │
│  │  GET    /api/v1/documents/:id/status        Parse status    │       │
│  │                                                               │       │
│  │  DATASETS                                                    │       │
│  │  POST   /api/v1/projects/:id/datasets       Create dataset  │       │
│  │  GET    /api/v1/datasets/:id                Get dataset      │       │
│  │  GET    /api/v1/datasets/:id/samples        Preview samples │       │
│  │  POST   /api/v1/datasets/:id/feedback       Submit feedback │       │
│  │  GET    /api/v1/datasets/:id/export         Download JSONL  │       │
│  │                                                               │       │
│  │  TRAINING                                                    │       │
│  │  POST   /api/v1/projects/:id/train          Start training  │       │
│  │  GET    /api/v1/training/:id/status         Training status │       │
│  │  GET    /api/v1/training/:id/metrics        Live metrics    │       │
│  │  POST   /api/v1/training/:id/cancel         Cancel training │       │
│  │                                                               │       │
│  │  EVALUATION                                                  │       │
│  │  GET    /api/v1/models/:id/evaluation       Eval report     │       │
│  │  POST   /api/v1/models/:id/evaluate         Trigger eval    │       │
│  │                                                               │       │
│  │  INFERENCE                                                   │       │
│  │  POST   /api/v1/models/:id/chat/completions OpenAI-compat  │       │
│  │  POST   /api/v1/models/:id/completions      Completions    │       │
│  │                                                               │       │
│  │  DEPLOYMENT                                                  │       │
│  │  POST   /api/v1/models/:id/deploy           Deploy model    │       │
│  │  GET    /api/v1/models/:id/deployment       Deploy status   │       │
│  │  POST   /api/v1/models/:id/export           Export GGUF     │       │
│  │                                                               │       │
│  │  WEBHOOKS                                                    │       │
│  │  POST   /api/v1/webhooks                    Register hook   │       │
│  │  Events: pipeline.complete, training.done, eval.ready       │       │
│  └──────────────────────────────────────────────────────────────┘       │
│                                                                          │
│  WebSocket: /ws/v1/training/:id/stream  (real-time training metrics)   │
│  SSE:       /api/v1/models/:id/chat/stream (streaming inference)       │
└──────────────────────────────────────────────────────────────────────────┘
```

### User Interface Architecture

```
┌──────────────────────────────────────────────────────────────────────────┐
│                         FRONTEND                                         │
│                    Next.js 15 + React 19                                 │
│                                                                          │
│  ┌─────────────────────────────────────────────────────────────┐        │
│  │  PAGE: Dashboard                                             │        │
│  │  ├─ Project list with status indicators                     │        │
│  │  ├─ Quick stats (models trained, tokens served)             │        │
│  │  └─ Usage summary                                            │        │
│  │                                                              │        │
│  │  PAGE: New Project Wizard                                    │        │
│  │  ├─ Step 1: Upload documents (drag & drop)                  │        │
│  │  ├─ Step 2: What should the model do? (3-5 questions)       │        │
│  │  │   ├─ "Answer questions about my documents"               │        │
│  │  │   ├─ "Follow specific writing style/rules"               │        │
│  │  │   ├─ "Analyze and reason about problems"                 │        │
│  │  │   └─ Custom description (free text)                      │        │
│  │  ├─ Step 3: Choose base model (recommended auto-selected)   │        │
│  │  ├─ Step 4: Review cost estimate                            │        │
│  │  └─ Step 5: Start pipeline (one click)                      │        │
│  │                                                              │        │
│  │  PAGE: Pipeline Monitor                                      │        │
│  │  ├─ Stage progress bar (Ingest → Refine → Train → Eval)    │        │
│  │  ├─ Real-time logs (expandable)                             │        │
│  │  ├─ Live training charts (loss, LR, GPU util)              │        │
│  │  └─ Cost accumulator                                        │        │
│  │                                                              │        │
│  │  PAGE: Data Review                                           │        │
│  │  ├─ Sample training pairs (paginated)                       │        │
│  │  ├─ Accept / Reject / Edit per sample                       │        │
│  │  ├─ Quality score distribution chart                        │        │
│  │  └─ Source document highlighting (where data came from)     │        │
│  │                                                              │        │
│  │  PAGE: Evaluation Report                                     │        │
│  │  ├─ Overall score with visual gauge                         │        │
│  │  ├─ Category breakdown (domain, general, safety)            │        │
│  │  ├─ A/B comparison examples                                 │        │
│  │  └─ Forgetting analysis (before/after on general tasks)     │        │
│  │                                                              │        │
│  │  PAGE: Model Playground                                      │        │
│  │  ├─ Chat interface (test your model)                        │        │
│  │  ├─ Side-by-side with base model                            │        │
│  │  ├─ Temperature / top_p / max_tokens controls               │        │
│  │  └─ Thumbs up/down feedback (feeds DPO pipeline)           │        │
│  │                                                              │        │
│  │  PAGE: Deployment                                            │        │
│  │  ├─ API key display and management                          │        │
│  │  ├─ Code snippets (Python, JS, cURL)                       │        │
│  │  ├─ Usage metrics (requests, tokens, latency)              │        │
│  │  └─ Download options (GGUF, ONNX)                          │        │
│  └─────────────────────────────────────────────────────────────┘        │
│                                                                          │
│  Tech: Next.js 15, React 19, Tailwind CSS, shadcn/ui                   │
│  Charts: Recharts                                                        │
│  Real-time: WebSocket (training) + SSE (inference)                      │
│  State: Zustand (client) + React Query (server)                         │
└──────────────────────────────────────────────────────────────────────────┘
```

---

## 11. Data Flow Diagrams

### Complete Pipeline Flow

```
USER ACTION                 SYSTEM                              STORAGE
───────────                 ──────                              ───────

Upload 5 PDFs ──────────▶  Virus scan
                            │
                            ▼
                            Classify (type, lang, domain) ────▶ PostgreSQL
                            │                                   (doc metadata)
                            ▼
                            Parse (MinerU/Docling/Nougat)
                            │
                            ▼
                            Quality check parse
                            │
                            ├─ If <70%: retry alt parser
                            │
                            ▼
                            Store parsed output ──────────────▶ S3/R2
                            │                                   (parsed JSON)
                            ▼
                            Chunk documents
                            │
                            ▼
                            Embed chunks ─────────────────────▶ Qdrant
                            │                                   (vectors)
                            ▼
                            Synthesize Q&A pairs
                            │ (calls LLM API: Claude/GPT)
                            │
                            ▼
                            Deduplicate (MinHash)
                            │
                            ▼
                            Quality score (LLM-as-Judge)
                            │
                            ▼
                            Filter (threshold >= 3.5/5)
                            │
                            ▼
                            Mix with general data (70/30)
                            │
                            ▼
                            Store dataset ────────────────────▶ S3/R2
                            │                                   (JSONL)
                            ▼
Review 15 samples ◀────── Present samples to user
                            │
Accept / Edit ─────────▶  Apply feedback
                            │
                            ▼
                            Auto-configure hyperparams
                            │
                            ▼
                            Estimate cost ────────────────────▶ Show to user
                            │
Approve cost ──────────▶  Provision GPU (Modal/RunPod)
                            │
                            ▼
                            Load base model + dataset
                            │
                            ▼
                            Train (SFT) ──────────────────────▶ W&B (metrics)
                            │                                   S3 (checkpoints)
                            ▼
                            [Optional] DPO alignment
                            │
                            ▼
                            Save LoRA adapter ────────────────▶ S3/R2
                            │                                   (adapter files)
                            ▼
                            Release GPU
                            │
                            ▼
                            Run evaluation suite
                            │
                            ▼
                            Generate report ──────────────────▶ PostgreSQL
                            │                                   (eval results)
                            ▼
View report ◀───────────  Present evaluation report
                            │
Deploy model ──────────▶  Register adapter with vLLM
                            │
                            ▼
                            Generate API key ─────────────────▶ PostgreSQL
                            │
                            ▼
Get API key + endpoint ◀  Ready for inference
```

### Data Transformation Flow

```
RAW DOCUMENT                    STRUCTURED TEXT                     TRAINING PAIR
──────────────                  ───────────────                     ─────────────

┌──────────────┐    Parse     ┌──────────────┐    Synthesize    ┌──────────────┐
│ contract.pdf │───────────▶  │ {             │─────────────▶   │ {            │
│              │              │   "sections": │                 │  "messages": │
│ 47 pages     │              │   [{          │                 │  [           │
│ Tables       │              │     "type":   │                 │   {system},  │
│ Legalese     │              │     "heading",│                 │   {user:     │
│              │              │     "content": │                 │    "What is  │
│              │              │     "..."     │                 │     the non- │
│              │              │   }]          │                 │     compete  │
│              │              │ }             │                 │     clause?"}│
│              │              │              │                 │   {assistant:│
│              │              │ 2,847 tokens  │                 │    "The non- │
│              │              │ per section   │                 │     compete..│
│              │              │              │                 │    "}        │
│              │              │              │                 │  ]           │
└──────────────┘              └──────────────┘                 └──────────────┘

Size: 2.3 MB                  Size: 890 KB                     Size: 1.2 KB
Format: PDF                   Format: JSON                     Format: JSONL
                                                                × 150 pairs
                                                                = 180 KB total
```

---

## 12. Storage Architecture

### Storage Layout

```
┌──────────────────────────────────────────────────────────────────────────┐
│                        STORAGE ARCHITECTURE                              │
│                                                                          │
│  ┌─────────────────────────────────────────────┐                        │
│  │  OBJECT STORAGE (S3 / Cloudflare R2)        │                        │
│  │                                              │                        │
│  │  platform-{env}/                          │                        │
│  │  ├── uploads/                               │                        │
│  │  │   └── {tenant_id}/{project_id}/          │                        │
│  │  │       └── {file_id}.{ext}                │                        │
│  │  ├── parsed/                                │                        │
│  │  │   └── {tenant_id}/{project_id}/          │                        │
│  │  │       └── {doc_id}.json                  │                        │
│  │  ├── datasets/                              │                        │
│  │  │   └── {tenant_id}/{project_id}/          │                        │
│  │  │       ├── {dataset_id}.jsonl             │                        │
│  │  │       └── {dataset_id}_meta.json         │                        │
│  │  ├── checkpoints/                           │                        │
│  │  │   └── {tenant_id}/{training_id}/         │                        │
│  │  │       └── checkpoint-{step}/             │                        │
│  │  ├── adapters/                              │                        │
│  │  │   └── {tenant_id}/{model_id}/            │                        │
│  │  │       ├── adapter_model.safetensors      │                        │
│  │  │       ├── adapter_config.json            │                        │
│  │  │       └── tokenizer_config.json          │                        │
│  │  └── exports/                               │                        │
│  │      └── {tenant_id}/{model_id}/            │                        │
│  │          └── model-{quant}.gguf             │                        │
│  │                                              │                        │
│  │  Lifecycle: uploads auto-delete after 30d   │                        │
│  │  Encryption: AES-256 at rest                │                        │
│  └─────────────────────────────────────────────┘                        │
│                                                                          │
│  ┌─────────────────────────────────────────────┐                        │
│  │  POSTGRESQL 16 (Metadata)                   │                        │
│  │                                              │                        │
│  │  Tables:                                    │                        │
│  │  ├── tenants (id, name, plan, created_at)   │                        │
│  │  ├── projects (id, tenant_id, name, config) │                        │
│  │  ├── documents (id, project_id, status,     │                        │
│  │  │             file_path, parse_quality,     │                        │
│  │  │             metadata JSONB)              │                        │
│  │  ├── datasets (id, project_id, file_path,   │                        │
│  │  │            stats JSONB, status)          │                        │
│  │  ├── training_jobs (id, project_id, model,  │                        │
│  │  │                  hyperparams JSONB,       │                        │
│  │  │                  status, metrics JSONB,   │                        │
│  │  │                  cost, gpu_class)         │                        │
│  │  ├── models (id, training_job_id,           │                        │
│  │  │          adapter_path, eval_scores,       │                        │
│  │  │          deployment_status)              │                        │
│  │  ├── evaluations (id, model_id, scores      │                        │
│  │  │               JSONB, report JSONB)       │                        │
│  │  ├── api_keys (id, model_id, key_hash,      │                        │
│  │  │            rate_limit, usage)            │                        │
│  │  └── billing (tenant_id, operation,         │                        │
│  │              tokens, cost, timestamp)        │                        │
│  │                                              │                        │
│  │  Indexes: tenant_id on all tables,          │                        │
│  │  status + created_at composite indexes      │                        │
│  └─────────────────────────────────────────────┘                        │
│                                                                          │
│  ┌─────────────────────────────────────────────┐                        │
│  │  QDRANT (Vector Store)                      │                        │
│  │                                              │                        │
│  │  Collections:                               │                        │
│  │  ├── chunks_{project_id}                    │                        │
│  │  │   ├─ vector: 1024d (bge-large-en-v1.5)  │                        │
│  │  │   ├─ payload: chunk_text, doc_id,        │                        │
│  │  │   │          section_path, page_num      │                        │
│  │  │   └─ used for: semantic dedup, diversity │                        │
│  │  └── training_pairs_{project_id}            │                        │
│  │      ├─ vector: 1024d                       │                        │
│  │      ├─ payload: pair_id, quality_score     │                        │
│  │      └─ used for: semantic dedup            │                        │
│  │                                              │                        │
│  │  Tenant isolation: collection per project   │                        │
│  └─────────────────────────────────────────────┘                        │
│                                                                          │
│  ┌─────────────────────────────────────────────┐                        │
│  │  REDIS 7 (Cache & Real-time)                │                        │
│  │                                              │                        │
│  │  Uses:                                      │                        │
│  │  ├── Session cache (auth tokens)            │                        │
│  │  ├── Rate limiting (sliding window)         │                        │
│  │  ├── Job status (pipeline progress)         │                        │
│  │  ├── Training metrics stream (real-time)    │                        │
│  │  ├── Message bus (Redis Streams) for MVP    │                        │
│  │  └── LLM response cache (synthesis dedup)   │                        │
│  │                                              │                        │
│  │  TTL: session=24h, metrics=7d, cache=1h    │                        │
│  └─────────────────────────────────────────────┘                        │
└──────────────────────────────────────────────────────────────────────────┘
```

---

## 13. Multi-Tenancy & Isolation

### Isolation Model

```
┌───────────────────────────────────────────────────────────┐
│                  MULTI-TENANCY MODEL                       │
│                                                            │
│  Layer          │ Isolation Strategy                       │
│  ───────────────│──────────────────────────────────────    │
│  API Gateway    │ JWT with tenant_id claim                │
│  Compute (GPU)  │ Dedicated containers per training job   │
│  Object Storage │ Prefix-based: /{tenant_id}/*            │
│  PostgreSQL     │ Row-level security (RLS) on tenant_id   │
│  Qdrant         │ Collection per project                  │
│  Redis          │ Key prefix: {tenant_id}:*               │
│  vLLM Serving   │ Adapter-level isolation (S-LoRA)        │
│  Temporal       │ Namespace per tenant                    │
│                                                            │
│  Data never leaves tenant boundary.                       │
│  Training jobs run in isolated containers.                │
│  No shared GPU memory between tenants.                    │
└───────────────────────────────────────────────────────────┘
```

---

## 14. Performance Metrics & SLAs

### End-to-End Pipeline SLAs

| Pipeline Stage | Target Duration | Notes |
|---------------|----------------|-------|
| Upload + Parse (100 pages) | <5 min | Parallel parsing |
| Chunking | <1 min | CPU-only, fast |
| Synthesis (10K pairs) | 10-20 hours | LLM API bound |
| Synthesis (1K pairs) | 1-2 hours | LLM API bound |
| Quality filtering | <30 min | Parallel scoring |
| Training (7B, 10K examples) | 1-3 hours | GPU bound |
| Training (70B, 50K examples) | 1-3 days | GPU bound |
| Evaluation | <30 min | Inference + scoring |
| Deployment | <5 min | Adapter registration |
| **Total (7B, quick, 1K)** | **~3-4 hours** | Upload to deployed |
| **Total (7B, quality, 10K)** | **~24-48 hours** | Upload to deployed |

### System Performance SLAs

| Metric | Target | Measurement |
|--------|--------|-------------|
| API latency (p50) | <100ms | Non-GPU endpoints |
| API latency (p99) | <500ms | Non-GPU endpoints |
| Inference TTFT (managed) | <200ms | Time to first token |
| Inference throughput | 30-50 tok/s | Per request |
| Upload throughput | 100 MB/s | Per connection |
| Pipeline availability | 99.9% | Monthly |
| Inference availability | 99.9% | Monthly |
| Data durability | 99.999999999% (11 9's) | S3/R2 guarantees |

### Observability Stack

```
┌──────────────────────────────────────────────────┐
│                OBSERVABILITY                      │
│                                                   │
│  Metrics:    Prometheus + Grafana                │
│  Logs:       Loki (or CloudWatch)                │
│  Traces:     OpenTelemetry → Jaeger/Tempo        │
│  Alerts:     Grafana Alerting → Slack/PagerDuty  │
│  Dashboards: Per-layer metrics                   │
│                                                   │
│  Key Dashboards:                                 │
│  ├── Pipeline: stage durations, throughput       │
│  ├── GPU: utilization, VRAM, cost/hr            │
│  ├── Inference: latency, throughput, errors      │
│  ├── Quality: parse scores, synth quality        │
│  └── Usage: models trained, tokens served         │
└──────────────────────────────────────────────────┘
```

---

## 15. Cost Model

### Per-User Cost Breakdown (7B QLoRA, 10K training pairs)

| Operation | Estimated Cost | Provider |
|-----------|---------------|----------|
| Document parsing (500 pages) | $0.50-1.00 | GPU compute |
| Synthetic data generation | $5-15 | LLM API (Tier 2) |
| Quality scoring (LLM-as-Judge) | $2-5 | LLM API |
| Training (7B QLoRA, A10G, 2hrs) | $2.20 | Modal |
| Evaluation (inference + judge) | $1-3 | GPU + LLM API |
| **Total pipeline cost** | **$10-26** | One-time |
| Serving (managed, per month) | $15-30 | vLLM shared |

### Infrastructure Fixed Costs (Monthly)

| Component | Cost | Notes |
|-----------|------|-------|
| PostgreSQL | $0 | self-hosted on the deploy box |
| Redis (managed) | $30-100 | Upstash / ElastiCache |
| Qdrant (cloud) | $50-150 | Qdrant Cloud |
| S3/R2 storage (1TB) | $15-25 | R2: $0.015/GB |
| Temporal.io (cloud) | $200 | Or self-host on K8s |
| vLLM inference (1 H100) | $2,850 | Serves ~100 users |
| Monitoring (Grafana Cloud) | $50 | Free tier initially |
| **Total fixed cost** | **~$3,500/month** | Serving 100 users |

### Cost Tiers (Reference)

If this were offered as a service, here's roughly how the economics would break down:

| Tier | Est. Cost/month | Scope | Use Case |
|------|----------------|-------|----------|
| Personal | ~$49 | 2 models, 1K pairs each, managed API | Personal projects |
| Small Team | ~$199 | 10 models, 10K pairs each, managed API | Team experimentation |
| Heavy Use | ~$499 | Unlimited models, 50K pairs, dedicated endpoints | Larger workloads |

---

## 16. Security Architecture

```
┌──────────────────────────────────────────────────────────────────────────┐
│                       SECURITY ARCHITECTURE                              │
│                                                                          │
│  DATA SECURITY                                                          │
│  ├── Encryption at rest: AES-256 (S3/R2, PostgreSQL, Qdrant)           │
│  ├── Encryption in transit: TLS 1.3 everywhere                         │
│  ├── Tenant isolation: RLS, prefix isolation, separate containers      │
│  ├── No data sharing between tenants (ever)                            │
│  ├── Upload virus scanning (ClamAV)                                    │
│  └── PII detection and redaction (optional, presidio)                  │
│                                                                          │
│  ACCESS CONTROL                                                         │
│  ├── Authentication: Clerk / Auth.js (OAuth2 + passwordless)           │
│  ├── Authorization: RBAC (owner, editor, viewer per project)           │
│  ├── API keys: SHA-256 hashed, scoped per model                       │
│  ├── Rate limiting: per-user, per-API-key                              │
│  └── Audit log: all mutations logged with actor, timestamp            │
│                                                                          │
│  INFRASTRUCTURE                                                         │
│  ├── Network: VPC with private subnets for DB/GPU                      │
│  ├── Secrets: Vault / AWS Secrets Manager                              │
│  ├── GPU containers: ephemeral, destroyed after training               │
│  ├── No persistent SSH to GPU instances                                │
│  └── SOC 2 compliance target (Phase 3)                                 │
│                                                                          │
│  MODEL SECURITY                                                         │
│  ├── Adapters encrypted at rest                                        │
│  ├── Model download requires auth token                                │
│  ├── Safety evaluation before deployment                               │
│  └── Content filtering on inference (optional)                         │
└──────────────────────────────────────────────────────────────────────────┘
```

---

## 17. Phased Build Roadmap

### Phase 0: Foundation (Weeks 1-3)

```
Goal: Skeleton project, CI/CD, core infrastructure
──────────────────────────────────────────────────

Tasks:
├── Monorepo setup (Turborepo: /api, /web, /workers, /packages)
├── FastAPI skeleton with auth (Clerk)
├── PostgreSQL schema + migrations (Alembic)
├── S3/R2 integration (upload endpoint)
├── Redis setup
├── Docker Compose for local dev
├── CI/CD pipeline (GitHub Actions)
├── Basic Next.js frontend with auth
└── Temporal.io local dev setup

Deliverable: Empty pipeline that accepts uploads and stores metadata
Tech to learn: Temporal.io, FastAPI async patterns
```

### Phase 1: Ingestion Engine (Weeks 4-7)

```
Goal: Upload docs → Parsed structured text
──────────────────────────────────────────

Tasks:
├── Document classifier (type, language, domain)
├── MinerU 2.5 integration (Docker container)
├── Docling integration (CPU fallback)
├── Parse quality checker
├── Parser selection router
├── Temporal IngestWorkflow
├── Frontend: upload page + parse status
└── Testing: 50+ documents across formats

Deliverable: Upload any document, get structured parsed output
Tech to learn: MinerU setup, GPU container orchestration
Performance gate: >85% parse accuracy on test corpus
```

### Phase 2: Data Refinery (Weeks 8-14)

```
Goal: Parsed docs → Quality training dataset
──────────────────────────────────────────────

Tasks:
├── Chunking engine (structure-aware + semantic + recursive)
├── distilabel pipeline setup
├── Agent pipeline (analyzer → generator → answerer → verifier)
├── Prompt templates (per task type, per domain)
├── LLM backend integration (Claude API + GPT-4o-mini + open models)
├── MinHash deduplication
├── LLM-as-Judge quality scoring
├── Source grounding verification
├── Anti-forgetting data mixer
├── Temporal RefineWorkflow
├── Frontend: data review page (accept/reject/edit)
├── Frontend: sample preview + quality distribution
└── Testing: end-to-end from upload to dataset

Deliverable: Upload docs → Get reviewable training dataset
Tech to learn: distilabel, AgentInstruct patterns, embedding models
Performance gate: >60% quality gate pass rate, >85% source grounding
```

### Phase 3: Training Core (Weeks 15-20)

```
Goal: Dataset → Trained LoRA adapter
────────────────────────────────────

Tasks:
├── Auto-configurator (hyperparameter selection logic)
├── Modal integration (GPU provisioning)
├── Unsloth SFT training pipeline
├── DPO training pipeline (TRL)
├── Checkpoint management (S3)
├── Training monitor (real-time metrics → WebSocket)
├── Cost estimator + user approval flow
├── Auto-recovery (OOM, NaN, loss spike)
├── Temporal TrainWorkflow
├── Frontend: training dashboard (live charts)
├── Frontend: cost approval dialog
└── Testing: train 10+ models across sizes

Deliverable: One-click training from approved dataset
Tech to learn: Unsloth internals, Modal GPU functions, PEFT/QLoRA
Performance gate: Successful training on 7B/13B, cost within 15% of estimate
```

### Phase 4: Evaluator Arena (Weeks 21-24)

```
Goal: Trained model → Comprehensive evaluation report
──────────────────────────────────────────────────────

Tasks:
├── Domain evaluation (hold-out test set + LLM-Judge)
├── General capability benchmark (mini MMLU/HellaSwag)
├── A/B comparison engine (base vs fine-tuned)
├── Safety evaluation suite
├── Forgetting detection (before/after comparison)
├── Report generator (user-friendly language)
├── Temporal EvalWorkflow
├── Frontend: evaluation report page
├── Frontend: A/B comparison viewer
└── Testing: evaluate 10+ models, validate scores

Deliverable: Automatic evaluation with readable report
Tech to learn: lm-eval-harness, LLM-as-Judge prompt engineering
Performance gate: >80% agreement with human preferences
```

### Phase 5: Serving & Deployment (Weeks 25-30)

```
Goal: Deploy models for inference
─────────────────────────────────

Tasks:
├── vLLM cluster setup with S-LoRA
├── Adapter registration (dynamic loading)
├── OpenAI-compatible API proxy
├── API key management
├── Usage metering (tokens, requests)
├── GGUF export pipeline (llama.cpp quantize)
├── Streaming inference (SSE)
├── Model playground (chat interface)
├── Temporal DeployWorkflow
├── Frontend: deployment page + API key display
├── Frontend: model playground
├── Frontend: usage dashboard
└── Testing: load test 50+ concurrent users

Deliverable: Full end-to-end: upload → train → deploy → use
Tech to learn: vLLM S-LoRA config, GGUF quantization, SSE streaming
Performance gate: <200ms TTFT, 30+ tok/s, 99.9% uptime
```

### Phase 6: Polish & Scale (Weeks 31-36)

```
Goal: Production-grade, scalable, polished
────────────────────────────────────────────────

Tasks:
├── Usage tracking and metering
├── Cost estimation per operation
├── RunPod integration (cost optimization)
├── GRPO training mode (reasoning)
├── Iterative training mode (active learning)
├── Webhook system
├── Multi-model base support (Llama, Qwen, Mistral, DeepSeek)
├── Advanced evaluation (custom benchmarks)
├── Load testing
├── Documentation (API docs, guides)
└── Final polish and cleanup

Deliverable: Fully functional end-to-end system
```

### Timeline Summary

```
         Weeks 1-3      Weeks 4-7       Weeks 8-14
        ┌──────────┐   ┌──────────┐   ┌──────────────┐
        │ Phase 0  │──▶│ Phase 1  │──▶│   Phase 2    │
        │Foundation│   │ Ingestion│   │ Data Refinery │
        └──────────┘   └──────────┘   └──────┬───────┘
                                              │
         Weeks 15-20     Weeks 21-24         │
        ┌──────────┐   ┌──────────┐          │
        │ Phase 3  │──▶│ Phase 4  │◀─────────┘
        │ Training │   │ Evaluator│
        └──────────┘   └──────┬───┘
                              │
         Weeks 25-30     Weeks 31-36
        ┌──────────┐   ┌──────────┐
        │ Phase 5  │──▶│ Phase 6  │
        │ Serving  │   │ Polish   │
        └──────────┘   └──────────┘

Total: ~36 weeks (9 months) to production-ready
MVP (end-to-end demo): ~24 weeks (6 months)
```

---

## 18. Learning Map

What you need to learn for each domain, organized by priority.

### Priority 1: Must Learn Before Building

| Domain | What to Learn | Resources |
|--------|--------------|-----------|
| **Rust + Axum** | Async Rust, Tokio runtime, Axum routing/extractors/middleware, Tower layers | Rust Book, Axum docs, Tokio tutorial |
| **SQLx** | Compile-time checked queries, async PostgreSQL, migrations | SQLx docs, examples |
| **Temporal.io** | Workflows, Activities, Signals, Workers, Python SDK | Temporal docs, Python SDK tutorials |
| **LoRA/QLoRA** | How adapter training works, rank, alpha, target modules | HuggingFace PEFT docs, LoRA paper |
| **Unsloth** | API, model loading, training loop, saving adapters | Unsloth GitHub, examples |
| **TRL** | SFTTrainer, DPOTrainer configuration | HuggingFace TRL docs |
| **vLLM** | Server setup, S-LoRA, OpenAI-compatible serving | vLLM docs, S-LoRA paper |
| **Modal** | @function decorator, GPU provisioning, volumes, secrets | Modal docs, examples |

### Priority 2: Learn During Build

| Domain | What to Learn | Resources |
|--------|--------------|-----------|
| **MinerU** | Setup, API, custom configuration, output format | MinerU GitHub, docs |
| **distilabel** | Pipeline construction, built-in tasks, custom steps | distilabel docs, Argilla tutorials |
| **Prompt engineering** | AgentInstruct patterns, Evol-Instruct, LLM-as-Judge prompts | Papers: AgentInstruct, WizardLM |
| **Embeddings** | Sentence-transformers, similarity search, clustering | HF sentence-transformers docs |
| **Qdrant** | Collection management, filtering, multi-tenancy | Qdrant docs |
| **Next.js 15** | App router, server components, streaming, WebSocket | Next.js docs |

### Priority 3: Learn for Scale

| Domain | What to Learn | Resources |
|--------|--------------|-----------|
| **Kubernetes** | GPU operator, pod scheduling, resource limits | K8s docs, NVIDIA GPU Operator |
| **RunPod** | Pods API, serverless endpoints, volume management | RunPod docs |
| **GRPO** | Group Relative Policy Optimization, reward functions | DeepSeek-R1 paper, TRL GRPO docs |
| **GGUF quantization** | llama.cpp quantize, quantization types, quality tradeoffs | llama.cpp docs |
| **Prometheus/Grafana** | Metrics collection, dashboard creation, alerting | Grafana docs |

### Recommended Learning Order

```
Week 1-2:  Rust + Axum + Tokio (async Rust, build a REST API)
Week 3-4:  SQLx + PostgreSQL (compile-time queries, migrations)
Week 5-6:  LoRA/QLoRA theory + Unsloth hands-on (fine-tune a 7B model)
Week 7-8:  vLLM setup + S-LoRA (serve a model with adapters)
Week 9-10: Temporal.io + Modal (workflows + cloud GPU)
Week 11+:  MinerU, distilabel, prompt engineering (build as you learn)
```

---

## 19. Technology Decision Records

### TDR-001: Orchestration — Temporal.io vs Alternatives

**Decision:** Temporal.io

| Option | Pros | Cons |
|--------|------|------|
| **Temporal.io** | Durable execution, crash recovery, human-in-the-loop signals, Python SDK, built-in monitoring | Operational complexity, learning curve |
| Celery + Redis | Simple, widely used | No durability, no workflow state, manual retry logic |
| Airflow | DAG-based, good for data pipelines | Not designed for long-running interactive workflows |
| Prefect | Modern, Python-native | Less mature for long-running GPU workflows |

**Rationale:** These pipelines are long-running (hours), require human-in-the-loop pauses (data review, cost approval), and must survive crashes (GPU jobs are expensive). Temporal is purpose-built for exactly this pattern.

### TDR-002: Training Framework — Unsloth vs Alternatives

**Decision:** Unsloth (primary) + TRL (fallback)

| Option | Pros | Cons |
|--------|------|------|
| **Unsloth** | 2-5x faster, 60-80% less VRAM, zero accuracy loss | Single-GPU only, smaller community than TRL |
| TRL (standalone) | HF ecosystem, multi-GPU, all training methods | Slower, more VRAM |
| LLaMA-Factory | Zero-code, web UI | Hard to customize, less programmatic control |
| Axolotl | YAML config, multi-GPU | Less maintained, slower development |

**Rationale:** For single-GPU QLoRA fine-tuning (7B-70B models), Unsloth's speed and VRAM savings directly translate to lower compute costs. TRL as fallback for multi-GPU or unsupported models.

### TDR-003: GPU Provider — Modal vs RunPod

**Decision:** Modal (MVP) → RunPod (scale)

| Option | Pros | Cons |
|--------|------|------|
| **Modal (MVP)** | Best DX, Python-native, per-second billing, zero DevOps | Higher $/hr, limited GPU types |
| **RunPod (scale)** | Cheapest, most GPU types, serverless + pods | More setup, less polish |
| CoreWeave | InfiniBand, enterprise | Expensive, overkill for MVP |
| Lambda Labs | Cheap reserved | Availability issues, less flexible |

**Rationale:** Modal lets you ship fast with near-zero infrastructure code. For larger scale, RunPod offers 30-40% cost savings with more setup.

### TDR-004: Serving — Backend Abstraction vs Single-Engine Coupling

**Decision:** Pluggable inference backend abstraction, with `vllm` as the primary production backend today

| Option | Pros | Cons |
|--------|------|------|
| **Backend abstraction + vLLM default** | Keeps current best backend while avoiding hard-coupling, enables per-instance routing | More control-plane code |
| vLLM only | Simplest implementation | Harder future backend replacement |
| TGI only | Strong HF ecosystem | Different LoRA/serving tradeoffs |
| TensorRT-LLM only | Maximum throughput in some NVIDIA setups | Ecosystem lock-in, less flexible |

**Rationale:** vLLM remains the strongest default serving backend for multi-adapter GPU economics, but the platform should not be structurally coupled to one engine. The production code now routes through a backend abstraction and an inference instance registry so backend choice can evolve without rewriting deploy, inference, and undeploy flows.

### TDR-005: Document Parsing — MinerU vs Alternatives

**Decision:** MinerU 2.5 (primary) + Docling (CPU) + Nougat (academic)

| Option | Pros | Cons |
|--------|------|------|
| **MinerU 2.5** | Best accuracy (90.67), fast on GPU, LaTeX/table support | Requires GPU |
| **Docling** | No GPU needed, strong structural fidelity | Slower, less accurate |
| **Nougat** | Best for academic/scientific PDFs | Limited to academic format |
| Unstructured.io | Broadest format support | Less accurate on complex layouts |
| Commercial (Reducto) | Best table extraction | Cost per page, vendor lock-in |

**Rationale:** Multi-parser approach with intelligent routing. MinerU handles 80% of documents. Docling for CPU-only environments and simple docs. Nougat for academic papers. This gives us best accuracy across document types while maintaining a CPU fallback.

### TDR-006: Synthetic Data — distilabel + AgentInstruct Pattern

**Decision:** distilabel framework implementing AgentInstruct-style multi-agent pipelines

| Option | Pros | Cons |
|--------|------|------|
| **distilabel** | Production framework, HF integration, built-in Evol-Instruct | Learning curve |
| Custom scripts | Full control | Maintenance burden, reinventing the wheel |
| NeMo Data Designer | GPU-accelerated | NVIDIA lock-in, enterprise-heavy |
| HF Synthetic Data Gen | No-code | Too basic, limited customization |

**Rationale:** distilabel provides the pipeline framework; we implement AgentInstruct-style multi-agent workflows within it. This gives us production-grade infrastructure (retry, batching, caching) with state-of-the-art data generation quality.

### TDR-007: Storage — S3/R2 + PostgreSQL + Qdrant + Redis

**Decision:** Four-store architecture

| Store | Purpose | Why Not Consolidate |
|-------|---------|-------------------|
| **S3/R2** | Documents, datasets, adapters, exports | Object storage is cheapest for large blobs |
| **PostgreSQL** | Metadata, users, billing, audit logs | Relational integrity, JSONB flexibility, RLS |
| **Qdrant** | Chunk vectors, training pair vectors | Purpose-built for vector similarity at scale |
| **Redis** | Cache, sessions, rate limiting, streams | Sub-millisecond latency, pub/sub, streams |

**Rationale:** Each store is optimized for its access pattern. Consolidating (e.g., pgvector instead of Qdrant) would work for MVP but create scaling bottlenecks. Starting with the right stores avoids painful migrations later.

### TDR-008: Frontend — Next.js 15 + React 19

**Decision:** Next.js with App Router

| Option | Pros | Cons |
|--------|------|------|
| **Next.js 15** | SSR, streaming, server components, huge ecosystem | Vercel-centric defaults |
| SvelteKit | Faster, simpler | Smaller ecosystem, fewer ML-oriented libraries |
| Remix | Nested routes, loaders | Smaller community |
| SPA (Vite + React) | Simple, no SSR complexity | No SSR, slower initial load |

**Rationale:** Next.js gives us SSR for landing/docs pages, streaming for real-time updates, and the React ecosystem for charting libraries (Recharts) and component libraries (shadcn/ui). The ML/AI tooling ecosystem is React-first.

### TDR-009: Infrastructure Language — Rust vs Python vs Go

**Decision:** Rust for all infrastructure; Python only for ML-specific code

| Criteria | Rust (Axum + Tokio) | Go (Fiber/Chi) | Python (FastAPI) |
|----------|---------------------|-----------------|------------------|
| **HTTP throughput** | **#1** (TechEmpower) | ~30-50% behind | ~10-20x behind |
| **Memory/instance** | **~5-15 MB** | ~25-50 MB | ~80-200 MB |
| **Cold start** | **~2-5 ms** | ~10-20 ms | ~500-2000 ms |
| **100 instances RAM** | **~1.5 GB** | ~5 GB | ~20 GB |
| **Concurrency** | **Zero-cost futures** | Goroutines (good) | asyncio + GIL |
| **Type safety** | **Compile-time** | Static types | Runtime only |
| **Deployment** | **Single static binary** | Single binary | Runtime + venv |
| **S3/DB/Redis** | Mature async clients | Mature | Mature |
| **Dev speed** | Slower initially | **Fastest** | Fast |
| **ML ecosystem** | None | None | **Dominant** |

**Rationale:** Performance is a core goal (and learning Rust is half the point of this project). When scaling to 100s of instances:
- Rust uses **13x less memory** than Python (1.5 GB vs 20 GB for 100 instances)
- Rust cold starts are **100-400x faster** than Python (2ms vs 500-2000ms)
- Go's only advantage over Rust is faster development speed — no unique capability Rust lacks
- Python is kept **exclusively** for ML code where the ecosystem (Unsloth, TRL, distilabel, MinerU) has no Rust/Go equivalent

**Language boundary:**
```
Rust (Axum)  → API Gateway, file upload, DB queries, Redis, S3, auth, middleware
Python       → Training (Unsloth/TRL), synthesis (distilabel), parsing (MinerU), Temporal ML workers
TypeScript   → Frontend (Next.js)
```

---

## Summary: The Stack at a Glance

```
┌─────────────────────────────────────────────────────────────┐
│                    PLATFORM STACK                          │
│                                                              │
│  ── INFRASTRUCTURE (Rust) ──                                │
│  API Gateway:  Rust (Axum + Tokio + Tower)                  │
│  Database:     SQLx (compile-time checked, async)           │
│  Cache:        redis-rs (async)                             │
│  Storage:      aws-sdk-rust (S3/R2/MinIO)                   │
│  Auth:         jsonwebtoken (Clerk JWT verification)        │
│  Logging:      tracing + tracing-subscriber                 │
│                                                              │
│  ── ML PIPELINE (Python) ──                                 │
│  Parsing:      MinerU 2.5 + Docling + Nougat                │
│  Chunking:     Custom (structure-aware + semantic)           │
│  Synthesis:    distilabel (AgentInstruct-style agents)       │
│  Quality:      LLM-as-Judge + MinHash + perplexity          │
│  Training:     Unsloth + TRL (SFT, DPO, GRPO)              │
│  Workers:      Temporal.io (Python SDK for ML activities)   │
│                                                              │
│  ── SERVING & INFRA ──                                      │
│  Serving:      vLLM + S-LoRA                                │
│  GPU:          Modal (MVP) → RunPod (scale)                 │
│  Export:       GGUF (llama.cpp) + ONNX                      │
│                                                              │
│  ── FRONTEND (TypeScript) ──                                │
│  UI:           Next.js 15 + React 19 + Tailwind + shadcn   │
│  Auth:         @clerk/nextjs                                │
│  Data:         TanStack React Query                         │
│                                                              │
│  ── SHARED INFRASTRUCTURE ──                                │
│  Storage:      S3/R2 + PostgreSQL 16 + Qdrant + Redis 7    │
│  Orchestration:Temporal.io                                   │
│  Message Bus:  Redis Streams → NATS JetStream               │
│  Monitoring:   Prometheus + Grafana + OpenTelemetry          │
│  CI/CD:        GitHub Actions                                │
│  Monorepo:     Cargo workspace (Rust) + Turborepo (TS)      │
│                                                              │
│  LLM Backends: Claude Sonnet (quality) / GPT-4o-mini        │
│                (balanced) / Qwen-72B (self-hosted volume)   │
└─────────────────────────────────────────────────────────────┘
```

---

*Architecture designed February 2026. Updated with Rust-first infrastructure decision. Based on research compiled in RESEARCH.md. All technology choices are production-ready as of this date. Architecture will evolve as the project progresses and I learn more.*
