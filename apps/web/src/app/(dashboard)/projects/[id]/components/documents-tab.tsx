"use client";

import { useState } from "react";
import type { Document } from "@/lib/api-client";
import { DocumentRow } from "./document-row";
import { DocumentDropzone } from "./document-dropzone";

interface UploadMutation {
  mutate: (files: File[]) => void;
  isPending: boolean;
  isError: boolean;
  error: Error | null;
}

export function DocumentsTab({
  allDocuments,
  uploadDocs,
}: {
  allDocuments: Document[];
  uploadDocs: UploadMutation;
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
    <div>
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
    </div>
  );
}
