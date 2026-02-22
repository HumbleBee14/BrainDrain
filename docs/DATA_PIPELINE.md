# BrainDrain — Data Pipeline: Complete Code Flow

> Detailed technical documentation of the data processing and synthetic data generation pipeline.
> Covers every stage from file upload through training-ready dataset creation.

---

## Pipeline Overview

```
User uploads file
       │
       ▼
┌──────────────┐     ┌───────────────────┐     ┌──────────────────────┐
│  Rust API    │────▶│  S3 (MinIO)       │     │  PostgreSQL          │
│  /documents  │     │  uploads/...      │     │  documents table     │
└──────┬───────┘     └───────────────────┘     └──────────────────────┘
       │ triggers
       ▼
┌──────────────────────────────────────────────────────────────────────┐
│                    Temporal Workflow Engine                          │
│                                                                      │
│  IngestWorkflow                     RefineWorkflow                   │
│  ┌────────────────┐                 ┌───────────────────────┐       │
│  │ get_document_  │                 │ Stage 1: chunk_text   │       │
│  │ info           │                 │         │              │       │
│  │     │          │                 │         ▼              │       │
│  │     ▼          │                 │ Stage 2: generate_    │       │
│  │ parse_document │                 │   synthetic_pairs     │       │
│  └────────────────┘                 │         │              │       │
│                                     │         ▼              │       │
│                                     │ Stage 3: build_       │       │
│                                     │   dataset             │       │
│                                     └───────────────────────┘       │
└──────────────────────────────────────────────────────────────────────┘
       │
       ▼
┌──────────────────┐
│  Training-ready  │
│  ChatML dataset  │
│  in S3           │
└──────────────────┘
```

The pipeline runs in **two phases**, each triggered independently:

1. **Ingest** — Parse uploaded documents into structured JSON (`POST /api/v1/projects/{id}/parse`)
2. **Refine** — Generate synthetic training data from parsed documents (`POST /api/v1/projects/{id}/refine`)

Both phases execute as **Temporal durable workflows** with retry policies, heartbeats, and partial failure handling.

---

## Infrastructure Layer

All activities share a common infrastructure container (`InfraContainer`) initialized once at worker startup.

**File:** `apps/workers/src/infra.py`

```python
class InfraContainer:
    s3: ObjectStore          # boto3 S3 client (MinIO-compatible)
    db: asyncpg.Pool         # PostgreSQL connection pool (2-10 connections)
    redis: aioredis.Redis    # Redis client for streaming/caching
    settings: WorkerSettings # Configuration from env vars
    circuit_breaker: CircuitBreakerPolicy  # LLM API resilience
```

**Initialization flow:**
```
Worker starts
  → init_container(settings)
    → boto3.client("s3", endpoint_url=settings.s3_endpoint)
    → asyncpg.create_pool(settings.database_url, min_size=2, max_size=10)
    → aioredis.from_url(settings.redis_url)
    → create_circuit_breaker("llm-api", fail_max=5, reset_timeout=30)
  → Module-level _container stored (singleton)
  → Activities receive InfraContainer via constructor injection
```

---

## S3 Path Convention

**File:** `apps/workers/src/s3_paths.py`

All paths are **tenant-scoped** for multi-tenancy isolation. The Rust API uses matching path builders (`crates/shared/src/s3_paths.rs`).

| Stage | Path Pattern | Format |
|-------|-------------|--------|
| Upload | `uploads/{tenant_id}/{project_id}/{file_id}.{ext}` | Raw file (PDF, DOCX, etc.) |
| Parsed | `parsed/{tenant_id}/{project_id}/{doc_id}.json` | Structured JSON |
| Chunks | `chunks/{tenant_id}/{project_id}/{batch_id}.jsonl` | JSONL (one chunk per line) |
| Pairs | `pairs/{tenant_id}/{project_id}/{batch_id}.jsonl` | JSONL (one pair per line) |
| Dataset | `datasets/{tenant_id}/{project_id}/{dataset_id}.jsonl` | JSONL (ChatML format) |
| Dataset (val) | `datasets/{tenant_id}/{project_id}/{dataset_id}_val.jsonl` | JSONL (validation split) |
| Adapters | `adapters/{tenant_id}/{model_id}/` | LoRA weights directory |
| Checkpoints | `checkpoints/{tenant_id}/{training_id}/` | Training checkpoints |
| Exports | `exports/{tenant_id}/{model_id}/{filename}` | GGUF / exported models |

---

## Phase 1: Ingest (Document Parsing)

### Workflow: IngestWorkflow

**File:** `apps/workers/src/workflows/ingest.py`

**Trigger:** `POST /api/v1/projects/{project_id}/parse` (Rust API starts Temporal workflow)

**Input:** `(tenant_id, project_id, document_ids: list[str])`

**Flow:**
```
For each document_id in document_ids:
  1. execute_activity("get_document_info", doc_id)
     → Returns: DocumentInfo { storage_path, mime_type, status }
     → Timeout: 30 seconds

  2. Skip if status == "parsed" (idempotency)

  3. execute_activity("parse_document", ParseDocumentInput { ... })
     → Timeout: 10 minutes
     → Retry: 3 attempts
     → Heartbeat: 2 minutes

  4. On failure: log error, add to failures list, continue to next doc
```

**Output:**
```json
{
  "project_id": "uuid",
  "documents_processed": 5,
  "documents_failed": 1,
  "failures": [{ "doc_id": "uuid", "error": "..." }]
}
```

**Key design:** Partial failure tolerance. If 1 of 10 documents fails, the other 9 still get parsed. The workflow returns a summary of successes and failures.

---

### Activity: parse_document

**File:** `apps/workers/src/activities/parse_document.py` (456 lines)

**Activity name:** `"parse_document"`

**Input:**
```python
@dataclass
class ParseDocumentInput:
    tenant_id: str
    project_id: str
    document_id: str
    storage_path: str    # S3 key of the raw uploaded file
    mime_type: str        # e.g., "application/pdf"
```

**Output:**
```python
@dataclass
class ParseDocumentOutput:
    page_count: int
    language: str | None    # ISO 639-1 code ("en", "es", etc.)
    parse_quality: float    # 0.0 to 1.0
    parsed_storage_path: str  # S3 key of the structured JSON
```

#### Step-by-Step Flow

```
1. IDEMPOTENCY CHECK
   → SELECT status FROM documents WHERE id = $1
   → If already "parsed", return early (skip re-parsing)

2. STATUS UPDATE
   → UPDATE documents SET status = 'parsing' WHERE id = $1

3. DOWNLOAD RAW FILE
   → s3.get_object(Bucket, Key=storage_path)
   → heartbeat("downloading")
   → Returns: raw_bytes

4. PARSE (route to parser by mime_type)
   → parser = get_parser(mime_type, storage_path)
   → pages = parser.parse(raw_bytes)
   → heartbeat("parsing")

5. LANGUAGE DETECTION
   → full_text = " ".join(page["text"] for page in pages)
   → language = langdetect.detect(full_text[:5000])
   → Returns ISO 639-1 code or None

6. QUALITY SCORING
   → quality = _compute_quality(pages, len(raw_bytes))
   → Heuristic: average of density + structure + encoding scores

7. BUILD STRUCTURED OUTPUT
   → JSON with: version, doc_id, parser name, page_count, language, quality, pages

8. UPLOAD TO S3
   → s3.put_object(Key=parsed/{tenant_id}/{project_id}/{doc_id}.json)
   → heartbeat("uploading_result")

9. UPDATE DATABASE
   → UPDATE documents SET status='parsed', parse_quality=X, page_count=N, language='en'

10. ON ERROR
    → UPDATE documents SET status='failed', error_message=str(e)[:500]
    → Re-raise exception (Temporal will retry per RetryPolicy)
```

#### Parser System

The parser uses a **Protocol + Registry** pattern, allowing new formats to be added without modifying existing code.

**Protocol:**
```python
class DocumentParser(Protocol):
    @property
    def name(self) -> str: ...
    def can_parse(self, mime_type: str, storage_path: str) -> bool: ...
    def parse(self, raw_bytes: bytes) -> list[dict]: ...
```

**Registry:** Parsers are registered in order. `get_parser()` returns the first parser where `can_parse()` returns `True`. `PlainTextParser` is registered last as the universal fallback.

**6 Parsers:**

| Parser | Library | MIME Type / Extension | Capabilities |
|--------|---------|----------------------|--------------|
| `PdfParser` | PyMuPDF (fitz) | `application/pdf`, `.pdf` | Page-level extraction, heading detection (font size > 14pt), structured sections |
| `DocxParser` | python-docx | `application/vnd.openxmlformats-...`, `.docx` | Paragraph extraction, heading level detection (Word styles), table extraction |
| `HtmlParser` | BeautifulSoup | `text/html`, `.html` | Strips `<script>`/`<style>`, extracts h1-h6, p, li, td elements |
| `MarkdownParser` | markdown + BS4 | `text/markdown`, `.md` | Converts MD→HTML, then delegates to `HtmlParser` (composition pattern) |
| `CsvParser` | csv (stdlib) | `text/csv`, `.csv` | Extracts headers + rows into tabular structure |
| `PlainTextParser` | — | `text/*`, `.txt`, **fallback** | Splits by double newlines into paragraphs |

**Registration order matters:**
```python
_PDF_PARSER = register_parser(PdfParser())
_DOCX_PARSER = register_parser(DocxParser())
_HTML_PARSER = register_parser(HtmlParser())
_MARKDOWN_PARSER = register_parser(MarkdownParser(_html_parser_instance))
_CSV_PARSER = register_parser(CsvParser())
_PLAIN_TEXT_PARSER = register_parser(PlainTextParser())  # MUST be last
```

#### Parsed Output Format

Each parser returns a `list[dict]` of pages:

```json
[
  {
    "page_num": 1,
    "text": "Full text content of the page...",
    "sections": [
      { "type": "heading", "level": 1, "content": "Chapter Title" },
      { "type": "paragraph", "content": "Body text here..." },
      { "type": "table", "headers": ["Col1", "Col2"], "rows": [["a", "b"]] }
    ]
  }
]
```

The final structured JSON stored in S3:

```json
{
  "version": "1.0",
  "doc_id": "uuid",
  "parser": "pymupdf",
  "page_count": 42,
  "language": "en",
  "parse_quality": 0.87,
  "pages": [ ... ]
}
```

#### Quality Score Algorithm

**Function:** `_compute_quality(pages, original_size) → float`

Three components averaged equally:

1. **Text density** (0.0–1.0): `min(chars_per_page / 400, 1.0)` — expects ~400 chars per page
2. **Structure detection** (0.5 or 1.0): 1.0 if any heading found, 0.5 otherwise
3. **Encoding quality** (0.0–1.0): `max(0, 1 - replacement_char_ratio × 10)` — penalizes Unicode replacement characters (`U+FFFD`)

**Example:** A 10-page PDF with 4000 chars and headings but no encoding issues → density: 1.0, structure: 1.0, encoding: 1.0 → quality: **1.0**

#### Language Detection

**Function:** `_detect_language(text) → str | None`

- Uses `langdetect` library (port of Google's language-detection)
- Requires minimum 20 characters
- Samples first 5000 characters for efficiency
- Returns ISO 639-1 code ("en", "es", "fr", "de", etc.) or `None` on failure
- Stored in `documents.language` column

---

## Phase 2: Refine (Synthetic Data Generation)

### Workflow: RefineWorkflow

**File:** `apps/workers/src/workflows/refine.py`

**Trigger:** `POST /api/v1/projects/{project_id}/refine` (Rust API starts Temporal workflow)

**Input:** `(tenant_id, project_id, document_ids, task_type, config)`

**Config options:**
```python
config = {
    "chunk_size": 1500,       # target characters per chunk
    "overlap": 200,            # overlap between consecutive chunks
    "pairs_per_chunk": 5,      # Q&A pairs to generate per chunk
    "system_prompt": "",       # custom system prompt for ChatML dataset
}
```

**3-Stage Pipeline:**

```
Stage 1: chunk_text
  Input:  document_ids + chunk_size/overlap config
  Output: chunks JSONL in S3
  Timeout: 10 minutes | Retries: 3
           │
           ▼ (early exit if chunk_count == 0)

Stage 2: generate_synthetic_pairs
  Input:  chunks_storage_path + task_type + pairs_per_chunk
  Output: pairs JSONL in S3
  Timeout: 30 minutes | Retries: 2 | Heartbeat: 5 minutes
           │
           ▼ (early exit if pair_count == 0)

Stage 3: build_dataset
  Input:  pairs_storage_path + dataset_id + system_prompt
  Output: ChatML JSONL in S3 + DB record
  Timeout: 15 minutes | Retries: 2
```

Each stage writes its output to S3 and passes the storage path to the next stage. Stages are sequential — each depends on the previous stage's output.

---

### Stage 1: Chunk Text

**File:** `apps/workers/src/activities/chunk_text.py` (139 lines)

**Activity name:** `"chunk_text"`

**Input:**
```python
@dataclass
class ChunkTextInput:
    tenant_id: str
    project_id: str
    document_ids: list[str]
    chunk_size: int = 1500    # target chars per chunk
    overlap: int = 200         # overlap between chunks
```

**Output:**
```python
@dataclass
class ChunkTextOutput:
    chunk_count: int
    chunks_storage_path: str   # S3 key of the JSONL file
```

#### Flow

```
For each document_id:
  1. Download parsed JSON from S3
     → Key: parsed/{tenant_id}/{project_id}/{doc_id}.json
     → heartbeat("chunking {doc_id}")

  2. For each page in parsed_data["pages"]:
     → Extract page["text"]
     → Skip empty pages

  3. Recursive split: _split_text(text, chunk_size, overlap)
     → Returns list of chunk strings

  4. For each chunk, create record:
     {
       "chunk_id": uuid4(),
       "doc_id": doc_id,
       "page_num": page_num,
       "chunk_index": i,
       "text": chunk_content,
       "char_count": len(chunk_content)
     }

Upload all chunks as JSONL:
  → Key: chunks/{tenant_id}/{project_id}/{batch_id}.jsonl
  → batch_id: new uuid4() per workflow run
```

#### Recursive Chunking Algorithm

**Function:** `_split_text(text, chunk_size=1500, overlap=200) → list[str]`

```
1. Base case: if len(text) <= chunk_size → return [text]

2. Split by paragraphs (double newline "\n\n"):
   For each paragraph:
     - If adding it to current chunk stays under chunk_size → append
     - If paragraph itself is > chunk_size → split by sentences:
         - Replace ". " with ".\n" and split on "\n"
         - Accumulate sentences into chunks within chunk_size
     - Otherwise → finalize current chunk, start new chunk

3. Apply overlap:
   For chunks[1], chunks[2], ...:
     Prepend the last `overlap` characters of the previous chunk
```

**Example with defaults (chunk_size=1500, overlap=200):**

```
Original text: 4000 chars, 3 paragraphs (1200 + 1800 + 1000 chars)

Step 1: Paragraph 1 (1200 chars) → fits in one chunk → Chunk A
Step 2: Paragraph 2 (1800 chars) → too large for one chunk
  → Split by sentences: Sentence 1 (600) + Sentence 2 (500) = Chunk B (1100 chars)
  → Sentence 3 (700) = Chunk C (700 chars)
Step 3: Paragraph 3 (1000 chars) → Chunk D

After overlap:
  Chunk A: [original]
  Chunk B: [last 200 chars of A] + " " + [Chunk B content]
  Chunk C: [last 200 chars of B] + " " + [Chunk C content]
  Chunk D: [last 200 chars of C] + " " + [Chunk D content]
```

#### Chunk JSONL Format

Each line in the JSONL file:
```json
{
  "chunk_id": "550e8400-e29b-41d4-a716-446655440000",
  "doc_id": "660e8400-e29b-41d4-a716-446655440001",
  "page_num": 3,
  "chunk_index": 0,
  "text": "The recursive chunking algorithm splits text by paragraphs first...",
  "char_count": 1342
}
```

---

### Stage 2: Generate Synthetic Pairs

**File:** `apps/workers/src/activities/generate_pairs.py` (177 lines)

**Activity name:** `"generate_synthetic_pairs"`

This is the most critical and expensive stage — it calls an external LLM API to generate training data from document chunks.

**Input:**
```python
@dataclass
class GenerateSyntheticPairsInput:
    tenant_id: str
    project_id: str
    chunks_storage_path: str   # S3 key from Stage 1
    task_type: str              # "question_answering" | "instruction_following" | "reasoning"
    pairs_per_chunk: int = 5
```

**Output:**
```python
@dataclass
class GenerateSyntheticPairsOutput:
    pair_count: int
    storage_path: str   # S3 key of the pairs JSONL
```

#### LLM Communication Design

**IMPORTANT:** The code is **provider-agnostic**. It uses raw HTTP calls (via `httpx`) to the OpenAI-compatible `/chat/completions` endpoint. There is no OpenAI SDK, no LangChain, no framework dependency.

```python
# From _call_llm() — the core LLM interaction:

url = f"{settings.llm_api_base_url.rstrip('/')}/chat/completions"

resp = await http.post(
    url,
    headers={"Authorization": f"Bearer {settings.llm_api_key}"},
    json={
        "model": settings.llm_model,
        "messages": [{"role": "user", "content": prompt}],
        "max_tokens": settings.llm_max_tokens,
        "temperature": 0.7,
    },
)
```

**Any provider supporting `/chat/completions`** works by changing 3 environment variables:

| Variable | Example (OpenAI) | Example (Groq) | Example (Ollama) |
|----------|-------------------|-----------------|-------------------|
| `APP_LLM_API_BASE_URL` | `https://api.openai.com/v1` | `https://api.groq.com/openai/v1` | `http://localhost:11434/v1` |
| `APP_LLM_API_KEY` | `sk-...` | `gsk_...` | `ollama` |
| `APP_LLM_MODEL` | `gpt-4o-mini` | `llama-3.1-70b-versatile` | `llama3.1` |

#### Circuit Breaker

LLM calls are wrapped in a circuit breaker for resilience:

```python
pairs = await self.infra.circuit_breaker.call(_call_llm, http, settings, prompt)
```

- **Fail threshold:** 5 consecutive failures → circuit opens
- **Reset timeout:** 30 seconds → circuit half-opens, allows one test call
- **Per-chunk failure:** Logged as warning, chunk skipped, pipeline continues

#### Prompt Templates

Three prompt templates exist, selected by `task_type`:

**`question_answering`** (default):
```
You are a training data generator. Given the following text excerpt from a document,
generate {count} diverse question-answer pairs. Each question should be answerable
from the text. Include factual, inferential, and comparative questions.

Text:
{text}

Respond with a JSON array of objects, each with 'question' and 'answer' keys.
Answers should be detailed and grounded in the source text.
```

**`instruction_following`:**
```
You are a training data generator. Given the following text, generate {count}
instruction-response pairs. Instructions should ask to perform tasks related
to the content (summarize, explain, extract, compare, etc.).

Text:
{text}

Respond with a JSON array of objects, each with 'instruction' and 'response' keys.
```

**`reasoning`:**
```
You are a training data generator. Given the following text, generate {count}
complex reasoning scenarios. Each should require analysis, critical thinking,
or multi-step reasoning based on the content.

Text:
{text}

Respond with a JSON array of objects, each with 'question' and 'answer' keys.
Answers should include step-by-step reasoning.
```

#### Flow

```
1. VALIDATE: Check APP_LLM_API_KEY is set (raise ValueError if missing)

2. DOWNLOAD chunks JSONL from S3
   → Parse: one JSON object per line

3. For each chunk:
   a. heartbeat() — keep Temporal informed we're alive
   b. Skip if chunk text < 50 characters
   c. Truncate chunk text to 3000 characters (avoid token overflow)
   d. Format prompt: template.format(count=pairs_per_chunk, text=chunk_text)
   e. Call LLM via circuit breaker:
      → POST /chat/completions
      → Parse JSON response (handles markdown code blocks)
   f. For each pair in LLM response:
      → Normalize keys: "question" → "instruction", "answer" → "response"
      → Attach metadata: doc_id, chunk_id, task_type, source_text preview
   g. On failure: log warning, skip chunk, continue

4. UPLOAD pairs as JSONL
   → Key: pairs/{tenant_id}/{project_id}/{batch_id}.jsonl
```

#### LLM Response Parsing

The LLM returns JSON, but often wraps it in markdown code blocks. The parser handles this:

```python
content = content.strip()
if content.startswith("```"):
    lines = content.split("\n")
    content = "\n".join(lines[1:-1])  # Strip ``` wrapper
return json.loads(content)
```

#### Pair Record Format

Each generated pair:
```json
{
  "id": "uuid",
  "doc_id": "uuid",
  "chunk_id": "uuid",
  "task_type": "question_answering",
  "instruction": "What is the main advantage of recursive chunking?",
  "response": "Recursive chunking preserves semantic boundaries by first splitting on paragraphs, then falling back to sentences only when a paragraph exceeds the target chunk size. This maintains context coherence within each chunk.",
  "source_text": "The recursive chunking algorithm splits text by paragraphs first..."
}
```

#### Cost & Scale

For a typical document:
- 10 pages → ~30 chunks (at 1500 chars/chunk) → 30 LLM calls → ~150 pairs
- At 5 pairs per chunk, each LLM call uses ~500 input tokens + ~1000 output tokens
- Total: ~45,000 tokens ≈ $0.007 with gpt-4o-mini

---

### Stage 3: Build Dataset

**File:** `apps/workers/src/activities/build_dataset.py` (194 lines)

**Activity name:** `"build_dataset"`

This final stage transforms raw pairs into a training-ready dataset with quality filtering, deduplication, ChatML formatting, and train/validation splitting.

**Input:**
```python
@dataclass
class BuildDatasetInput:
    tenant_id: str
    project_id: str
    dataset_id: str              # Pre-generated by RefineWorkflow
    pairs_storage_path: str      # S3 key from Stage 2
    system_prompt: str = ""      # Custom system prompt (default: "You are a helpful assistant.")
```

**Output:**
```python
@dataclass
class BuildDatasetOutput:
    pair_count: int
    storage_path: str   # S3 key of the training dataset
```

#### Flow

```
1. DOWNLOAD raw pairs JSONL from S3

2. QUALITY FILTERING: _filter_pairs(pairs)
   Remove pairs where:
   → instruction is empty
   → response is empty
   → response < 20 characters (too short to be useful)
   → response > 5000 characters (too long, likely noise)
   → instruction < 10 characters (too vague)

3. DEDUPLICATION: _deduplicate(filtered)
   → Hash: MD5(instruction + "|" + response)
   → Keep first occurrence, remove exact duplicates

4. CHATML FORMATTING:
   For each pair, create:
   {
     "messages": [
       {"role": "system", "content": system_prompt},
       {"role": "user", "content": instruction},
       {"role": "assistant", "content": response}
     ],
     "metadata": {
       "doc_id": "...",
       "chunk_id": "...",
       "task_type": "..."
     }
   }

5. TRAIN/VAL SPLIT (90/10):
   → split_idx = max(1, int(len(records) * 0.9))
   → train_records = records[:split_idx]
   → val_records = records[split_idx:]

6. UPLOAD TO S3:
   → Train: datasets/{tenant_id}/{project_id}/{dataset_id}.jsonl
   → Val:   datasets/{tenant_id}/{project_id}/{dataset_id}_val.jsonl

7. CREATE DATABASE RECORD:
   → INSERT INTO datasets (id, tenant_id, project_id, name, format, ...)
   → format: 'chatml'
   → status: 'review_pending'
   → stats: { total_pairs, train_pairs, val_pairs, filtered_out, deduplicated }
```

#### Quality Filtering Rules

| Rule | Threshold | Rationale |
|------|-----------|-----------|
| Empty instruction | Removed | Cannot train on empty input |
| Empty response | Removed | Cannot train on empty output |
| Response < 20 chars | Removed | Too short to be a meaningful response |
| Response > 5000 chars | Removed | Likely noise, LLM rambling, or copy errors |
| Instruction < 10 chars | Removed | Too vague (e.g., "What?" or "Explain") |

#### Deduplication

```python
def _deduplicate(pairs):
    seen = set()
    unique = []
    for pair in pairs:
        content = pair["instruction"] + "|" + pair["response"]
        h = hashlib.md5(content.encode()).hexdigest()
        if h not in seen:
            seen.add(h)
            unique.append(pair)
    return unique
```

This is **exact match** deduplication. Near-duplicate or semantically similar pairs are NOT caught (flagged as a future improvement — see IMPROVEMENTS.md).

#### ChatML Output Format

The final training format (one JSON per line in JSONL):

```json
{
  "messages": [
    {
      "role": "system",
      "content": "You are a helpful assistant."
    },
    {
      "role": "user",
      "content": "What is the main advantage of recursive chunking over fixed-size chunking?"
    },
    {
      "role": "assistant",
      "content": "Recursive chunking preserves semantic boundaries by first attempting to split on paragraph breaks. Only when a paragraph exceeds the target chunk size does it fall back to sentence-level splitting. This means most chunks contain complete thoughts rather than arbitrary text segments, which produces higher-quality training data since the model learns coherent reasoning patterns."
    }
  ],
  "metadata": {
    "doc_id": "660e8400-e29b-41d4-a716-446655440001",
    "chunk_id": "550e8400-e29b-41d4-a716-446655440000",
    "task_type": "question_answering"
  }
}
```

This format is compatible with:
- TRL `SFTTrainer` (used by our training pipeline)
- HuggingFace datasets
- OpenAI fine-tuning API
- Any system expecting ChatML/conversation format

#### Dataset Statistics

The `stats` JSON stored in PostgreSQL:

```json
{
  "total_pairs": 142,
  "train_pairs": 128,
  "val_pairs": 14,
  "filtered_out": 8,
  "deduplicated": 3
}
```

---

## Complete Data Flow Example

Let's trace a concrete example end-to-end.

### Input: A 15-page PDF about machine learning

**Step 1 — Upload:** User uploads `ml_textbook.pdf` (2.3MB) via API.
```
POST /api/v1/projects/{pid}/documents
  → File stored at: uploads/{tid}/{pid}/{fid}.pdf
  → DB: documents row created with status="uploaded"
```

**Step 2 — Parse (IngestWorkflow):**
```
Activity: get_document_info → { storage_path, mime_type: "application/pdf" }
Activity: parse_document
  → PdfParser (PyMuPDF) extracts 15 pages
  → Heading detection: finds 8 headings (font size > 14pt)
  → Language: "en" (langdetect)
  → Quality: 0.83 (good density, has structure, clean encoding)
  → Output stored at: parsed/{tid}/{pid}/{did}.json (285KB)
  → DB: documents.status = "parsed", page_count=15, language="en"
```

**Step 3 — Chunk (RefineWorkflow, Stage 1):**
```
Activity: chunk_text
  → Reads 15 pages from parsed JSON
  → Recursive splitting at 1500 chars with 200 overlap
  → Produces 28 chunks
  → Output stored at: chunks/{tid}/{pid}/{batch}.jsonl
```

**Step 4 — Generate Pairs (RefineWorkflow, Stage 2):**
```
Activity: generate_synthetic_pairs
  → 28 chunks × 5 pairs each = target 140 pairs
  → 2 chunks skipped (text < 50 chars — chapter breaks)
  → 26 LLM calls to gpt-4o-mini
  → 1 call fails (timeout) — skipped with warning
  → 125 pairs generated
  → Output stored at: pairs/{tid}/{pid}/{batch}.jsonl
```

**Step 5 — Build Dataset (RefineWorkflow, Stage 3):**
```
Activity: build_dataset
  → 125 raw pairs
  → Filtering: 6 removed (3 short responses, 2 short instructions, 1 too long)
  → Deduplication: 2 removed (exact duplicates from overlapping chunks)
  → 117 ChatML records created
  → Split: 105 train / 12 validation
  → Output stored at:
      datasets/{tid}/{pid}/{dsid}.jsonl (train)
      datasets/{tid}/{pid}/{dsid}_val.jsonl (val)
  → DB: datasets row created with:
      pair_count=117, format="chatml", status="review_pending"
      stats={"total_pairs":117, "train_pairs":105, "val_pairs":12,
             "filtered_out":6, "deduplicated":2}
```

**Result:** A training-ready ChatML dataset with 105 training examples and 12 validation examples, ready for fine-tuning via the training pipeline.

---

## Error Handling & Resilience

| Component | Failure Mode | Handling |
|-----------|-------------|----------|
| S3 download | Network error, key not found | Temporal retry policy (3 attempts) |
| Document parsing | Corrupt file, unsupported format | PlainTextParser fallback; DB marked "failed" with error message |
| LLM API | Timeout, rate limit, error response | Circuit breaker (5 failures → open); per-chunk skip on failure |
| LLM response | Invalid JSON, unexpected format | Markdown code block stripping; catch-all exception handler |
| DB update | Connection error | Temporal retry; asyncpg connection pool auto-reconnects |
| Workflow timeout | Long-running parse/generation | Configurable timeouts per stage; heartbeats keep Temporal informed |

**Temporal guarantees:**
- **Durable execution:** If the worker crashes mid-activity, Temporal replays the workflow and retries from the failed activity
- **Heartbeats:** Activities send periodic heartbeats; if a heartbeat is missed beyond the timeout, Temporal kills and retries the activity
- **Idempotency:** `parse_document` checks document status before re-parsing; the build_dataset uses `ON CONFLICT DO UPDATE` for the DB insert

---

## Configuration Reference

All configurable via environment variables (`APP_` prefix):

| Variable | Default | Used In |
|----------|---------|---------|
| `APP_LLM_API_BASE_URL` | `https://api.openai.com/v1` | `generate_pairs.py` — LLM endpoint |
| `APP_LLM_API_KEY` | *(required)* | `generate_pairs.py` — Bearer auth |
| `APP_LLM_MODEL` | `gpt-4o-mini` | `generate_pairs.py` — Model selection |
| `APP_LLM_MAX_TOKENS` | `2000` | `generate_pairs.py` — Response length limit |
| `APP_S3_ENDPOINT` | `http://localhost:9000` | All activities — object storage |
| `APP_S3_BUCKET` | `platform` | All activities — bucket name |
| `APP_DATABASE_URL` | `postgresql://...` | `parse_document`, `build_dataset` — status updates |
| `APP_CIRCUIT_BREAKER_ENABLED` | `true` | `generate_pairs.py` — LLM resilience |
| `APP_CIRCUIT_BREAKER_FAIL_MAX` | `5` | Failures before circuit opens |
| `APP_CIRCUIT_BREAKER_RESET_TIMEOUT` | `30` | Seconds before half-open |

**Per-workflow config** (passed in the `config` dict from the API):

| Key | Default | Description |
|-----|---------|-------------|
| `chunk_size` | `1500` | Target characters per chunk |
| `overlap` | `200` | Overlap between consecutive chunks |
| `pairs_per_chunk` | `5` | Q&A pairs to generate per chunk |
| `system_prompt` | `"You are a helpful assistant."` | System message in ChatML output |

---

## File Reference

| File | Lines | Purpose |
|------|-------|---------|
| `apps/workers/src/workflows/ingest.py` | 66 | IngestWorkflow — document parsing orchestration |
| `apps/workers/src/workflows/refine.py` | 88 | RefineWorkflow — chunk → generate → build pipeline |
| `apps/workers/src/activities/parse_document.py` | 456 | Document parsing with 6 format parsers |
| `apps/workers/src/activities/chunk_text.py` | 139 | Recursive text chunking with overlap |
| `apps/workers/src/activities/generate_pairs.py` | 177 | LLM-powered synthetic pair generation |
| `apps/workers/src/activities/build_dataset.py` | 194 | Quality filtering, dedup, ChatML formatting |
| `apps/workers/src/s3_paths.py` | 48 | Tenant-scoped S3 path builders |
| `apps/workers/src/infra.py` | 141 | Infrastructure container (S3, DB, Redis, circuit breaker) |
| `apps/workers/src/config.py` | 93 | Worker configuration (env vars with `APP_` prefix) |

---

*For improvements and future enhancements to this pipeline, see [IMPROVEMENTS.md](./IMPROVEMENTS.md).*
*For the training pipeline (after dataset creation), see the `train_model.py` activity and `training_engine.py` protocols.*
