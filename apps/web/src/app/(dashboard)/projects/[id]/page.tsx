"use client";

import { useParams, useRouter } from "next/navigation";
import Link from "next/link";
import { useProject, useDeleteProject } from "@/hooks/use-projects";
import { useDocuments, useUploadDocuments } from "@/hooks/use-documents";
import { usePipelineStatus, useTriggerParse, useTriggerRefine } from "@/hooks/use-pipeline";
import { useDatasets } from "@/hooks/use-datasets";
import { useTrainingJobs, useCreateTrainingJob, useCancelTrainingJob } from "@/hooks/use-training";
import { useModels } from "@/hooks/use-models";
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

function TrainingStatusBadge({ status }: { status: string }) {
  const colors: Record<string, string> = {
    pending: "bg-zinc-800 text-zinc-400 border-zinc-700",
    cost_approval: "bg-amber-900/50 text-amber-400 border-amber-800",
    provisioning: "bg-blue-900/50 text-blue-400 border-blue-800",
    training: "bg-violet-900/50 text-violet-400 border-violet-800 animate-pulse",
    completed: "bg-emerald-900/50 text-emerald-400 border-emerald-800",
    failed: "bg-red-900/50 text-red-400 border-red-800",
    cancelled: "bg-zinc-800 text-zinc-500 border-zinc-700",
  };

  const cls = colors[status] || "bg-zinc-800 text-zinc-400 border-zinc-700";

  return (
    <span className={`inline-flex items-center rounded-full border px-2 py-0.5 text-xs font-medium ${cls}`}>
      {status.replace("_", " ")}
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
    (pipelineStatus?.datasets.generating ?? 0) > 0 ||
    (pipelineStatus?.training_jobs?.training ?? 0) > 0;
  const { data: docsData } = useDocuments(params.id, 0, 50, isActive ? 3000 : false);
  const { data: datasetsData } = useDatasets(params.id);
  const { data: trainingJobsData } = useTrainingJobs(params.id);
  const { data: modelsData } = useModels(params.id);
  const uploadDocs = useUploadDocuments(params.id);
  const triggerParse = useTriggerParse(params.id);
  const triggerRefine = useTriggerRefine(params.id);
  const createTrainingJob = useCreateTrainingJob(params.id);
  const cancelTrainingJob = useCancelTrainingJob(params.id);

  const [confirmDelete, setConfirmDelete] = useState(false);
  const [isDragging, setIsDragging] = useState(false);
  const [showTrainForm, setShowTrainForm] = useState(false);
  const [trainForm, setTrainForm] = useState({
    dataset_id: "",
    base_model: "unsloth/Llama-3.2-1B-Instruct",
    method: "qlora",
    mode: "quick",
  });
  const fileInputRef = useRef<HTMLInputElement>(null);

  const documents = docsData?.data ?? [];
  const datasets = datasetsData?.data ?? [];
  const trainingJobs = trainingJobsData?.data ?? [];
  const models = modelsData?.data ?? [];
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
  const hasApprovedDatasets = (status?.datasets.approved ?? 0) > 0;
  const approvedDatasets = datasets.filter((ds) => ds.status === "approved");

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
          <div className="grid grid-cols-2 md:grid-cols-4 lg:grid-cols-6 gap-3 mb-4">
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
              label="Datasets"
              count={status.datasets.total}
              active={status.datasets.approved > 0}
            />
            <PipelineStageCard
              label="Training"
              count={status.training_jobs?.training ?? 0}
              active={(status.training_jobs?.training ?? 0) > 0}
            />
            <PipelineStageCard
              label="Models"
              count={status.models?.total ?? 0}
              active={(status.models?.active ?? 0) > 0}
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

      {/* Training Jobs section */}
      <div className="mb-8">
        <div className="flex items-center justify-between mb-4">
          <h2 className="text-lg font-semibold text-white">
            Training Jobs {trainingJobs.length > 0 && `(${trainingJobsData?.total ?? trainingJobs.length})`}
          </h2>
          <button
            onClick={() => setShowTrainForm(!showTrainForm)}
            disabled={!hasApprovedDatasets}
            className="rounded-lg bg-violet-600 px-4 py-2 text-sm font-medium text-white hover:bg-violet-500 transition disabled:opacity-50 disabled:cursor-not-allowed"
          >
            Start Training
          </button>
        </div>

        {/* Training form */}
        {showTrainForm && (
          <div className="rounded-lg border border-zinc-800 p-4 mb-4 space-y-3">
            <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
              <div>
                <label className="block text-xs text-zinc-500 mb-1">Dataset</label>
                <select
                  value={trainForm.dataset_id}
                  onChange={(e) => setTrainForm({ ...trainForm, dataset_id: e.target.value })}
                  className="w-full rounded-lg border border-zinc-700 bg-zinc-900 px-3 py-2 text-sm text-white"
                >
                  <option value="">Select dataset...</option>
                  {approvedDatasets.map((ds) => (
                    <option key={ds.id} value={ds.id}>
                      {ds.name} ({ds.pair_count ?? 0} pairs)
                    </option>
                  ))}
                </select>
              </div>
              <div>
                <label className="block text-xs text-zinc-500 mb-1">Base Model</label>
                <select
                  value={trainForm.base_model}
                  onChange={(e) => setTrainForm({ ...trainForm, base_model: e.target.value })}
                  className="w-full rounded-lg border border-zinc-700 bg-zinc-900 px-3 py-2 text-sm text-white"
                >
                  <option value="unsloth/Llama-3.2-1B-Instruct">Llama 3.2 1B Instruct</option>
                  <option value="unsloth/Llama-3.2-3B-Instruct">Llama 3.2 3B Instruct</option>
                  <option value="unsloth/Meta-Llama-3.1-8B-Instruct">Llama 3.1 8B Instruct</option>
                  <option value="unsloth/Qwen2.5-7B-Instruct">Qwen 2.5 7B Instruct</option>
                  <option value="unsloth/Mistral-7B-Instruct-v0.3">Mistral 7B Instruct v0.3</option>
                  <option value="unsloth/gemma-2-2b-it">Gemma 2 2B IT</option>
                </select>
              </div>
              <div>
                <label className="block text-xs text-zinc-500 mb-1">Method</label>
                <select
                  value={trainForm.method}
                  onChange={(e) => setTrainForm({ ...trainForm, method: e.target.value })}
                  className="w-full rounded-lg border border-zinc-700 bg-zinc-900 px-3 py-2 text-sm text-white"
                >
                  <option value="qlora">QLoRA (4-bit, fastest)</option>
                  <option value="lora">LoRA (16-bit)</option>
                  <option value="full">Full Fine-tune</option>
                </select>
              </div>
              <div>
                <label className="block text-xs text-zinc-500 mb-1">Mode</label>
                <select
                  value={trainForm.mode}
                  onChange={(e) => setTrainForm({ ...trainForm, mode: e.target.value })}
                  className="w-full rounded-lg border border-zinc-700 bg-zinc-900 px-3 py-2 text-sm text-white"
                >
                  <option value="quick">Quick (SFT only)</option>
                  <option value="aligned">Aligned (SFT + DPO)</option>
                  <option value="reasoning">Reasoning (SFT + GRPO)</option>
                  <option value="iterative">Iterative (Multi-round)</option>
                </select>
              </div>
            </div>
            <div className="flex gap-2 pt-2">
              <button
                onClick={() => {
                  if (!trainForm.dataset_id) return;
                  createTrainingJob.mutate(trainForm);
                  setShowTrainForm(false);
                }}
                disabled={!trainForm.dataset_id || createTrainingJob.isPending}
                className="rounded-lg bg-violet-600 px-4 py-2 text-sm font-medium text-white hover:bg-violet-500 transition disabled:opacity-50"
              >
                {createTrainingJob.isPending ? "Starting..." : "Start Training Job"}
              </button>
              <button
                onClick={() => setShowTrainForm(false)}
                className="rounded-lg border border-zinc-700 px-4 py-2 text-sm text-zinc-400 hover:border-zinc-600 transition"
              >
                Cancel
              </button>
            </div>
            {createTrainingJob.isError && (
              <p className="text-sm text-red-400">{createTrainingJob.error.message}</p>
            )}
          </div>
        )}

        {/* Training jobs list */}
        {trainingJobs.length > 0 && (
          <div className="rounded-lg border border-zinc-800">
            {trainingJobs.map((job) => (
              <Link
                key={job.id}
                href={`/projects/${params.id}/training/${job.id}`}
                className="flex items-center justify-between py-3 px-4 border-b border-zinc-800 last:border-b-0 hover:bg-zinc-900/50 transition"
              >
                <div>
                  <p className="text-sm text-white">
                    {job.base_model.split("/").pop()} &mdash; {job.mode}
                  </p>
                  <p className="text-xs text-zinc-600">
                    {job.method.toUpperCase()}
                    {job.cost_estimate != null && ` \u00b7 ~$${job.cost_estimate.toFixed(2)}`}
                    {" \u00b7 "}
                    {new Date(job.created_at).toLocaleDateString()}
                  </p>
                </div>
                <div className="flex items-center gap-2">
                  {["pending", "cost_approval"].includes(job.status) && (
                    <button
                      onClick={(e) => {
                        e.preventDefault();
                        cancelTrainingJob.mutate(job.id);
                      }}
                      className="text-xs text-red-400 hover:text-red-300 transition"
                    >
                      Cancel
                    </button>
                  )}
                  <TrainingStatusBadge status={job.status} />
                </div>
              </Link>
            ))}
          </div>
        )}

        {trainingJobs.length === 0 && !showTrainForm && (
          <p className="text-sm text-zinc-600">
            {hasApprovedDatasets
              ? "No training jobs yet. Click \"Start Training\" to begin."
              : "Approve a dataset first to start training."}
          </p>
        )}
      </div>

      {/* Models section */}
      {models.length > 0 && (
        <div className="mb-8">
          <h2 className="text-lg font-semibold text-white mb-4">
            Models ({modelsData?.total ?? models.length})
          </h2>
          <div className="rounded-lg border border-zinc-800">
            {models.map((model) => (
              <Link
                key={model.id}
                href={`/projects/${params.id}/models/${model.id}`}
                className="flex items-center justify-between py-3 px-4 border-b border-zinc-800 last:border-b-0 hover:bg-zinc-900/50 transition"
              >
                <div>
                  <p className="text-sm text-white">{model.name}</p>
                  <p className="text-xs text-zinc-600">
                    v{model.version} &middot; {model.base_model.split("/").pop()}
                  </p>
                </div>
                <DocStatusBadge status={model.deployment_status} />
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
