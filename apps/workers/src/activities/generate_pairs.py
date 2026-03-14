"""Synthetic pair generation activity — creates instruction/response pairs from chunks.

Uses a configurable LLM API (OpenAI-compatible format) to generate
training data from document chunks. LLM provider config is resolved
per-tenant from the database at execution time.
"""

import json
import logging
import uuid
from dataclasses import dataclass

import httpx
from temporalio import activity

from src import s3_paths
from src.backends.llm_provider import get as get_llm_provider
from src.backends.llm_provider import parse_pairs_json
from src.infra import InfraContainer
from src.tenant_config import get_tenant_llm_config

logger = logging.getLogger("platform.generate")

# Prompt templates per task type
PROMPTS = {
    "question_answering": (
        "You are a training data generator. Given the following text excerpt from a document, "
        "generate {count} diverse question-answer pairs. Each question should be answerable "
        "from the text. Include factual, inferential, and comparative questions.\n\n"
        "Text:\n{text}\n\n"
        "Respond with a JSON array of objects, each with 'question' and 'answer' keys. "
        "Answers should be detailed and grounded in the source text."
    ),
    "instruction_following": (
        "You are a training data generator. Given the following text, generate {count} "
        "instruction-response pairs. Instructions should ask to perform tasks related "
        "to the content (summarize, explain, extract, compare, etc.).\n\n"
        "Text:\n{text}\n\n"
        "Respond with a JSON array of objects, each with 'instruction' and 'response' keys."
    ),
    "reasoning": (
        "You are a training data generator. Given the following text, generate {count} "
        "complex reasoning scenarios. Each should require analysis, critical thinking, "
        "or multi-step reasoning based on the content.\n\n"
        "Text:\n{text}\n\n"
        "Respond with a JSON array of objects, each with 'question' and 'answer' keys. "
        "Answers should include step-by-step reasoning."
    ),
}

DEFAULT_PROMPT = PROMPTS["question_answering"]


@dataclass
class GenerateSyntheticPairsInput:
    tenant_id: str
    project_id: str
    chunks_storage_path: str
    task_type: str
    pairs_per_chunk: int = 5


@dataclass
class GenerateSyntheticPairsOutput:
    pair_count: int
    storage_path: str


class GeneratePairsActivity:
    def __init__(self, infra: InfraContainer):
        self.infra = infra

    @activity.defn(name="generate_synthetic_pairs")
    async def run(self, input: GenerateSyntheticPairsInput) -> GenerateSyntheticPairsOutput:
        """Generate instruction/response pairs from chunked text using LLM API."""
        s3 = self.infra.s3
        bucket = self.infra.s3_bucket

        # Resolve LLM config for this tenant (DB lookup, falls back to env var defaults)
        llm_config = await get_tenant_llm_config(
            db=self.infra.db,
            tenant_id=input.tenant_id,
            default_api_base_url=self.infra.settings.llm_api_base_url,
            default_api_key=self.infra.settings.llm_api_key,
            default_model=self.infra.settings.llm_model,
            default_max_tokens=self.infra.settings.llm_max_tokens,
        )

        if not llm_config.api_key:
            raise ValueError(
                "LLM API key not configured. Set per-tenant LLM settings via "
                "PUT /api/v1/settings/llm or set APP_LLM_API_KEY environment variable."
            )

        if llm_config.is_custom:
            logger.info(
                "Using tenant-specific LLM config for %s (provider: %s, model: %s)",
                input.tenant_id,
                llm_config.api_base_url,
                llm_config.model,
            )

        # Download chunks
        if not input.chunks_storage_path:
            return GenerateSyntheticPairsOutput(pair_count=0, storage_path="")

        response = s3.get_object(Bucket=bucket, Key=input.chunks_storage_path)
        chunks_data = response["Body"].read().decode("utf-8")
        chunks = [json.loads(line) for line in chunks_data.strip().split("\n") if line.strip()]

        if not chunks:
            return GenerateSyntheticPairsOutput(pair_count=0, storage_path="")

        # Select prompt template
        prompt_template = PROMPTS.get(input.task_type, DEFAULT_PROMPT)

        provider = get_llm_provider(self.infra.settings.llm_provider_backend)

        all_pairs = []
        async with httpx.AsyncClient(timeout=120.0) as http:
            for chunk in chunks:
                activity.heartbeat()
                chunk_text = chunk.get("text", "")
                if len(chunk_text) < 50:
                    continue

                prompt = prompt_template.format(
                    count=input.pairs_per_chunk,
                    text=chunk_text[:3000],  # limit context to avoid token overflow
                )

                try:
                    raw = await self.infra.circuit_breaker.call(
                        provider.generate,
                        http,
                        prompt,
                        model=llm_config.model,
                        api_base_url=llm_config.api_base_url,
                        api_key=llm_config.api_key,
                        max_tokens=llm_config.max_tokens,
                    )
                    pairs = parse_pairs_json(raw)
                    for pair in pairs:
                        all_pairs.append(
                            {
                                "id": str(uuid.uuid4()),
                                "doc_id": chunk.get("doc_id"),
                                "chunk_id": chunk.get("chunk_id"),
                                "task_type": input.task_type,
                                "instruction": pair.get("question") or pair.get("instruction", ""),
                                "response": pair.get("answer") or pair.get("response", ""),
                                "source_text": chunk_text[:500],
                            }
                        )
                except Exception as e:
                    activity.logger.warning(
                        "LLM call failed for chunk %s: %s", chunk.get("chunk_id"), e
                    )
                    continue

        if not all_pairs:
            return GenerateSyntheticPairsOutput(pair_count=0, storage_path="")

        # Upload pairs as JSONL
        batch_id = str(uuid.uuid4())
        pairs_key = s3_paths.pairs_path(input.tenant_id, input.project_id, batch_id)
        lines = [json.dumps(p, ensure_ascii=False) for p in all_pairs]
        s3.put_object(
            Bucket=bucket,
            Key=pairs_key,
            Body="\n".join(lines).encode("utf-8"),
            ContentType="application/jsonl",
        )

        activity.logger.info("Generated %d pairs from %d chunks", len(all_pairs), len(chunks))
        return GenerateSyntheticPairsOutput(pair_count=len(all_pairs), storage_path=pairs_key)


