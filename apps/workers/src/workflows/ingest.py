"""Ingest workflow — handles document upload → parse → store.

Triggered when a user uploads documents to a project.
"""

from datetime import timedelta

from temporalio import workflow

with workflow.unsafe.imports_passed_through():
    from src.activities.stubs import (
        ParseDocumentInput,
        ParseDocumentOutput,
        parse_document,
    )


@workflow.defn
class IngestWorkflow:
    """Parse uploaded documents and store structured results.

    Input: tenant_id, project_id, list of document IDs to process.
    For each document: parse → update DB status → emit event.
    """

    @workflow.run
    async def run(self, tenant_id: str, project_id: str, document_ids: list[str]) -> dict:
        results: dict[str, ParseDocumentOutput] = {}

        for doc_id in document_ids:
            result = await workflow.execute_activity(
                parse_document,
                ParseDocumentInput(
                    tenant_id=tenant_id,
                    document_id=doc_id,
                    storage_path=f"{tenant_id}/{project_id}/uploads/{doc_id}",
                    mime_type="application/pdf",
                ),
                start_to_close_timeout=timedelta(minutes=10),
                retry_policy=workflow.RetryPolicy(maximum_attempts=3),
            )
            results[doc_id] = result

        return {
            "project_id": project_id,
            "documents_processed": len(results),
        }
