"""Build dataset activity — assembles pairs into a training-ready dataset.

Delegates quality filtering and deduplication to pluggable backends.
Formats into ChatML and creates train/val split.
"""

import json
import logging
from dataclasses import dataclass
from datetime import UTC, datetime

from temporalio import activity

from src import s3_paths
from src.backends.dataset_filter import (
    get_deduplicator,
    get_filter,
)
from src.infra import InfraContainer
from src.teacher import build_provenance, parse_teacher_config

logger = logging.getLogger("platform.dataset")


@dataclass
class BuildDatasetInput:
    tenant_id: str
    project_id: str
    dataset_id: str
    pairs_storage_path: str
    system_prompt: str = ""
    # Raw golden pairs (from held-out chunks) to format into the companion
    # `<dataset>_golden.jsonl` eval set. Empty = no golden set.
    golden_storage_path: str = ""
    # Distillation: key-stripped teacher block; recorded as write-once
    # provenance in `datasets.config.teacher`.
    teacher: dict | None = None


@dataclass
class BuildDatasetOutput:
    pair_count: int
    storage_path: str
    golden_pair_count: int = 0
    # Id of the datasets row this activity created; "" when nothing was built.
    dataset_id: str = ""


def _to_chat_records(pairs: list[dict], system_prompt: str) -> list[dict]:
    """Format raw instruction/response pairs into chat-format records."""
    records = []
    for pair in pairs:
        instruction = pair.get("instruction", "").strip()
        response_text = pair.get("response", "").strip()
        if not instruction or not response_text:
            continue

        records.append(
            {
                "messages": [
                    {"role": "system", "content": system_prompt},
                    {"role": "user", "content": instruction},
                    {"role": "assistant", "content": response_text},
                ],
                "metadata": {
                    "doc_id": pair.get("doc_id"),
                    "chunk_id": pair.get("chunk_id"),
                    "task_type": pair.get("task_type"),
                },
            }
        )
    return records


class BuildDatasetActivity:
    def __init__(self, infra: InfraContainer):
        self.infra = infra

    @activity.defn(name="build_dataset")
    async def run(self, input: BuildDatasetInput) -> BuildDatasetOutput:
        """Build a ChatML-formatted dataset from generated pairs."""
        s3 = self.infra.s3
        bucket = self.infra.s3_bucket
        db = self.infra.db

        if not input.pairs_storage_path:
            return BuildDatasetOutput(pair_count=0, storage_path="")

        # Download raw pairs
        response = s3.get_object(Bucket=bucket, Key=input.pairs_storage_path)
        raw_data = response["Body"].read().decode("utf-8")
        pairs = [json.loads(line) for line in raw_data.strip().split("\n") if line.strip()]

        if not pairs:
            return BuildDatasetOutput(pair_count=0, storage_path="")

        # Quality filtering (backend-driven)
        pair_filter = get_filter(self.infra.settings.dataset_filter_backend)
        filtered = pair_filter.filter(pairs)

        # Deduplicate (backend-driven)
        deduper = get_deduplicator(self.infra.settings.dedup_backend)
        filtered = deduper.deduplicate(filtered)

        if not filtered:
            return BuildDatasetOutput(pair_count=0, storage_path="")

        # Format into chat records
        system_prompt = input.system_prompt or "You are a helpful assistant."
        chat_records = _to_chat_records(filtered, system_prompt)

        if not chat_records:
            return BuildDatasetOutput(pair_count=0, storage_path="")

        # Train/val split (90/10)
        split_idx = max(1, int(len(chat_records) * 0.9))
        train_records = chat_records[:split_idx]
        val_records = chat_records[split_idx:]

        # Upload dataset
        dataset_key = s3_paths.dataset_path(input.tenant_id, input.project_id, input.dataset_id)

        # Main dataset (train)
        train_lines = [json.dumps(r, ensure_ascii=False) for r in train_records]
        s3.put_object(
            Bucket=bucket,
            Key=dataset_key,
            Body="\n".join(train_lines).encode("utf-8"),
            ContentType="application/jsonl",
        )

        # Validation split
        if val_records:
            val_key = dataset_key.replace(".jsonl", "_val.jsonl")
            val_lines = [json.dumps(r, ensure_ascii=False) for r in val_records]
            s3.put_object(
                Bucket=bucket,
                Key=val_key,
                Body="\n".join(val_lines).encode("utf-8"),
                ContentType="application/jsonl",
            )

        # Golden eval set — pairs generated from chunks the model never trains
        # on, so downstream evaluation measures document knowledge rather than
        # memorization. Written by path convention next to train/val so the
        # evaluation activity can discover it without any schema change.
        golden_records: list[dict] = []
        if input.golden_storage_path:
            golden_resp = s3.get_object(Bucket=bucket, Key=input.golden_storage_path)
            golden_raw = golden_resp["Body"].read().decode("utf-8")
            golden_pairs = [
                json.loads(line) for line in golden_raw.strip().split("\n") if line.strip()
            ]
            golden_records = _to_chat_records(golden_pairs, system_prompt)
            if golden_records:
                golden_key = dataset_key.replace(".jsonl", "_golden.jsonl")
                golden_lines = [json.dumps(r, ensure_ascii=False) for r in golden_records]
                s3.put_object(
                    Bucket=bucket,
                    Key=golden_key,
                    Body="\n".join(golden_lines).encode("utf-8"),
                    ContentType="application/jsonl",
                )

        # Create/update dataset record in DB. Teacher provenance is
        # write-once: a row that already carries `config.teacher` (reserved by
        # the API with provenance, or a previous build) keeps it — a dataset
        # generated by one teacher must never be relabeled as another's.
        teacher_config = parse_teacher_config(input.teacher)
        config_json = json.dumps(
            {"teacher": build_provenance(teacher_config, datetime.now(UTC).isoformat())}
            if teacher_config is not None
            else {}
        )
        await db.execute(
            """
            INSERT INTO datasets (id, tenant_id, project_id, name, format, storage_path,
                                  status, pair_count, stats, config, created_at, updated_at)
            VALUES ($1::uuid, $2::uuid, $3::uuid, $4, 'chatml', $5, 'review_pending',
                    $6, $7::jsonb, $8::jsonb, now(), now())
            ON CONFLICT (id) DO UPDATE SET
                pair_count = $6, storage_path = $5, status = 'review_pending',
                stats = $7::jsonb,
                config = CASE WHEN datasets.config ? 'teacher'
                              THEN datasets.config
                              ELSE datasets.config || EXCLUDED.config END,
                updated_at = now()
            """,
            input.dataset_id,
            input.tenant_id,
            input.project_id,
            f"Dataset for {input.project_id[:8]}",
            dataset_key,
            len(chat_records),
            json.dumps(
                {
                    "total_pairs": len(chat_records),
                    "train_pairs": len(train_records),
                    "val_pairs": len(val_records),
                    "golden_pairs": len(golden_records),
                    "filtered_out": len(pairs) - len(filtered),
                    "deduplicated": len(filtered) - len(chat_records),
                }
            ),
            config_json,
        )

        activity.logger.info(
            "Built dataset %s: %d train / %d val / %d golden pairs (filtered %d)",
            input.dataset_id,
            len(train_records),
            len(val_records),
            len(golden_records),
            len(pairs) - len(filtered),
        )

        return BuildDatasetOutput(
            pair_count=len(chat_records),
            storage_path=dataset_key,
            golden_pair_count=len(golden_records),
            dataset_id=input.dataset_id,
        )
