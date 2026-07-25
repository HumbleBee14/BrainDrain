"""Ingest workflow — handles document parsing for a project.

Triggered when a user clicks "Parse Documents". For each document:
fetches metadata from DB, then calls the parse activity.
Handles partial failures — some docs can fail without killing the workflow.
"""

from temporalio import workflow
from temporalio.common import RetryPolicy
from temporalio.exceptions import ApplicationError

with workflow.unsafe.imports_passed_through():
    from src import timeouts
    from src.activities.parse_document import ParseDocumentInput
    from src.activities.stubs import DocumentInfo, GetDocumentInfoInput


@workflow.defn
class IngestWorkflow:
    """Parse uploaded documents and store structured results."""

    @workflow.run
    async def run(self, tenant_id: str, project_id: str, document_ids: list[str]) -> dict:
        successes = []
        failures = []

        for doc_id in document_ids:
            try:
                # Fetch document metadata from DB (storage_path, mime_type)
                doc_info: DocumentInfo = await workflow.execute_activity(
                    "get_document_info",
                    GetDocumentInfoInput(tenant_id=tenant_id, document_id=doc_id),
                    start_to_close_timeout=timeouts.db_lookup(),
                    result_type=DocumentInfo,
                )

                # Skip if already parsed
                if doc_info.status == "parsed":
                    successes.append(doc_id)
                    continue

                # Parse the document
                await workflow.execute_activity(
                    "parse_document",
                    ParseDocumentInput(
                        tenant_id=tenant_id,
                        project_id=project_id,
                        document_id=doc_id,
                        storage_path=doc_info.storage_path,
                        mime_type=doc_info.mime_type,
                    ),
                    start_to_close_timeout=timeouts.parse_activity(),
                    retry_policy=RetryPolicy(maximum_attempts=3),
                    heartbeat_timeout=timeouts.parse_heartbeat(),
                )
                successes.append(doc_id)

            except Exception as e:
                workflow.logger.error("Failed to parse document %s: %s", doc_id, str(e))
                failures.append({"doc_id": doc_id, "error": str(e)[:200]})

        # Partial failure is tolerated, total failure is not: reporting success
        # with zero parsed documents leaves the user staring at an idle pipeline
        # with no error anywhere.
        if failures and not successes:
            raise ApplicationError(
                f"All {len(failures)} document(s) failed to parse: {failures[0]['error']}",
                non_retryable=True,
            )

        return {
            "project_id": project_id,
            "documents_processed": len(successes),
            "documents_failed": len(failures),
            "failures": failures,
        }
