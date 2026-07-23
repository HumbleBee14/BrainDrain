# Platform — Deep Research & Technical Notes (February 2026)

> **This is landscape/market research compiled at project inception
> (February 2026) to inform design decisions** — it surveys third-party
> tools and providers (MinerU, distilabel, RunPod, etc.) that were evaluated,
> not all of which were ultimately adopted. It is not a description of what
> was built. For the as-built stack, see
> [SYSTEM_ARCHITECTURE.md](./SYSTEM_ARCHITECTURE.md),
> [PROJECT_FLOW.md](./PROJECT_FLOW.md), and
> [CLOUD_GPU_TRAINING.md](./CLOUD_GPU_TRAINING.md).

## Project Goal
A learning project to explore the full pipeline of fine-tuning LLMs end-to-end — from raw document ingestion through data curation, training, evaluation, and deployment. The goal is to deeply understand each stage by building it: Rust for infrastructure, Python for ML, and everything in between.

---

# PART 1: THE FINE-TUNING LANDSCAPE (2025-2026)

## 1.1 Fine-Tuning Techniques — What's Production-Ready

| Technique | Status | Trainable Params | VRAM Savings | Key Tradeoff |
|-----------|--------|-----------------|--------------|--------------|
| **LoRA** | Production | ~1-2% of base | ~60-70% | Industry default. Best balance of quality vs efficiency |
| **QLoRA** | Production | ~1-2% + 4-bit quant | ~80-90% | Slight quality loss from quantization; enables 70B on 48GB |
| **DoRA** | Production | Similar to LoRA | Similar | More robust to rank choice than LoRA, better at low ranks |
| **Full Fine-Tuning** | Production | 100% | None | Best quality ceiling but requires massive compute |
| **GaLore** | Experimental | Full-parameter learning | ~65% optimizer mem | Enables 7B pre-training on 24GB consumer GPU; not yet widely adopted |
| **OFT/BOFT** | Experimental | Orthogonal transforms | 3x less than LoRA | Stronger generalization, stabler training; still maturing |

**Reality:** LoRA and QLoRA dominate production fine-tuning in 2025-2026. They give 80-90% of the quality of full fine-tuning at a fraction of the cost.

## 1.2 Alignment / Preference Optimization Methods

| Method | Status | Key Property | Best For |
|--------|--------|-------------|----------|
| **SFT** | Production staple | Learns from (input, output) pairs | Domain adaptation, instruction following |
| **DPO** | Production | No reward model needed | Final polish, preference alignment |
| **SimPO** | Production | Reference-free; stabler gradients | Outperforms DPO by 6.4 pts on AlpacaEval 2 |
| **ORPO** | Production | Joint SFT + preference | Single-stage training, simpler pipeline |
| **KTO** | Production | Works with non-paired preference data | When you only have thumbs-up/down |
| **GRPO** | Rapidly maturing | No critic/value model | Reasoning tasks; used to train DeepSeek-R1 |
| **RLHF/PPO** | Production but complex | Full RL loop with reward model | Maximum control, but high complexity |

**The practical pipeline in 2025-2026:** Most teams do **SFT first**, then optionally **DPO or SimPO** for preference alignment. **GRPO** is hot for reasoning models. RLHF/PPO is seen as unnecessarily complex for most use cases.

## 1.3 Training Frameworks

### Tier 1: Production-Grade, Most Active

**Unsloth** — The speed champion
- 2-5x faster training, 70-80% less VRAM, zero accuracy degradation
- Trains Qwen3-4B on just 3.9GB VRAM
- Custom Triton kernels for RoPE + MLP
- Best for: Single-GPU fine-tuning, rapid prototyping

**Hugging Face TRL** — The standard library
- SFTTrainer, DPOTrainer, GRPOTrainer, RewardTrainer, ORPOTrainer, KTOTrainer
- Scales from single GPU to multi-node via Accelerate/DeepSpeed
- Best for: Maximum flexibility and ecosystem integration

**LLaMA-Factory** — The zero-code option
- 100+ model support, web UI + CLI, ACL 2024 paper
- Supports SFT, DPO, KTO, ORPO, PPO, GRPO
- Best for: Lowest barrier to entry; non-ML-engineer users

### Tier 2: Solid

- **Axolotl** — YAML-based configs, great community, multi-GPU
- **torchtune** — PyTorch official, backed by Meta, extensible

### Tier 3: Enterprise/Specialized

- **NVIDIA NeMo** — Enterprise-grade, Kubernetes-native, REST API microservices
- **MLX (Apple)** — Apple Silicon only, local fine-tuning on Mac

## 1.4 Base Models for Fine-Tuning

| Model Family | Sizes | License | Fine-Tuning Popularity | Key Strength |
|-------------|-------|---------|----------------------|--------------|
| **Llama 4** | Scout, Maverick (MoE) | Apache 2.0 | Very High | Most permissive, huge community |
| **Llama 3.x** | 1B-405B | Llama Community | Very High | Battle-tested, enormous ecosystem |
| **Qwen 3** | Dense + MoE | Apache 2.0 (patent clause) | High & Rising | MoE efficiency |
| **DeepSeek R1** | 1.5B-70B distilled | MIT | High | Best reasoning models |
| **Mistral** | 3B-24B | Apache 2.0 | Moderate-High | 3x faster than Llama 3.3 70B |
| **Gemma 3** | 270M-27B | Gemma License | Moderate-High | Same arch as Gemini 2.0, multimodal |
| **Phi-4** | 3.8B-14B | MIT | Moderate | Best reasoning for size |

**Safest for commercial use:** Apache 2.0 (Llama 4, Qwen 3, Mistral) and MIT (DeepSeek R1, Phi-4).

---

# PART 2: THE DATA CURATION PIPELINE

## 2.1 Document Parsing (Ingestion)

This is the entry point of the entire pipeline. The landscape has matured significantly.

### Tier 1: Best Open-Source Parsers

| Tool | Speed | Accuracy | Best For |
|------|-------|----------|----------|
| **MinerU 2.5** | 2.12 pages/sec (A100) | 90.67 on OmniDocBench | Overall leader. Auto-converts math to LaTeX, tables to HTML |
| **Docling (IBM)** | 1.27 sec/page (M3 Max, no GPU) | Strong structural fidelity | Best for CPU-only / commodity hardware |
| **Marker** | 4.2 sec/page (M3 Mac) | Good | Markdown/JSON/HTML output, all languages |
| **Unstructured.io** | 4.2 sec/page (CPU) | 100% on simple tables | Broadest format support (PDF, DOCX, HTML, images) |

### Tier 2: Specialized

- **Surya OCR** — 90+ languages, good for multilingual corpora
- **Nougat (Meta)** — Purpose-built for academic/scientific PDFs, excellent math handling

### Tier 3: Commercial

- **Reducto** — Vision-first hybrid, ~0.90 table similarity (best), YC-backed
- **LlamaParse** — $0.003/page, 78% edit similarity, cheapest
- **Google Document AI / AWS Textract / Azure Doc Intelligence** — Managed cloud, mature

### Google LangExtract (New, 2025)

Not a PDF parser — sits on top of parsed text. Extracts **structured information with precise source grounding** (exact character offsets back to source). Schema-enforced output. Supports multi-model backends (Gemini, Azure OpenAI, Ollama). Useful as a **hallucination firewall** — verify that generated Q&A pairs trace back to exact document spans.

### Critical Finding

**Domain matters more than parser choice.** An Applied AI study of 800+ docs found accuracy variation of 55+ percentage points by document type: legal contracts hit 95%, academic papers struggle at 40-60%.

**Approach for this project:** MinerU 2.5 (primary) + Docling (CPU fallback) + Nougat (academic). Auto-detect document type and route to best parser.

## 2.2 Chunking Strategies

### For RAG vs Fine-Tuning — Critical Distinction

**For RAG:** Small chunks (256-512 tokens) with overlap. Semantic chunking helps for precise retrieval.

**For fine-tuning:** The question is fundamentally different. You're not retrieving chunks — you're constructing instruction-response pairs. The "chunking" is about:
1. How much source text to feed the teacher LLM when generating Q&A pairs (typically 1,000-4,000 tokens)
2. Preserving document coherence (structure-aware splitting)
3. Coverage (every part of the corpus generates training examples)

### Methods

| Strategy | How It Works | Best For |
|---|---|---|
| **Fixed-size** | Split by token count with overlap | Baseline, simple content |
| **Recursive character** | Hierarchically split by paragraph/sentence | General-purpose (LangChain default) |
| **Semantic chunking** | Embed sentences, merge by similarity score | Dense, unstructured text |
| **Document-structure-aware** | Use headings/sections/tables as boundaries | Structured docs (legal, academic) |
| **Vision-guided** | Use document vision models for boundaries | Complex layouts (2025, GPU-intensive) |

### Recent Research (2025)

- **Structure-Aware Chunking for Legal Docs (ACL 2025):** Naive sequential chunking causes "context fragmentation." Partitioning by rhetorical strata (Facts, Arguments, Conclusion) significantly improves coherence.
- **Semantic-Structural Synergistic Chunking (2025):** Combines dynamic semantic unit identification with document organization reconstruction.
- **Structure-aware Domain Knowledge Injection:** Treats each chunk as a "knowledge point" and extracts domain taxonomy.

**Recommendation:** Document-structure-aware chunking as default (leveraging layout detection from MinerU/Docling), semantic chunking as fallback for unstructured text, fixed-size as baseline option. Default 1,500-2,000 tokens for fine-tuning synthesis.

## 2.3 Synthetic Data Generation

### Approaches (Ranked by Sophistication)

**1. Alpaca-Style (Simple Distillation)** — OUTDATED
- Teacher LLM generates instruction-response pairs from seed examples
- Low diversity, repetitive patterns

**2. Self-Instruct** — FOUNDATION
- Model generates its own instructions from seed tasks
- Better diversity but limited by seed distribution

**3. Evol-Instruct (WizardLM)** — PRODUCTION-READY
- Rewrites instructions using in-depth (add constraints, deeper reasoning) and in-breadth (topic diversity) strategies
- WizardCoder surpassed Claude and Bard on HumanEval using this

**4. Orca-Style (Explanation Traces)** — PRODUCTION-READY FOR REASONING
- Trains on detailed reasoning traces, not just input-output pairs
- Teaches small models *how to reason*

**5. Phi-Style (Textbook Generation)** — STATE-OF-THE-ART FOR SLMs
- Generates "textbook-like" synthetic data for teaching
- Quality >>> quantity. Phi-3-mini (3.8B) outperforms models 2x its size

**6. AgentInstruct (Microsoft, 2024-2025)** — CURRENT STATE OF THE ART
- Multi-agent agentic workflows to generate data from **raw documents**
- 100+ subcategories for diversity
- Does NOT require seed prompts — uses raw documents as seeds
- Results: 40% improvement on AGIEval, 54% on GSM8K, 45% on AlpacaEval
- **Most relevant to a "documents to training data" pipeline**

### Production Frameworks

- **Distilabel (Argilla/HuggingFace)** — Best open-source option. Pipeline architecture, implements Evol-Instruct, UltraFeedback, etc.
- **NeMo Curator/Data Designer (NVIDIA)** — GPU-accelerated, operates at ~100K sample scale
- **HuggingFace Synthetic Data Generator** — No-code UI, powered by distilabel

### Critical Risk: Model Collapse

By April 2025, **over 74% of new webpages contained AI-generated text**. Training on synthetic data recursively leads to model collapse.

**Mitigations:**
- Always maintain a real-data anchor (synthetic augments, not replaces)
- Cap synthetic data ratios
- As little as 10% contamination with recursive synthetic data causes degradation
- Use diverse teacher models

## 2.4 Data Quality & Deduplication

### Deduplication Methods

| Method | Type | Scale | Speed |
|---|---|---|---|
| **Exact hash (MD5/SHA)** | Exact duplicate | Unlimited | Fastest |
| **MinHash + LSH** | Near-duplicate | Trillion-scale | Fast (sublinear) |
| **SemDeDup** | Semantic duplicate | Moderate | Slow (needs embeddings) |
| **Hybrid** | Both | Large | Medium |

### Quality Filtering (Before Training)

1. **Perplexity-Based Filtering** — Measures model prediction confidence. Production-ready.
2. **IFD Score** — Measures instructional difficulty. Higher = teaches more. Production-ready.
3. **Reward Model Scoring** — Trained model scores examples. Production-ready if reward model exists.
4. **LLM-as-Judge** — Strong LLM evaluates samples (FineWeb-Edu approach). Dominant in 2025.
5. **Diversity Metrics** — Cluster-based scoring, DCScore. Early production.

### Key Insight

Paper: *"Supervised Fine-Tuning on Curated Data is Reinforcement Learning"* (2025) — demonstrates that **data curation IS the training signal**. The quality of curation directly determines model quality. This is a key insight for anyone building fine-tuning pipelines.

## 2.5 Dataset Formats

**ChatML/messages format is the emerging standard.** All major frameworks support it.

```json
{
  "messages": [
    {"role": "system", "content": "You are a helpful assistant."},
    {"role": "user", "content": "What is..."},
    {"role": "assistant", "content": "It is..."}
  ]
}
```

File format: JSONL is universal. Parquet preferred for large datasets (columnar, compressed, faster loading).

**Recommendation:** Generate ChatML/JSONL as canonical output. Provide conversion to Alpaca/ShareGPT. Parquet export for datasets >1GB.

## 2.6 The "Data Flywheel" Problem — What's Still Manual

The typical journey from "we have docs" to "we have a model":

```
Raw Documents (10TB)
  → [MANUAL] Document triage (which docs are relevant?)
  → [SEMI-AUTO] PDF/DOCX parsing (often fails on scans, tables)
  → [MANUAL] Quality review of parsed output
  → [MANUAL] Domain expert annotation
  → [MANUAL] Prompt engineering for synthetic data
  → [SEMI-AUTO] Synthetic data generation (custom scripts)
  → [MANUAL] Quality review of synthetic data
  → [MANUAL] Format conversion for training framework
  → [SEMI-AUTO] Training
  → [MANUAL] Evaluation
  → Rinse and repeat
```

### What's still largely manual (interesting areas to automate):

1. **Document Triage & Routing** — Deciding which docs are relevant, which parser to use. Currently: human decisions.
2. **Parser Quality Verification** — Checking if tables/math came through. Currently: visual inspection.
3. **Document-to-Training-Objective Mapping** — "What kind of training data should this doc produce?" Currently: domain experts write custom prompts.
4. **Synthesis Quality Loop** — Reviewing generated Q&A for accuracy against source. Currently: human review.
5. **Iterative Data Improvement** — Train → find weaknesses → generate more data for weak areas → retrain. Currently: fully manual.

---

# PART 3: GPU INFRASTRUCTURE & COMPUTE

## 3.1 GPU Supply Situation (2026)

The shortage has **largely resolved**:
- H100 lead times: from 52+ weeks (2023) → readily available on-demand (2025-2026)
- H100 rental prices collapsed **64%** from peak: $8-10/hr → $2.40-3.95/hr
- 300+ new GPU cloud providers in 2025
- A100s are commodity hardware at sub-$1.50/hr
- By mid-2026: H100 expected below $2/hr universally

## 3.2 Serverless GPU Providers

| Provider | H100/hr | A100 80GB/hr | Best For | Notes |
|----------|---------|-------------|----------|-------|
| **Modal** | $3.95 | $2.50 | Training jobs (best DX) | Per-second billing, Python-native |
| **RunPod Pods** | $2.49 | $1.64 | Sustained training | Community cloud, cheapest |
| **RunPod Serverless** | $2.72 | $1.90 | Inference endpoints | FlashBoot <200ms cold starts |
| **Lambda Labs** | $2.49-3.29 | $1.29 | Reserved instances | Pre-configured ML stacks |
| **CoreWeave** | ~$6.16 | ~$3.00 | Multi-node distributed | InfiniBand, enterprise SLAs |
| **Together AI** | N/A | N/A | API-based fine-tuning | $0.48/M tokens (LoRA) |

### Best For Each Use Case

| Use Case | Best Providers |
|---|---|
| **Ephemeral training jobs** | Modal (best DX), RunPod Pods (cheapest) |
| **Large-scale distributed training** | CoreWeave, Lambda Labs |
| **Production inference** | RunPod Serverless, Baseten, Together AI |
| **Fastest inference** | Together AI, Fireworks AI |

## 3.3 VRAM Requirements for QLoRA Fine-Tuning

| Model Size | Min VRAM (QLoRA 4-bit) | Recommended GPU | Cost/hr |
|-----------|----------------------|-----------------|---------|
| **7B** | 12-16 GB | T4 16GB / A10G 24GB | $0.59-1.10 |
| **13B** | 20-24 GB | A10G 24GB / L40S 48GB | $1.10-1.95 |
| **70B** | 40-48 GB | A100 80GB | $2.50 |

## 3.4 Realistic Training Cost

| Scenario | GPU | Time | Cost |
|----------|-----|------|------|
| 7B QLoRA, 10K examples | A10G | 1-3 hrs | **$1-5** |
| 7B QLoRA, 100K examples | A100 | 12-24 hrs | **$15-35** |
| 13B QLoRA, 50K examples | A100 | 8-16 hrs | **$10-22** |
| 70B QLoRA, 50K examples | A100 | 1-3 days | **$35-200** |

**Key insight:** The sweet spot for most use cases is **7B-14B QLoRA, costing $3-35 per run**. This makes fine-tuning very accessible for personal and small-scale projects.

## 3.5 Model Serving & LoRA Adapter Serving

### Serving Frameworks

| Framework | Best For | LoRA Support |
|-----------|---------|-------------|
| **vLLM** | General production (de facto standard) | Yes (S-LoRA: 2,000+ adapters simultaneously) |
| **TGI v3** | HuggingFace ecosystem, long-context | Yes |
| **LoRAX (Predibase)** | Multi-tenant LoRA serving | Excellent (core feature, 60+ adapters) |
| **TensorRT-LLM** | Maximum throughput on Nvidia | Limited |
| **llama.cpp / Ollama** | Local/edge inference | Basic |

### Multi-Tenant LoRA Serving

**vLLM with S-LoRA** or **LoRAX**: Load ONE base model (~14GB for 7B), serve HUNDREDS of LoRA adapters (~10-50MB each) from the same GPU. This is how multi-adapter serving works economically.

Cost: One H100 running vLLM = **~$3,000/month** serving 100+ adapters = **~$30/adapter/month** infra cost.

## 3.6 Deployment Options for End Users

| Option | Cold Start | Monthly Cost | Best For |
|--------|-----------|-------------|----------|
| **Multi-tenant serverless** (shared vLLM) | <100ms (warm) | $5-50/adapter | Shared inference |
| **Dedicated serverless** (scale-to-zero) | 2-30 sec | $50-200/model | Dedicated inference |
| **Dedicated always-on** | None | $420-2,850 | Always-on inference |
| **Edge download (GGUF/ONNX)** | Local | One-time fee | Self-hosted |

## 3.7 RLHF/DPO for Non-Technical Users

**Start with DPO.** Zero additional infrastructure needed beyond existing training pipeline.

User flow:
1. Fine-tune model (SFT) with uploaded data
2. Test model, identify bad responses
3. UI lets them mark good/bad → creates preference pairs
4. Click "Align model" → DPO training runs on same infra
5. Cost: Same as an additional training run ($1-50)

No reward model. No RL complexity. Just binary feedback.

---

# PART 4: EXISTING TOOLS & PLATFORMS

## 4.1 Fine-Tuning Platforms (What's Already Out There)

### OpenAI Fine-Tuning API
- **What:** API-only fine-tuning for GPT-4o, GPT-4o-mini, GPT-3.5-turbo
- **Pricing:** $25/M training tokens (GPT-4o), $3/M (GPT-4o-mini)
- **Limitations:** No data curation. No synthetic data. No evaluation. Locked to OpenAI models only. No open-source model support. Black box.

### Google Vertex AI Model Tuning
- **What:** Fine-tune Gemini models via Google Cloud
- **Limitations:** Locked to Google ecosystem. No data curation. Enterprise-heavy pricing.

### Amazon Bedrock Custom Models
- **What:** Fine-tune Llama, Mistral, etc. on AWS
- **Limitations:** Complex setup. No data curation pipeline. AWS lock-in.

### Together AI
- **What:** Fine-tuning API + inference. $0.48/M tokens (LoRA <=16B)
- **Limitations:** API-only, no UI for data prep. Assumes clean JSONL input.

### Fireworks AI
- **What:** Fast inference + fine-tuning API
- **Limitations:** Developer-focused, no data curation, no evaluation suite.

### H2O LLM Studio
- **What:** GUI for fine-tuning with hyperparameter tuning
- **Limitations:** Feels like Excel for Data Scientists. No agentic data cleaning. Assumes you already have clean data. Does NOT ingest raw documents.

### Predibase / Ludwig
- **What:** Declarative ML with LoRAX multi-tenant serving
- **Strengths:** Best multi-LoRA serving (60+ adapters on one GPU)
- **Limitations:** Developer-focused. No document ingestion. No synthetic data.

### MonsterAPI
- **What:** No-code fine-tuning with pre-built templates
- **Limitations:** Limited model selection. Basic evaluation. No data curation.

### Lamini
- **What:** Enterprise fine-tuning platform
- **Limitations:** Focused on enterprise contracts. No self-serve data pipeline.

### LLaMA-Factory (Open Source)
- **What:** Zero-code web UI for fine-tuning 100+ models
- **Strengths:** Broadest model support, YAML/web config
- **Limitations:** No data curation. No document ingestion. No deployment. Just the training step.

### HuggingFace AutoTrain
- **What:** Automated training with minimal config
- **Limitations:** No data curation. Limited evaluation. Basic UI.

## 4.2 Entry Point AI

- **What:** No-code fine-tuning platform across multiple providers (OpenAI, AI21, Replicate, Anthropic, Groq, Gemini)
- **Pricing:** $490/yr (Starter, 5K examples) → $990/yr (Growth, 25K) → $2,490/yr (Pro, 100K)
- **Strengths:** Unified interface across providers. Templating engine. Some synthetic data generation. Single-click deployment.
- **Limitations:**
  - Does NOT handle raw document ingestion (expects structured data input)
  - Synthetic data generation is basic (not agentic, no AgentInstruct-style workflows)
  - No automated data quality scoring / hallucination detection
  - No source grounding verification
  - Not listed among top enterprise picks for 2026
  - Training costs on external platforms are extra (not included)
  - Limited to models from connected providers (no self-hosted open-source training)
  - No RLHF/DPO support visible
  - Small team, limited scale

EntryPoint AI is the closest thing to an end-to-end fine-tuning UI, but it's essentially a **UI wrapper around provider fine-tuning APIs** rather than a full data pipeline. It doesn't handle the hard part (raw docs → quality training data).

## 4.3 Data Curation Tools

### Scale AI / Labelbox
- **What:** Human-in-the-loop annotation at scale
- **Limitation:** Slow, expensive, data leaves your infrastructure, not automated

### Argilla
- **What:** Open-source data curation for NLP/LLMs
- **Strength:** Best human feedback collection tool. Integrates with distilabel.
- **Limitation:** Not an end-to-end platform. Just the annotation layer.

### LabelStudio
- **What:** Open-source data labeling
- **Limitation:** General-purpose labeling, not specialized for LLM fine-tuning

### Lilac (Google)
- **What:** Dataset exploration and curation
- **Limitation:** Visualization tool, not a pipeline

## 4.4 MLOps Platforms

| Platform | Fine-Tuning? | Data Curation? | Deployment? |
|----------|-------------|----------------|-------------|
| Weights & Biases | Experiment tracking only | No | No |
| MLflow | Experiment tracking only | No | Basic |
| ClearML | Some pipeline support | No | Basic |
| Neptune.ai | Experiment tracking only | No | No |

**None of these are end-to-end fine-tuning platforms.** They're monitoring/tracking tools.

## 4.5 Coverage Comparison — What Each Tool Handles

Here's how existing tools cover the full pipeline. This helps understand which stages are well-served and which are still DIY:

| Platform | Ingestion | Data Curation | Synthetic Data | Training | Evaluation | Deployment |
|----------|-----------|--------------|----------------|----------|------------|------------|
| OpenAI Fine-Tuning | - | - | - | Yes | Basic | Yes |
| Entry Point AI | - | Basic | Basic | Yes (via providers) | Basic | Yes |
| H2O LLM Studio | - | - | - | Yes | Good | - |
| LLaMA-Factory | - | - | - | Yes | Basic | - |
| Scale AI | - | Yes (human) | - | - | - | - |
| NVIDIA NeMo | - | Yes (GPU) | Yes | Yes | Yes | Yes |
| Databricks | - | Partial | - | Yes | Partial | Yes |
| **Platform (this project)** | **Yes** | **Yes (Agentic)** | **Yes (AgentInstruct)** | **Yes** | **Yes (LLM-as-Judge)** | **Yes** |

NVIDIA NeMo comes closest to full coverage but requires NVIDIA infrastructure buy-in and a team of ML engineers. Databricks requires a data engineering team. The document ingestion → data curation → training pipeline is where the most manual work remains across all tools.

---

# PART 5: UNSOLVED CHALLENGES (The Hard Engineering)

## 5.1 Catastrophic Forgetting — #1 Pain Point

Fine-tuning on domain-specific data **reliably degrades general capabilities**. Larger models forget MORE (counter-intuitive).

**Current mitigations:**
- LoRA (frozen base weights) reduces but doesn't eliminate it
- MIT's Self-Distillation Fine-Tuning preserves prior knowledge but is 4x slower
- RL-based post-training (GRPO/PPO) forgets significantly less than SFT
- Mixing domain data with general instruction data (70/30 or 80/20 ratio)

**What I want to explore:** Auto-mix domain data with general instruction data. Run before/after benchmark comparisons. Warn users when general capability drops.

## 5.2 Data Quality — The Unsexy Bottleneck

The LIMA paper proved **1,000 high-quality examples can match GPT-3 performance**. Data quality >>> quantity. Most fine-tuning failures trace back to bad data, not bad hyperparameters.

**What I want to explore:** Automated quality scoring (perplexity filtering, LLM-as-judge, source grounding verification). This is one of the most impactful parts of the pipeline to automate.

## 5.3 Evaluation — Nobody Agrees How to Measure

Standard benchmarks (MMLU, HellaSwag) often don't correlate with real-world task performance. LLM-as-judge is becoming standard but expensive and has biases.

**What I want to explore:** Domain-specific evaluation suites. A/B testing (base vs fine-tuned). Regression detection. Style alignment scoring.

## 5.4 Hyperparameter Sensitivity

Learning rate, LoRA rank, alpha, target modules, warmup ratio — all interact non-linearly. Small changes can dramatically affect results.

**What I want to explore:** Auto-select hyperparameters based on model size, dataset size, and use case. Use community-proven recipes as defaults.

## 5.5 Model Collapse from Synthetic Data

Training on synthetic data recursively causes output drift. As little as 10% contamination causes measurable degradation.

**What I want to explore:** Always anchor on real data. Track synthetic data ratios. Use diverse teacher models. Watermark synthetic data.

---

# PART 6: RECENT BREAKTHROUGHS (2025-2026)

| Breakthrough | Impact | How I Plan to Use It |
|-------------|--------|----------------------|
| **GRPO (DeepSeek)** | RL training without critic model, 50% less compute than PPO | Implement reasoning fine-tuning mode |
| **Unsloth Triton Kernels** | 3x faster, 30% less VRAM, zero accuracy loss | Training backend — faster, cheaper runs |
| **AgentInstruct (Microsoft)** | Multi-agent synthetic data from raw docs, 40-54% improvement | Core of the data synthesis pipeline |
| **SimPO** | Outperforms DPO by 6.4 pts, simpler, cheaper | Default alignment method |
| **GaLore** | Full-parameter learning on consumer GPUs | Explore full fine-tuning at LoRA cost |
| **MIT Self-Distillation** | Preserves prior knowledge during fine-tuning | Address catastrophic forgetting |
| **"SFT on Curated Data = RL" paper** | Data curation IS the training signal | Key motivation for building the data pipeline |
| **MinerU 2.5** | 1.2B model, 90.67 on OmniDocBench, 2.12 pages/sec | Best document parser for ingestion |
| **Active Synthetic Data Gen (Dec 2025)** | Iterative, closed-loop generation guided by student model | Automate the "retrain on weak areas" loop |

---

# PART 7: ARCHITECTURAL IMPLICATIONS

Based on all research, here are the key architectural decisions for this project:

## The Pipeline (What We Build)

```
[Upload] → [Parse] → [Chunk] → [Synthesize] → [Quality Filter] → [Train] → [Evaluate] → [Deploy]
    |          |          |           |                |              |           |            |
  S3/Upload  MinerU   Semantic   AgentInstruct    LLM-as-Judge   Unsloth     Arena      vLLM/LoRAX
             Docling  Structure  Evol-Instruct    Perplexity     QLoRA     A/B Test    GGUF export
             Nougat   Aware      Orca-Style       MinHash        DPO/SimPO  Regression
```

## Tech Stack Decisions

| Layer | Choice | Why |
|-------|--------|-----|
| **Parsing** | MinerU 2.5 + Docling + Nougat | Best accuracy, speed, coverage |
| **Chunking** | Structure-aware (default) + Semantic (fallback) | Preserves document coherence for synthesis |
| **Synthetic Data** | AgentInstruct-style via distilabel | State-of-the-art, raw-docs-as-seeds, production framework |
| **Quality** | LLM-as-Judge + Perplexity + MinHash + LangExtract | Multi-signal quality assurance |
| **Training** | Unsloth + TRL | 2x speed, 60% less VRAM, supports SFT+DPO+GRPO |
| **Training Infra** | Modal (MVP) → RunPod/K8s (scale) | Best DX for MVP, cheapest at scale |
| **Default GPU** | A10G (7B) / A100 (13B-70B) | Sweet spot of cost vs capability |
| **Serving** | vLLM with S-LoRA | Multi-tenant, 2000+ adapters, OpenAI-compatible API |
| **Edge Export** | GGUF (llama.cpp) + ONNX | Covers consumer + enterprise edge |
| **Alignment** | DPO via TRL/Unsloth | Same infra as training, no RL complexity |
| **Orchestration** | Temporal.io | Durable execution, crash recovery, exactly-once semantics |
| **Storage** | S3 + PostgreSQL + Qdrant/LanceDB | Objects + metadata + vectors |
| **Dataset Format** | ChatML/JSONL (canonical) with Alpaca/ShareGPT conversion | Industry standard |

## Cost Structure (Per User, Estimated)

| Operation | Cost |
|-----------|------|
| Document parsing (1000 pages) | $0.50-2.00 (compute) |
| Synthetic data generation (10K pairs) | $5-20 (LLM API calls) |
| Training 7B QLoRA | $1-5 (GPU) |
| Training 70B QLoRA | $35-200 (GPU) |
| DPO alignment | $1-5 (additional run) |
| Serving (multi-tenant) | ~$30/user/month |
| GGUF export (one-time) | $0.50-2.00 |

---

# PART 8: WHY THIS IS A GREAT LEARNING PROJECT

## What Makes This Worth Building

1. **Covers the full stack.** Raw document ingestion, data curation, synthetic data generation, model training, evaluation, and deployment — every stage teaches different skills and tools.

2. **The timing is right.** GPU costs have collapsed. Training frameworks are mature. The bottleneck has shifted to data quality, which is the most interesting and under-explored part of the pipeline.

3. **Solid research foundation.** The 2025 paper proving "SFT on curated data = RL" means data curation pipelines are where the real learning happens. AgentInstruct proves multi-agent synthetic data from raw docs works at scale.

4. **The engineering is challenging but achievable.** Rust for infrastructure, Temporal for orchestration, Unsloth for training, vLLM for serving — all production-ready building blocks. The challenge is in the **integration and end-to-end automation**.

5. **Teaches production Rust.** Building the API gateway, storage layer, and infrastructure in Rust provides deep systems programming experience with async, concurrency, and real-world service architecture.

## What This Project Aims to Implement

- **Start from raw documents** (not pre-cleaned JSONL)
- **Agentic data curation** pipeline (multi-agent synthesis)
- **Source-grounded quality verification** (hallucination detection)
- **Auto-hyperparameter selection** (no manual tuning needed)
- **Multi-tenant LoRA serving** (efficient adapter management)
- **Catastrophic forgetting detection** (before/after benchmarking)
- **Simple UX** even for non-technical use cases (personal digital twin, email assistant, etc.)

---

# PART 9: LEARNING OBJECTIVES

## What I'm Learning By Building This

### Rust (Production-Grade Infrastructure)
- Async Rust with Tokio runtime and Axum web framework
- Compile-time checked SQL with SQLx
- Zero-copy file streaming and backpressure handling
- Building production REST APIs with middleware, auth, rate limiting
- Working with aws-sdk-rust, redis-rs, and other async ecosystem crates
- Single-binary deployment and minimal memory footprint at scale

### ML / Fine-Tuning Pipeline
- How LoRA, QLoRA, and adapter-based fine-tuning actually work under the hood
- SFT, DPO, GRPO — different training paradigms and when to use each
- Synthetic data generation using multi-agent pipelines (AgentInstruct patterns)
- Data quality scoring: perplexity filtering, LLM-as-Judge, deduplication (MinHash)
- Document parsing at scale (MinerU, Docling, Nougat)
- Chunking strategies and how they affect training data quality

### Model Serving & Deployment
- vLLM internals, S-LoRA for multi-adapter serving
- GGUF quantization and edge model export
- OpenAI-compatible API proxy design
- Streaming inference with SSE

### Systems Engineering
- Temporal.io for durable, long-running workflow orchestration
- Event-driven architecture with Redis Streams
- Multi-tenant isolation patterns (RLS, prefix isolation, collection-per-project)
- GPU provisioning and ephemeral compute (Modal, RunPod)
- Observability: Prometheus, Grafana, OpenTelemetry

### Personal Use Cases I Want to Build With This
- A digital twin that answers messages in my writing style
- An email assistant trained on my past emails
- Domain-specific Q&A models from personal knowledge bases
- Experimenting with reasoning fine-tuning (GRPO) on custom tasks

---

*Research compiled February 2026. Sources include web searches across Modal, RunPod, Lambda Labs, HuggingFace, arXiv papers, NVIDIA documentation, Applied AI benchmarks, and product websites.*
