"use client";

import Link from "next/link";
import { useState } from "react";
import type { Dataset, Document } from "@/lib/api-client";
import { DatasetImportCard } from "@/components/dataset-import-card";
import { Button } from "@/components/ui/button";
import { DatasetStatusBadge } from "./dataset-status-badge";
import { DocumentRow } from "./document-row";
import { DocumentDropzone } from "./document-dropzone";

interface UploadMutation {
  mutate: (files: File[]) => void;
  isPending: boolean;
  isError: boolean;
  error: Error | null;
}

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
      : `${dataset.pair_count ?? 0} pairs · ${dataset.format} · click to review`;

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

export function DataTab({
  projectId,
  allDocuments,
  uploadDocs,
  datasets,
  hasParsedDocuments,
  canParse,
  isParsing,
  onParse,
  parsePending,
  onGenerate,
  generatePending,
}: {
  projectId: string;
  allDocuments: Document[];
  uploadDocs: UploadMutation;
  datasets: Dataset[];
  hasParsedDocuments: boolean;
  canParse: boolean;
  isParsing: boolean;
  onParse: () => void;
  parsePending: boolean;
  onGenerate: () => void;
  generatePending: boolean;
}) {
  const [search, setSearch] = useState("");
  const [statusFilter, setStatusFilter] = useState<string>("all");

  const documents = allDocuments.filter((doc) => {
    const matchesSearch =
      !search || doc.filename.toLowerCase().includes(search.toLowerCase());
    const matchesStatus = statusFilter === "all" || doc.status === statusFilter;
    return matchesSearch && matchesStatus;
  });

  return (
    <div className="space-y-8">
      {/* Path A step 1: source documents */}
      <section>
        <div className="mb-3 flex flex-col justify-between gap-3 sm:flex-row sm:items-center">
          <div>
            <h3 className="text-sm font-semibold text-zinc-900 dark:text-white">
              Source documents
              <span className="ml-2 text-xs font-normal text-zinc-400 dark:text-zinc-600">
                optional
              </span>
            </h3>
            <p className="mt-0.5 text-sm text-zinc-500">
              Only needed if the platform should generate training data for
              you. Upload files, then parse them. Already have a dataset? Skip
              straight to the import below.
            </p>
          </div>
          <Button
            onClick={onParse}
            disabled={!canParse}
            loading={parsePending}
            title={
              canParse
                ? undefined
                : isParsing
                  ? "Parsing is already running"
                  : "Upload a document first"
            }
            className="shrink-0"
          >
            {parsePending
              ? "Starting..."
              : isParsing
                ? "Parsing..."
                : "Parse Documents"}
          </Button>
        </div>

        <DocumentDropzone uploadDocs={uploadDocs} />

        {allDocuments.length > 3 && (
          <div className="mt-4 flex flex-col gap-2 sm:flex-row">
            <input
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              placeholder="Search documents..."
              className="flex-1 rounded-lg border border-zinc-300 bg-zinc-50 px-3 py-1.5 text-sm text-zinc-900 placeholder:text-zinc-400 dark:border-zinc-700 dark:bg-zinc-900 dark:text-white dark:placeholder:text-zinc-600"
            />
            <select
              value={statusFilter}
              onChange={(e) => setStatusFilter(e.target.value)}
              className="rounded-lg border border-zinc-300 bg-zinc-50 px-3 py-1.5 text-sm text-zinc-900 dark:border-zinc-700 dark:bg-zinc-900 dark:text-white"
            >
              <option value="all">All statuses</option>
              <option value="uploaded">Uploaded</option>
              <option value="parsing">Parsing</option>
              <option value="parsed">Parsed</option>
              <option value="failed">Failed</option>
            </select>
          </div>
        )}

        {documents.length > 0 && (
          <div className="mt-4 divide-y divide-zinc-200 rounded-lg border border-zinc-200 dark:divide-zinc-800 dark:border-zinc-800">
            {documents.map((doc) => (
              <DocumentRow key={doc.id} doc={doc} />
            ))}
          </div>
        )}
        {allDocuments.length > 0 && documents.length === 0 && (
          <p className="mt-4 text-sm text-zinc-400 dark:text-zinc-600">
            No documents match the current filter.
          </p>
        )}
      </section>

      {/* Path A step 2 / Path B: the training dataset */}
      <section>
        <div className="mb-3 flex flex-col justify-between gap-3 sm:flex-row sm:items-center">
          <div>
            <h3 className="text-sm font-semibold text-zinc-900 dark:text-white">
              Training dataset
              <span className="ml-2 text-xs font-normal text-zinc-400 dark:text-zinc-600">
                required for fine-tuning
              </span>
            </h3>
            <p className="mt-0.5 text-sm text-zinc-500">
              Two ways to get one: generate it from your parsed documents, or
              import a JSONL file directly — no documents needed. Either way it
              appears below; click a dataset to review and approve its pairs.
            </p>
          </div>
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
              <Button
                variant="secondary"
                disabled
                title="Parse documents first"
              >
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

        {datasets.length > 0 && (
          <div className="mb-4 rounded-lg border border-zinc-200 dark:border-zinc-800">
            {datasets.map((ds) => (
              <DatasetRow key={ds.id} dataset={ds} projectId={projectId} />
            ))}
          </div>
        )}

        <DatasetImportCard projectId={projectId} />
      </section>
    </div>
  );
}
