"""Text chunking activity — splits parsed documents into training-sized chunks.

Delegates to a pluggable ChunkingStrategy backend (default: recursive).
"""

import json
import logging
import uuid
from dataclasses import dataclass

from temporalio import activity

from src import s3_paths
from src.backends.chunking_strategy import get as get_chunking_strategy
from src.infra import InfraContainer

logger = logging.getLogger("platform.chunk")


@dataclass
class ChunkTextInput:
    tenant_id: str
    project_id: str
    document_ids: list[str]
    chunk_size: int = 1500  # target chars per chunk
    overlap: int = 200  # overlap between chunks


@dataclass
class ChunkTextOutput:
    chunk_count: int
    chunks_storage_path: str


class ChunkTextActivity:
    def __init__(self, infra: InfraContainer):
        self.infra = infra

    @activity.defn(name="chunk_text")
    async def run(self, input: ChunkTextInput) -> ChunkTextOutput:
        """Download parsed JSONs from S3, split into chunks, upload as JSONL."""
        s3 = self.infra.s3
        bucket = self.infra.s3_bucket
        chunker = get_chunking_strategy(self.infra.settings.chunking_backend)

        all_chunks = []

        for doc_id in input.document_ids:
            activity.heartbeat(f"chunking {doc_id}")
            # Download parsed JSON
            parsed_key = s3_paths.parsed_path(input.tenant_id, input.project_id, doc_id)
            try:
                response = s3.get_object(Bucket=bucket, Key=parsed_key)
                parsed_data = json.loads(response["Body"].read())
            except Exception as e:
                activity.logger.warning("Could not read parsed data for %s: %s", doc_id, e)
                continue

            # Extract full text from pages
            for page in parsed_data.get("pages", []):
                text = page.get("text", "")
                if not text.strip():
                    continue

                chunks = chunker.chunk(text, input.chunk_size, input.overlap)
                for i, chunk_text_content in enumerate(chunks):
                    all_chunks.append(
                        {
                            "chunk_id": str(uuid.uuid4()),
                            "doc_id": doc_id,
                            "page_num": page.get("page_num", 1),
                            "chunk_index": i,
                            "text": chunk_text_content,
                            "char_count": len(chunk_text_content),
                        }
                    )

        if not all_chunks:
            return ChunkTextOutput(chunk_count=0, chunks_storage_path="")

        # Upload chunks as JSONL
        batch_id = str(uuid.uuid4())
        chunks_key = s3_paths.chunks_path(input.tenant_id, input.project_id, batch_id)
        lines = [json.dumps(c, ensure_ascii=False) for c in all_chunks]
        s3.put_object(
            Bucket=bucket,
            Key=chunks_key,
            Body="\n".join(lines).encode("utf-8"),
            ContentType="application/jsonl",
        )

        activity.logger.info(
            "Chunked %d documents into %d chunks", len(input.document_ids), len(all_chunks)
        )
        return ChunkTextOutput(chunk_count=len(all_chunks), chunks_storage_path=chunks_key)


