"use client";

import Link from "next/link";
import type { Dataset } from "@/lib/api-client";
import { DatasetImportCard } from "@/components/dataset-import-card";
import { Button } from "@/components/ui/button";
import { DatasetStatusBadge } from "./dataset-status-badge";

function DatasetRow({
  dataset,
  projectId,
}: {
  dataset: Dataset;
  projectId: string;
}) {
  const isGenerating = dataset.status === "generating";
  const isFailed = dataset.status === "failed";

  const detail = isFailed
    ? (dataset.error ?? "Generation failed")
    : isGenerating
      ? "Generating pairs — this takes a few minutes"
      : `${dataset.pair_count ?? 0} pairs · ${dataset.format}`;

  const body = (
    <div className="flex items-center justify-between gap-3 px-4 py-3">
      <div className="min-w-0">
        <p className="truncate text-sm text-zinc-900 dark:text-white">
          {dataset.name}
        </p>
        <p
          className={`mt-0.5 text-xs ${isFailed ? "text-red-600 dark:text-red-400" : "text-zinc-400 dark:text-zinc-600"}`}
        >
          {detail}
        </p>
      </div>
      <DatasetStatusBadge status={dataset.status} />
    </div>
  );

  // Nothing to review until pairs exist, so these rows are not navigable.
  if (isGenerating || isFailed) {
    return (
      <div className="border-b border-zinc-200 last:border-b-0 dark:border-zinc-800">
        {body}
      </div>
    );
  }

  return (
    <Link
      href={`/projects/${projectId}/dataset?datasetId=${dataset.id}`}
      className="block border-b border-zinc-200 transition last:border-b-0 hover:bg-zinc-50/50 dark:border-zinc-800 dark:hover:bg-zinc-900/50"
    >
      {body}
    </Link>
  );
}

export function DatasetsTab({
  projectId,
  datasets,
  hasParsedDocuments,
  onGenerate,
  generatePending,
}: {
  projectId: string;
  datasets: Dataset[];
  hasParsedDocuments: boolean;
  onGenerate: () => void;
  generatePending: boolean;
}) {
  return (
    <div>
      <div className="mb-4 flex flex-col justify-between gap-3 sm:flex-row sm:items-center">
        <p className="text-sm text-zinc-500">
          {hasParsedDocuments
            ? "Generate a dataset from your parsed documents, or import one."
            : "Parse documents first to generate a dataset, or import one below."}
        </p>
        <div className="flex shrink-0 items-center gap-2">
          {hasParsedDocuments ? (
            <Link href={`/projects/${projectId}/data-studio`}>
              <Button
                variant="secondary"
                title="Review facets and preview samples before generating the dataset"
              >
                Data Studio (Guided)
              </Button>
            </Link>
          ) : (
            <Button variant="secondary" disabled title="Parse documents first">
              Data Studio (Guided)
            </Button>
          )}
          <Button
            onClick={onGenerate}
            disabled={!hasParsedDocuments}
            loading={generatePending}
            title={hasParsedDocuments ? undefined : "Parse documents first"}
          >
            {generatePending ? "Starting..." : "Generate Training Data"}
          </Button>
        </div>
      </div>
      <DatasetImportCard projectId={projectId} />
      {datasets.length > 0 && (
        <div className="mt-4 rounded-lg border border-zinc-200 dark:border-zinc-800">
          {datasets.map((ds) => (
            <DatasetRow key={ds.id} dataset={ds} projectId={projectId} />
          ))}
        </div>
      )}
      {datasets.length === 0 && (
        <p className="mt-4 text-sm text-zinc-400 dark:text-zinc-600">
          No datasets yet — generate one from parsed documents or import a
          JSONL file above.
        </p>
      )}
    </div>
  );
}
