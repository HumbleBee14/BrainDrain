"use client";

import { useParams, useRouter } from "next/navigation";
import Link from "next/link";
import { useProject, useDeleteProject } from "@/hooks/use-projects";
import { useDocuments, useUploadDocuments } from "@/hooks/use-documents";
import { usePipelineStatus, useTriggerParse, useTriggerRefine } from "@/hooks/use-pipeline";
import { useDatasets } from "@/hooks/use-datasets";
import { useCallback, useRef, useState } from "react";
import type { Document } from "@/lib/api-client";

function StatusBadge({ status }: { status: string }) {
  const colors: Record<string, string> = {
    active: "bg-emerald-900/50 text-emerald-400 border-emerald-800",
    created: "bg-blue-900/50 text-blue-400 border-blue-800",
    archived: "bg-zinc-800 text-zinc-400 border-zinc-700",
    draft: "bg-amber-900/50 text-amber-400 border-amber-800",
  };

  const cls = colors[status] || "bg-zinc-800 text-zinc-400 border-zinc-700";

  return (
    <span className={`inline-flex items-center rounded-full border px-2.5 py-0.5 text-xs font-medium ${cls}`}>
      {status}
    </span>
  );
}

function DocStatusBadge({ status }: { status: string }) {
  const colors: Record<string, string> = {
    uploaded: "bg-blue-900/50 text-blue-400 border-blue-800",
    parsing: "bg-amber-900/50 text-amber-400 border-amber-800",
    parsed: "bg-emerald-900/50 text-emerald-400 border-emerald-800",
    failed: "bg-red-900/50 text-red-400 border-red-800",
  };

  const cls = colors[status] || "bg-zinc-800 text-zinc-400 border-zinc-700";

  return (
    <span className={`inline-flex items-center rounded-full border px-2 py-0.5 text-xs font-medium ${cls}`}>
      {status}
    </span>
  );
}

function formatFileSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function DocumentRow({ doc }: { doc: Document }) {
  return (
    <div className="flex items-center justify-between py-3 px-4 border-b border-zinc-800 last:border-b-0">
      <div className="flex items-center gap-3 min-w-0">
        <div className="min-w-0">
          <p className="text-sm text-white truncate">{doc.filename}</p>
          <p className="text-xs text-zinc-600">
            {formatFileSize(doc.file_size)}
            {doc.language && ` \u00b7 ${doc.language}`}
            {doc.page_count && ` \u00b7 ${doc.page_count} pages`}
          </p>
        </div>
      </div>
      <div className="flex items-center gap-3 shrink-0">
        {doc.parse_quality != null && (
          <span className="text-xs text-zinc-500">
            {(doc.parse_quality * 100).toFixed(0)}% quality
          </span>
        )}
        <DocStatusBadge status={doc.status} />
      </div>
    </div>
  );
}

function PipelineStageCard({
  label,
  count,
  active,
}: {
  label: string;
  count: number;
  active: boolean;
}) {
  return (
    <div
      className={`rounded-lg border p-4 text-center ${
        active
          ? "border-emerald-800 bg-emerald-900/20"
          : "border-zinc-800"
      }`}
    >
      <p className="text-2xl font-bold text-white">{count}</p>
      <p className="text-xs text-zinc-500 mt-1">{label}</p>
    </div>
  );
}

export default function ProjectDetailPage() {
  const params = useParams<{ id: string }>();
  const router = useRouter();
  const { data: project, isLoading, error } = useProject(params.id);
  const deleteProject = useDeleteProject();
  const { data: pipelineStatus } = usePipelineStatus(params.id);
  const isActive =
    (pipelineStatus?.documents.parsing ?? 0) > 0 ||
    (pipelineStatus?.datasets.generating ?? 0) > 0;
  const { data: docsData } = useDocuments(params.id, 0, 50, isActive ? 3000 : false);
  const { data: datasetsData } = useDatasets(params.id);
  const uploadDocs = useUploadDocuments(params.id);
  const triggerParse = useTriggerParse(params.id);
  const triggerRefine = useTriggerRefine(params.id);

  const [confirmDelete, setConfirmDelete] = useState(false);
  const [isDragging, setIsDragging] = useState(false);
  const fileInputRef = useRef<HTMLInputElement>(null);

  const documents = docsData?.data ?? [];
  const datasets = datasetsData?.data ?? [];
  const status = pipelineStatus;

  const handleFiles = useCallback(
    (files: FileList | File[]) => {
      const fileArray = Array.from(files);
      if (fileArray.length > 0) {
        uploadDocs.mutate(fileArray);
      }
    },
    [uploadDocs],
  );

  const handleDrop = useCallback(
    (e: React.DragEvent) => {
      e.preventDefault();
      setIsDragging(false);
      handleFiles(e.dataTransfer.files);
    },
    [handleFiles],
  );

  const handleDelete = async () => {
    if (!confirmDelete) {
      setConfirmDelete(true);
      return;
    }
    await deleteProject.mutateAsync(params.id);
    router.push("/projects");
  };

  if (isLoading) {
    return (
      <div className="flex items-center justify-center py-20">
        <p className="text-zinc-500">Loading project...</p>
      </div>
    );
  }

  if (error || !project) {
    return (
      <div className="flex flex-col items-center justify-center py-20 gap-4">
        <p className="text-zinc-500">Project not found</p>
        <Link href="/projects" className="text-sm text-white underline hover:no-underline">
          Back to Projects
        </Link>
      </div>
    );
  }

  const hasUploaded = (status?.documents.uploaded ?? 0) > 0;
  const hasParsed = (status?.documents.parsed ?? 0) > 0;
  const isParsing = (status?.documents.parsing ?? 0) > 0;
  const isGenerating = (status?.datasets.generating ?? 0) > 0;

  return (
    <div>
      {/* Header */}
      <div className="mb-8">
        <Link
          href="/projects"
          className="text-sm text-zinc-500 hover:text-zinc-300 transition"
        >
          &larr; Back to Projects
        </Link>
        <div className="flex items-center gap-3 mt-2">
          <h1 className="text-2xl font-bold text-white">{project.name}</h1>
          <StatusBadge status={project.status} />
        </div>
        {project.description && (
          <p className="text-zinc-500 mt-1">{project.description}</p>
        )}
      </div>

      {/* Pipeline status overview */}
      {status && (
        <div className="mb-8">
          <h2 className="text-lg font-semibold text-white mb-4">Pipeline Status</h2>
          <div className="grid grid-cols-2 md:grid-cols-4 gap-3 mb-4">
            <PipelineStageCard
              label="Uploaded"
              count={status.documents.uploaded}
              active={status.documents.uploaded > 0}
            />
            <PipelineStageCard
              label="Parsing"
              count={status.documents.parsing}
              active={status.documents.parsing > 0}
            />
            <PipelineStageCard
              label="Parsed"
              count={status.documents.parsed}
              active={status.documents.parsed > 0}
            />
            <PipelineStageCard
              label="Failed"
              count={status.documents.failed}
              active={status.documents.failed > 0}
            />
          </div>

          {/* Action buttons */}
          <div className="flex gap-3">
            <button
              onClick={() => triggerParse.mutate()}
              disabled={!hasUploaded || triggerParse.isPending || isParsing}
              className="rounded-lg bg-blue-600 px-4 py-2 text-sm font-medium text-white hover:bg-blue-500 transition disabled:opacity-50 disabled:cursor-not-allowed"
            >
              {triggerParse.isPending
                ? "Starting..."
                : isParsing
                  ? "Parsing..."
                  : "Parse Documents"}
            </button>
            <button
              onClick={() => triggerRefine.mutate({ taskType: project.task_type || "question_answering" })}
              disabled={!hasParsed || triggerRefine.isPending || isGenerating}
              className="rounded-lg bg-emerald-600 px-4 py-2 text-sm font-medium text-white hover:bg-emerald-500 transition disabled:opacity-50 disabled:cursor-not-allowed"
            >
              {triggerRefine.isPending
                ? "Starting..."
                : isGenerating
                  ? "Generating..."
                  : "Generate Training Data"}
            </button>
          </div>

          {triggerParse.isError && (
            <p className="text-sm text-red-400 mt-2">{triggerParse.error.message}</p>
          )}
          {triggerRefine.isError && (
            <p className="text-sm text-red-400 mt-2">{triggerRefine.error.message}</p>
          )}
          {triggerParse.isSuccess && (
            <p className="text-sm text-emerald-400 mt-2">
              Parse workflow started for {triggerParse.data.document_count} documents
            </p>
          )}
          {triggerRefine.isSuccess && (
            <p className="text-sm text-emerald-400 mt-2">
              Refine workflow started for {triggerRefine.data.document_count} documents
            </p>
          )}
        </div>
      )}

      {/* Upload area */}
      <div className="mb-8">
        <div className="flex items-center justify-between mb-4">
          <h2 className="text-lg font-semibold text-white">
            Documents {documents.length > 0 && `(${docsData?.total ?? documents.length})`}
          </h2>
        </div>
        <div
          className={`rounded-lg border-2 border-dashed p-8 text-center transition ${
            isDragging
              ? "border-blue-500 bg-blue-900/10"
              : "border-zinc-700 hover:border-zinc-600"
          }`}
          onDragOver={(e) => {
            e.preventDefault();
            setIsDragging(true);
          }}
          onDragLeave={() => setIsDragging(false)}
          onDrop={handleDrop}
        >
          {uploadDocs.isPending ? (
            <p className="text-zinc-400">Uploading...</p>
          ) : (
            <>
              <p className="text-zinc-500 mb-2">
                {isDragging ? "Drop files here" : "Drag and drop files here or click to upload"}
              </p>
              <p className="text-xs text-zinc-600">
                Supports PDF, DOCX, TXT, CSV, JSON, JSONL, MD (max 500 MB)
              </p>
              <label className="mt-4 inline-block cursor-pointer rounded-lg bg-zinc-800 px-4 py-2 text-sm text-white hover:bg-zinc-700 transition">
                Choose Files
                <input
                  ref={fileInputRef}
                  type="file"
                  multiple
                  className="hidden"
                  accept=".pdf,.docx,.txt,.csv,.json,.jsonl,.md,.html"
                  onChange={(e) => {
                    if (e.target.files) handleFiles(e.target.files);
                  }}
                />
              </label>
            </>
          )}
          {uploadDocs.isError && (
            <p className="text-sm text-red-400 mt-2">{uploadDocs.error.message}</p>
          )}
          {uploadDocs.isSuccess && (
            <p className="text-sm text-emerald-400 mt-2">
              {uploadDocs.data.length} file(s) uploaded successfully
            </p>
          )}
        </div>

        {/* Document list */}
        {documents.length > 0 && (
          <div className="mt-4 rounded-lg border border-zinc-800 divide-y divide-zinc-800">
            {documents.map((doc) => (
              <DocumentRow key={doc.id} doc={doc} />
            ))}
          </div>
        )}
      </div>

      {/* Datasets section */}
      {datasets.length > 0 && (
        <div className="mb-8">
          <h2 className="text-lg font-semibold text-white mb-4">
            Datasets ({datasetsData?.total ?? datasets.length})
          </h2>
          <div className="rounded-lg border border-zinc-800">
            {datasets.map((ds) => (
              <Link
                key={ds.id}
                href={`/projects/${params.id}/dataset?datasetId=${ds.id}`}
                className="flex items-center justify-between py-3 px-4 border-b border-zinc-800 last:border-b-0 hover:bg-zinc-900/50 transition"
              >
                <div>
                  <p className="text-sm text-white">{ds.name}</p>
                  <p className="text-xs text-zinc-600">
                    {ds.pair_count != null ? `${ds.pair_count} pairs` : "Generating..."}
                    {" \u00b7 "}
                    {ds.format}
                  </p>
                </div>
                <DocStatusBadge status={ds.status} />
              </Link>
            ))}
          </div>
        </div>
      )}

      {/* Info grid */}
      <div className="grid grid-cols-1 md:grid-cols-3 gap-4 mb-8">
        <div className="rounded-lg border border-zinc-800 p-4">
          <p className="text-xs text-zinc-500 uppercase tracking-wider">Task Type</p>
          <p className="text-white mt-1">{project.task_type || "Not set"}</p>
        </div>
        <div className="rounded-lg border border-zinc-800 p-4">
          <p className="text-xs text-zinc-500 uppercase tracking-wider">Created</p>
          <p className="text-white mt-1">
            {new Date(project.created_at).toLocaleDateString()}
          </p>
        </div>
        <div className="rounded-lg border border-zinc-800 p-4">
          <p className="text-xs text-zinc-500 uppercase tracking-wider">Updated</p>
          <p className="text-white mt-1">
            {new Date(project.updated_at).toLocaleDateString()}
          </p>
        </div>
      </div>

      {/* Danger zone */}
      <div className="rounded-lg border border-zinc-800 p-6">
        <h3 className="text-sm font-medium text-zinc-400 mb-4">Danger Zone</h3>
        <div className="flex items-center justify-between">
          <div>
            <p className="text-sm text-white">Delete this project</p>
            <p className="text-xs text-zinc-600">This action cannot be undone.</p>
          </div>
          <button
            onClick={handleDelete}
            disabled={deleteProject.isPending}
            className={`rounded-lg px-4 py-2 text-sm font-medium transition ${
              confirmDelete
                ? "bg-red-600 text-white hover:bg-red-500"
                : "border border-red-800 text-red-400 hover:bg-red-900/30"
            } disabled:opacity-50`}
          >
            {deleteProject.isPending
              ? "Deleting..."
              : confirmDelete
                ? "Confirm Delete"
                : "Delete Project"}
          </button>
        </div>
      </div>
    </div>
  );
}
