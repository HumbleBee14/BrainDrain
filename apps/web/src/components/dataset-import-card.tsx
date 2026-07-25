"use client";

import { useRef, useState } from "react";
import { toast } from "sonner";
import { useImportDataset } from "@/hooks/use-datasets";
import { ApiClientError, type DatasetImportRowError } from "@/lib/api-client";

const MAX_IMPORT_BYTES = 100 * 1024 * 1024;

interface ImportOutcome {
  datasetName: string;
  importedRows: number;
  rejectedRows: number;
  errors: DatasetImportRowError[];
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

function validateFile(file: File): string | null {
  if (!file.name.toLowerCase().endsWith(".jsonl")) {
    return "Select a .jsonl file — one JSON object per line.";
  }
  if (file.size === 0) return "That file is empty.";
  if (file.size > MAX_IMPORT_BYTES) {
    return `File is ${formatBytes(file.size)}; the limit is ${formatBytes(MAX_IMPORT_BYTES)}.`;
  }
  return null;
}

function SpinnerIcon() {
  return (
    <svg className="h-4 w-4 animate-spin" viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" />
      <path
        className="opacity-75"
        fill="currentColor"
        d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"
      />
    </svg>
  );
}

function UploadIcon() {
  return (
    <svg
      className="h-4 w-4"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d="M21 15v4a2 2 0 01-2 2H5a2 2 0 01-2-2v-4" />
      <polyline points="17 8 12 3 7 8" />
      <line x1="12" y1="3" x2="12" y2="15" />
    </svg>
  );
}

export function DatasetImportCard({ projectId }: { projectId: string }) {
  const importDataset = useImportDataset(projectId);
  const fileInputRef = useRef<HTMLInputElement>(null);

  const [file, setFile] = useState<File | null>(null);
  const [name, setName] = useState("");
  const [validationError, setValidationError] = useState<string | null>(null);
  const [requestError, setRequestError] = useState<string | null>(null);
  const [outcome, setOutcome] = useState<ImportOutcome | null>(null);
  const [isDragging, setIsDragging] = useState(false);

  const isUploading = importDataset.isPending;

  const selectFile = (next: File | null) => {
    setRequestError(null);
    setOutcome(null);
    if (!next) {
      setFile(null);
      setValidationError(null);
      return;
    }
    const problem = validateFile(next);
    setValidationError(problem);
    setFile(problem ? null : next);
  };

  const reset = () => {
    setFile(null);
    setName("");
    setValidationError(null);
    setRequestError(null);
    setOutcome(null);
    if (fileInputRef.current) fileInputRef.current.value = "";
  };

  const handleUpload = async () => {
    if (!file || isUploading) return;
    setRequestError(null);
    try {
      const result = await importDataset.mutateAsync({ file, name });
      setOutcome({
        datasetName: result.dataset.name,
        importedRows: result.imported_rows,
        rejectedRows: result.rejected_rows,
        errors: result.errors,
      });
      setFile(null);
      setName("");
      if (fileInputRef.current) fileInputRef.current.value = "";
      toast.success(
        `Imported ${result.imported_rows} pair${result.imported_rows === 1 ? "" : "s"}`,
      );
    } catch (err) {
      const message =
        err instanceof ApiClientError
          ? err.message
          : "Import failed. Check your connection and try again.";
      setRequestError(message);
      toast.error(message);
    }
  };

  const handleDrop = (event: React.DragEvent<HTMLDivElement>) => {
    event.preventDefault();
    setIsDragging(false);
    if (isUploading) return;
    selectFile(event.dataTransfer.files?.[0] ?? null);
  };

  return (
    <div className="rounded-lg border border-zinc-200 dark:border-zinc-800 p-4">
      <div className="mb-3">
        <h3 className="text-sm font-medium text-zinc-900 dark:text-white">
          Import a dataset
        </h3>
        <p className="text-xs text-zinc-500 dark:text-zinc-400 mt-1">
          Upload OpenAI chat-format JSONL to skip generation. Each line needs a{" "}
          <code className="text-[11px]">messages</code> array. Imported datasets
          go through the same review step before training.
        </p>
      </div>

      <div
        onDragOver={(e) => {
          e.preventDefault();
          if (!isUploading) setIsDragging(true);
        }}
        onDragLeave={() => setIsDragging(false)}
        onDrop={handleDrop}
        className={`rounded-lg border border-dashed px-4 py-6 text-center transition ${
          isDragging
            ? "border-blue-500 bg-blue-50/50 dark:bg-blue-950/20"
            : "border-zinc-300 dark:border-zinc-700"
        } ${isUploading ? "opacity-60" : ""}`}
      >
        <input
          ref={fileInputRef}
          type="file"
          accept=".jsonl"
          disabled={isUploading}
          onChange={(e) => selectFile(e.target.files?.[0] ?? null)}
          className="hidden"
          id="dataset-import-file"
        />
        <label
          htmlFor="dataset-import-file"
          className={`text-sm ${
            isUploading
              ? "text-zinc-400 dark:text-zinc-600"
              : "text-blue-600 dark:text-blue-400 hover:underline cursor-pointer"
          }`}
        >
          Choose a .jsonl file
        </label>
        <p className="text-xs text-zinc-400 dark:text-zinc-600 mt-1">
          or drag it here &middot; up to {formatBytes(MAX_IMPORT_BYTES)}
        </p>
        {file && (
          <p className="text-xs text-zinc-600 dark:text-zinc-300 mt-3">
            {file.name}{" "}
            <span className="text-zinc-400 dark:text-zinc-600">
              ({formatBytes(file.size)})
            </span>
          </p>
        )}
      </div>

      {validationError && (
        <p role="alert" className="text-xs text-red-600 dark:text-red-400 mt-3">
          {validationError}
        </p>
      )}

      {file && (
        <div className="mt-3">
          <label
            htmlFor="dataset-import-name"
            className="block text-xs text-zinc-500 dark:text-zinc-400 mb-1"
          >
            Dataset name (optional)
          </label>
          <input
            id="dataset-import-name"
            type="text"
            value={name}
            disabled={isUploading}
            placeholder={file.name}
            onChange={(e) => setName(e.target.value)}
            className="w-full rounded-lg border border-zinc-200 dark:border-zinc-800 bg-white dark:bg-zinc-950 px-3 py-2 text-sm text-zinc-900 dark:text-white placeholder:text-zinc-400 dark:placeholder:text-zinc-600 disabled:opacity-60"
          />
        </div>
      )}

      <div className="flex items-center gap-2 mt-3">
        <button
          onClick={handleUpload}
          disabled={!file || isUploading}
          className="inline-flex items-center gap-2 rounded-lg bg-blue-600 px-4 py-2 text-sm font-medium text-white hover:bg-blue-500 disabled:opacity-40 disabled:cursor-not-allowed transition"
        >
          {isUploading ? <SpinnerIcon /> : <UploadIcon />}
          {isUploading ? "Importing..." : "Import dataset"}
        </button>
        {(file || outcome || requestError) && !isUploading && (
          <button
            onClick={reset}
            className="rounded-lg border border-zinc-200 dark:border-zinc-800 px-3 py-2 text-sm text-zinc-600 dark:text-zinc-300 hover:bg-zinc-50 dark:hover:bg-zinc-900 transition"
          >
            Clear
          </button>
        )}
      </div>

      {requestError && (
        <p role="alert" className="text-xs text-red-600 dark:text-red-400 mt-3">
          {requestError}
        </p>
      )}

      {outcome && (
        <div className="mt-4 rounded-lg border border-zinc-200 dark:border-zinc-800 p-3">
          <p className="text-sm text-zinc-900 dark:text-white">
            {outcome.datasetName}
          </p>
          <p className="text-xs text-zinc-500 dark:text-zinc-400 mt-1">
            {outcome.importedRows} imported
            {outcome.rejectedRows > 0 && ` · ${outcome.rejectedRows} rejected`}
          </p>
          {outcome.importedRows === 0 && (
            <p className="text-xs text-red-600 dark:text-red-400 mt-2">
              No rows were valid, so there is nothing to train on.
            </p>
          )}
          {outcome.errors.length > 0 && (
            <div className="mt-2 max-h-40 overflow-y-auto">
              <ul className="space-y-1">
                {outcome.errors.map((rowError) => (
                  <li
                    key={rowError.line}
                    className="text-xs text-yellow-700 dark:text-yellow-500"
                  >
                    Line {rowError.line}: {rowError.error}
                  </li>
                ))}
              </ul>
              {outcome.rejectedRows > outcome.errors.length && (
                <p className="text-xs text-zinc-400 dark:text-zinc-600 mt-1">
                  ...and {outcome.rejectedRows - outcome.errors.length} more.
                </p>
              )}
            </div>
          )}
          {outcome.importedRows > 0 && (
            <p className="text-xs text-zinc-500 dark:text-zinc-400 mt-2">
              Approve it in the Datasets list below to train on it.
            </p>
          )}
        </div>
      )}
    </div>
  );
}
