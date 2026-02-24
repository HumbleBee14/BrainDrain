"use client";

import { useParams, useRouter } from "next/navigation";
import Link from "next/link";
import { toast } from "sonner";
import { useProject, useDeleteProject } from "@/hooks/use-projects";
import { useDocuments, useUploadDocuments } from "@/hooks/use-documents";
import {
  usePipelineStatus,
  useTriggerParse,
  useTriggerRefine,
  useTriggerFullPipeline,
} from "@/hooks/use-pipeline";
import { useDatasets } from "@/hooks/use-datasets";
import {
  useTrainingJobs,
  useCreateTrainingJob,
  useCancelTrainingJob,
  useEstimateTrainingCost,
} from "@/hooks/use-training";
import type { CreateTrainingJobInput } from "@/lib/api-client";
import { useModels } from "@/hooks/use-models";
import { useCallback, useEffect, useRef, useState } from "react";
import { useOnboarding } from "@/hooks/use-onboarding";
import { Breadcrumbs } from "@/components/breadcrumbs";
import {
  StatusBadge,
  DatasetStatusBadge,
  DeploymentStatusBadge,
  TrainingStatusBadge,
  DocumentRow,
  PipelineStageCard,
} from "./components";

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
  const { data: docsData } = useDocuments(
    params.id,
    0,
    50,
    isActive ? 3000 : false,
  );
  const { data: datasetsData } = useDatasets(params.id);
  const { data: trainingJobsData } = useTrainingJobs(params.id);
  const { data: modelsData } = useModels(params.id);
  const uploadDocs = useUploadDocuments(params.id);
  const triggerParse = useTriggerParse(params.id);
  const triggerRefine = useTriggerRefine(params.id);
  const triggerFullPipeline = useTriggerFullPipeline(params.id);
  const createTrainingJob = useCreateTrainingJob(params.id);
  const cancelTrainingJob = useCancelTrainingJob(params.id);
  const { markStepComplete } = useOnboarding();

  // Track onboarding steps + show toast notifications when mutations succeed/fail
  useEffect(() => {
    if (uploadDocs.isSuccess) {
      markStepComplete("upload_document");
      toast.success(`${uploadDocs.data.length} file(s) uploaded successfully`);
    }
  }, [uploadDocs.isSuccess, uploadDocs.data, markStepComplete]);

  useEffect(() => {
    if (uploadDocs.isError) toast.error(uploadDocs.error.message);
  }, [uploadDocs.isError, uploadDocs.error]);

  useEffect(() => {
    if (triggerParse.isSuccess) {
      markStepComplete("parse_documents");
      toast.success(
        `Parse started for ${triggerParse.data.document_count} documents`,
      );
    }
  }, [triggerParse.isSuccess, triggerParse.data, markStepComplete]);

  useEffect(() => {
    if (triggerParse.isError) toast.error(triggerParse.error.message);
  }, [triggerParse.isError, triggerParse.error]);

  useEffect(() => {
    if (triggerRefine.isSuccess) {
      markStepComplete("generate_data");
      toast.success(
        `Refine started for ${triggerRefine.data.document_count} documents`,
      );
    }
  }, [triggerRefine.isSuccess, triggerRefine.data, markStepComplete]);

  useEffect(() => {
    if (triggerRefine.isError) toast.error(triggerRefine.error.message);
  }, [triggerRefine.isError, triggerRefine.error]);

  useEffect(() => {
    if (createTrainingJob.isSuccess) {
      markStepComplete("start_training");
      toast.success("Training job created");
    }
  }, [createTrainingJob.isSuccess, markStepComplete]);

  useEffect(() => {
    if (createTrainingJob.isError) toast.error(createTrainingJob.error.message);
  }, [createTrainingJob.isError, createTrainingJob.error]);

  useEffect(() => {
    if (cancelTrainingJob.isSuccess) toast.success("Training job cancelled");
  }, [cancelTrainingJob.isSuccess]);

  useEffect(() => {
    if (cancelTrainingJob.isError) toast.error(cancelTrainingJob.error.message);
  }, [cancelTrainingJob.isError, cancelTrainingJob.error]);

  useEffect(() => {
    if (triggerFullPipeline.isSuccess)
      toast.success(
        `Full pipeline started for ${triggerFullPipeline.data.document_count} documents`,
      );
  }, [triggerFullPipeline.isSuccess, triggerFullPipeline.data]);

  useEffect(() => {
    if (triggerFullPipeline.isError)
      toast.error(triggerFullPipeline.error.message);
  }, [triggerFullPipeline.isError, triggerFullPipeline.error]);

  const [confirmDelete, setConfirmDelete] = useState(false);
  const [isDragging, setIsDragging] = useState(false);
  const [showTrainForm, setShowTrainForm] = useState(false);
  const [trainForm, setTrainForm] = useState<CreateTrainingJobInput>({
    dataset_id: "",
    base_model: "unsloth/Llama-3.2-1B-Instruct",
    method: "qlora",
    mode: "quick",
  });
  const fileInputRef = useRef<HTMLInputElement>(null);
  const { data: costEstimate } = useEstimateTrainingCost(params.id, trainForm);

  // Search/filter state
  const [docSearch, setDocSearch] = useState("");
  const [docStatusFilter, setDocStatusFilter] = useState<string>("all");
  const [jobStatusFilter, setJobStatusFilter] = useState<string>("all");
  const [compareIds, setCompareIds] = useState<string[]>([]);

  const allDocuments = docsData?.data ?? [];
  const datasets = datasetsData?.data ?? [];
  const allTrainingJobs = trainingJobsData?.data ?? [];

  // Filter documents
  const documents = allDocuments.filter((doc) => {
    const matchesSearch =
      !docSearch ||
      doc.filename.toLowerCase().includes(docSearch.toLowerCase());
    const matchesStatus =
      docStatusFilter === "all" || doc.status === docStatusFilter;
    return matchesSearch && matchesStatus;
  });

  // Filter training jobs
  const trainingJobs = allTrainingJobs.filter((job) => {
    return jobStatusFilter === "all" || job.status === jobStatusFilter;
  });
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
    try {
      await deleteProject.mutateAsync(params.id);
      router.push("/projects");
    } catch {
      // Error is captured by React Query and surfaced via deleteProject.isError
      setConfirmDelete(false);
    }
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
        <Link
          href="/projects"
          className="text-sm text-white underline hover:no-underline"
        >
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
        <Breadcrumbs
          items={[
            { label: "Projects", href: "/projects" },
            { label: project.name },
          ]}
        />
        <div className="flex items-center gap-3">
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
          <h2 className="text-lg font-semibold text-white mb-4">
            Pipeline Status
          </h2>
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
              onClick={() =>
                triggerRefine.mutate({
                  taskType: project.task_type || "question_answering",
                })
              }
              disabled={!hasParsed || triggerRefine.isPending || isGenerating}
              className="rounded-lg bg-emerald-600 px-4 py-2 text-sm font-medium text-white hover:bg-emerald-500 transition disabled:opacity-50 disabled:cursor-not-allowed"
            >
              {triggerRefine.isPending
                ? "Starting..."
                : isGenerating
                  ? "Generating..."
                  : "Generate Training Data"}
            </button>
            <button
              onClick={() =>
                triggerFullPipeline.mutate({
                  task_type: project.task_type || "question_answering",
                  base_model: "unsloth/Llama-3.2-1B-Instruct",
                  training_config: {
                    method: "qlora",
                    mode: "quick",
                  },
                })
              }
              disabled={
                (!hasUploaded && !hasParsed) || triggerFullPipeline.isPending
              }
              className="rounded-lg bg-violet-600 px-4 py-2 text-sm font-medium text-white hover:bg-violet-500 transition disabled:opacity-50 disabled:cursor-not-allowed"
              title="Run the entire pipeline: parse → generate data → train → evaluate"
            >
              {triggerFullPipeline.isPending
                ? "Starting..."
                : "One-Click Fine-Tune"}
            </button>
          </div>

          {triggerParse.isError && (
            <p className="text-sm text-red-400 mt-2">
              {triggerParse.error.message}
            </p>
          )}
          {triggerRefine.isError && (
            <p className="text-sm text-red-400 mt-2">
              {triggerRefine.error.message}
            </p>
          )}
          {triggerParse.isSuccess && (
            <p className="text-sm text-emerald-400 mt-2">
              Parse workflow started for {triggerParse.data.document_count}{" "}
              documents
            </p>
          )}
          {triggerRefine.isSuccess && (
            <p className="text-sm text-emerald-400 mt-2">
              Refine workflow started for {triggerRefine.data.document_count}{" "}
              documents
            </p>
          )}
        </div>
      )}

      {/* Upload area */}
      <div className="mb-8">
        <div className="flex items-center justify-between mb-4">
          <h2 className="text-lg font-semibold text-white">
            Documents{" "}
            {allDocuments.length > 0 &&
              `(${documents.length}${documents.length !== allDocuments.length ? ` of ${allDocuments.length}` : ""})`}
          </h2>
        </div>
        {/* Document search & filter */}
        {allDocuments.length > 3 && (
          <div className="flex gap-2 mb-3">
            <input
              value={docSearch}
              onChange={(e) => setDocSearch(e.target.value)}
              placeholder="Search documents..."
              className="flex-1 rounded-lg border border-zinc-700 bg-zinc-900 px-3 py-1.5 text-sm text-white placeholder:text-zinc-600"
            />
            <select
              value={docStatusFilter}
              onChange={(e) => setDocStatusFilter(e.target.value)}
              className="rounded-lg border border-zinc-700 bg-zinc-900 px-3 py-1.5 text-sm text-white"
            >
              <option value="all">All statuses</option>
              <option value="uploaded">Uploaded</option>
              <option value="parsing">Parsing</option>
              <option value="parsed">Parsed</option>
              <option value="failed">Failed</option>
            </select>
          </div>
        )}
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
                {isDragging
                  ? "Drop files here"
                  : "Drag and drop files here or click to upload"}
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
            <p className="text-sm text-red-400 mt-2">
              {uploadDocs.error.message}
            </p>
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
                    {ds.pair_count != null
                      ? `${ds.pair_count} pairs`
                      : "Generating..."}
                    {" \u00b7 "}
                    {ds.format}
                  </p>
                </div>
                <DatasetStatusBadge status={ds.status} />
              </Link>
            ))}
          </div>
        </div>
      )}

      {/* Training Jobs section */}
      <div className="mb-8">
        <div className="flex items-center justify-between mb-4">
          <h2 className="text-lg font-semibold text-white">
            Training Jobs{" "}
            {allTrainingJobs.length > 0 &&
              `(${trainingJobs.length}${trainingJobs.length !== allTrainingJobs.length ? ` of ${allTrainingJobs.length}` : ""})`}
          </h2>
          <div className="flex items-center gap-2">
            {compareIds.length >= 2 && (
              <button
                onClick={() =>
                  router.push(
                    `/projects/${params.id}/compare?jobs=${compareIds.slice(0, 2).join(",")}`,
                  )
                }
                className="rounded-lg border border-blue-700 bg-blue-600/10 px-4 py-2 text-sm font-medium text-blue-400 hover:bg-blue-600/20 transition"
              >
                Compare ({compareIds.length})
              </button>
            )}
            <button
              onClick={() => setShowTrainForm(!showTrainForm)}
              disabled={!hasApprovedDatasets}
              className="rounded-lg bg-violet-600 px-4 py-2 text-sm font-medium text-white hover:bg-violet-500 transition disabled:opacity-50 disabled:cursor-not-allowed"
            >
              Start Training
            </button>
          </div>
        </div>

        {/* Training jobs filter */}
        {allTrainingJobs.length > 3 && (
          <div className="flex gap-2 mb-3">
            <select
              value={jobStatusFilter}
              onChange={(e) => setJobStatusFilter(e.target.value)}
              className="rounded-lg border border-zinc-700 bg-zinc-900 px-3 py-1.5 text-sm text-white"
            >
              <option value="all">All statuses</option>
              <option value="pending">Pending</option>
              <option value="training">Training</option>
              <option value="completed">Completed</option>
              <option value="failed">Failed</option>
              <option value="cancelled">Cancelled</option>
            </select>
          </div>
        )}

        {/* Training form */}
        {showTrainForm && (
          <div className="rounded-lg border border-zinc-800 p-4 mb-4 space-y-3">
            {/* Hyperparameter presets */}
            <div>
              <label className="block text-xs text-zinc-500 mb-2">
                Quick Presets
              </label>
              <div className="flex gap-2">
                {[
                  {
                    label: "Quick Experiment",
                    method: "qlora" as const,
                    mode: "quick" as const,
                    base_model: "unsloth/Llama-3.2-1B-Instruct",
                    desc: "Fastest, smallest model, QLoRA",
                  },
                  {
                    label: "Balanced",
                    method: "qlora" as const,
                    mode: "aligned" as const,
                    base_model: "unsloth/Llama-3.2-3B-Instruct",
                    desc: "SFT + DPO, 3B model",
                  },
                  {
                    label: "Production",
                    method: "lora" as const,
                    mode: "aligned" as const,
                    base_model: "unsloth/Meta-Llama-3.1-8B-Instruct",
                    gpu_class: "a10g" as const,
                    desc: "8B, LoRA, A10G GPU",
                  },
                  {
                    label: "Max Quality",
                    method: "lora" as const,
                    mode: "reasoning" as const,
                    base_model: "unsloth/Meta-Llama-3.1-8B-Instruct",
                    gpu_class: "l40s" as const,
                    desc: "8B, GRPO reasoning, L40S",
                  },
                ].map((preset) => (
                  <button
                    key={preset.label}
                    type="button"
                    onClick={() =>
                      setTrainForm({
                        ...trainForm,
                        method: preset.method,
                        mode: preset.mode,
                        base_model: preset.base_model,
                        gpu_class: preset.gpu_class,
                      })
                    }
                    className="rounded-lg border border-zinc-700 px-3 py-1.5 text-xs text-zinc-400 hover:border-zinc-500 hover:text-white transition"
                    title={preset.desc}
                  >
                    {preset.label}
                  </button>
                ))}
              </div>
            </div>
            <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-3">
              <div>
                <label className="block text-xs text-zinc-500 mb-1">
                  Dataset
                </label>
                <select
                  value={trainForm.dataset_id}
                  onChange={(e) =>
                    setTrainForm({ ...trainForm, dataset_id: e.target.value })
                  }
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
                <label className="block text-xs text-zinc-500 mb-1">
                  Base Model
                </label>
                <select
                  value={trainForm.base_model}
                  onChange={(e) =>
                    setTrainForm({ ...trainForm, base_model: e.target.value })
                  }
                  className="w-full rounded-lg border border-zinc-700 bg-zinc-900 px-3 py-2 text-sm text-white"
                >
                  <option value="unsloth/Llama-3.2-1B-Instruct">
                    Llama 3.2 1B Instruct
                  </option>
                  <option value="unsloth/Llama-3.2-3B-Instruct">
                    Llama 3.2 3B Instruct
                  </option>
                  <option value="unsloth/Meta-Llama-3.1-8B-Instruct">
                    Llama 3.1 8B Instruct
                  </option>
                  <option value="unsloth/Qwen2.5-7B-Instruct">
                    Qwen 2.5 7B Instruct
                  </option>
                  <option value="unsloth/Mistral-7B-Instruct-v0.3">
                    Mistral 7B Instruct v0.3
                  </option>
                  <option value="unsloth/gemma-2-2b-it">Gemma 2 2B IT</option>
                </select>
              </div>
              <div>
                <label className="block text-xs text-zinc-500 mb-1">
                  Method
                </label>
                <select
                  value={trainForm.method}
                  onChange={(e) =>
                    setTrainForm({
                      ...trainForm,
                      method: e.target
                        .value as CreateTrainingJobInput["method"],
                    })
                  }
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
                  onChange={(e) =>
                    setTrainForm({
                      ...trainForm,
                      mode: e.target.value as CreateTrainingJobInput["mode"],
                    })
                  }
                  className="w-full rounded-lg border border-zinc-700 bg-zinc-900 px-3 py-2 text-sm text-white"
                >
                  <option value="quick">Quick (SFT only)</option>
                  <option value="aligned">Aligned (SFT + DPO)</option>
                  <option value="reasoning">Reasoning (SFT + GRPO)</option>
                  <option value="iterative">Iterative (Multi-round)</option>
                </select>
              </div>
              <div>
                <label className="block text-xs text-zinc-500 mb-1">
                  GPU Class
                </label>
                <select
                  value={trainForm.gpu_class ?? ""}
                  onChange={(e) =>
                    setTrainForm({
                      ...trainForm,
                      gpu_class: e.target.value || undefined,
                    })
                  }
                  className="w-full rounded-lg border border-zinc-700 bg-zinc-900 px-3 py-2 text-sm text-white"
                >
                  <option value="">Auto (default)</option>
                  <option value="t4">T4 (budget, small models)</option>
                  <option value="a10g">A10G (7B-13B LoRA)</option>
                  <option value="l40s">L40S (13B-30B)</option>
                  <option value="a10040gb">A100 40GB (30B+)</option>
                  <option value="a10080gb">A100 80GB (large models)</option>
                  <option value="h100">H100 (max throughput)</option>
                </select>
              </div>
            </div>
            {/* Cost estimate breakdown */}
            {costEstimate && (
              <div className="rounded-lg border border-zinc-700 bg-zinc-900/50 p-3 text-sm">
                <p className="text-zinc-400 font-medium mb-1">Estimated Cost</p>
                <p className="text-white text-lg font-semibold">
                  ${costEstimate.cost_estimate.toFixed(2)}
                </p>
                <div className="mt-1 text-xs text-zinc-500 space-y-0.5">
                  <p>
                    GPU: {costEstimate.gpu_class.toUpperCase()} ($
                    {costEstimate.gpu_rate_per_hour.toFixed(2)}/hr)
                  </p>
                  <p>
                    Duration: ~{costEstimate.estimated_hours.toFixed(1)} hours
                  </p>
                  <p>
                    Mode: {trainForm.mode}{" "}
                    {trainForm.mode === "aligned"
                      ? "(SFT + DPO)"
                      : trainForm.mode === "reasoning"
                        ? "(SFT + GRPO)"
                        : trainForm.mode === "iterative"
                          ? "(multi-round)"
                          : "(SFT only)"}
                  </p>
                </div>
              </div>
            )}

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
                {createTrainingJob.isPending
                  ? "Starting..."
                  : costEstimate
                    ? `Start Training (~$${costEstimate.cost_estimate.toFixed(2)})`
                    : "Start Training Job"}
              </button>
              <button
                onClick={() => setShowTrainForm(false)}
                className="rounded-lg border border-zinc-700 px-4 py-2 text-sm text-zinc-400 hover:border-zinc-600 transition"
              >
                Cancel
              </button>
            </div>
            {createTrainingJob.isError && (
              <p className="text-sm text-red-400">
                {createTrainingJob.error.message}
              </p>
            )}
          </div>
        )}

        {/* Training jobs list */}
        {trainingJobs.length > 0 && (
          <div className="rounded-lg border border-zinc-800">
            {trainingJobs.map((job) => (
              <div
                key={job.id}
                className="flex items-center border-b border-zinc-800 last:border-b-0"
              >
                {allTrainingJobs.length >= 2 && (
                  <label
                    className="flex items-center pl-4 cursor-pointer"
                    onClick={(e) => e.stopPropagation()}
                  >
                    <input
                      type="checkbox"
                      checked={compareIds.includes(job.id)}
                      onChange={() =>
                        setCompareIds((prev) =>
                          prev.includes(job.id)
                            ? prev.filter((id) => id !== job.id)
                            : [...prev, job.id].slice(-2),
                        )
                      }
                      className="h-3.5 w-3.5 rounded border-zinc-600 bg-zinc-900 text-violet-500 focus:ring-violet-500 focus:ring-offset-0"
                    />
                  </label>
                )}
                <Link
                  href={`/projects/${params.id}/training/${job.id}`}
                  className="flex-1 flex items-center justify-between py-3 px-4 hover:bg-zinc-900/50 transition"
                >
                  <div>
                    <p className="text-sm text-white">
                      {job.base_model.split("/").pop()} &mdash; {job.mode}
                    </p>
                    <p className="text-xs text-zinc-600">
                      {job.method.toUpperCase()}
                      {job.cost_estimate != null &&
                        ` \u00b7 ~$${job.cost_estimate.toFixed(2)}`}
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
              </div>
            ))}
          </div>
        )}

        {trainingJobs.length === 0 && !showTrainForm && (
          <p className="text-sm text-zinc-600">
            {allTrainingJobs.length > 0 && jobStatusFilter !== "all"
              ? "No training jobs match the current filter."
              : hasApprovedDatasets
                ? 'No training jobs yet. Click "Start Training" to begin.'
                : "Approve a dataset first to start training."}
          </p>
        )}
      </div>

      {/* Models section */}
      {models.length > 0 && (
        <div className="mb-8">
          <div className="flex items-center justify-between mb-4">
            <h2 className="text-lg font-semibold text-white">
              Models ({modelsData?.total ?? models.length})
            </h2>
            {models.filter((m) => m.deployment_status === "active").length >=
              1 && (
              <Link
                href={`/projects/${params.id}/playground`}
                className="rounded-lg border border-blue-700 bg-blue-600/10 px-4 py-2 text-sm font-medium text-blue-400 hover:bg-blue-600/20 transition"
              >
                A/B Playground
              </Link>
            )}
          </div>
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
                    v{model.version} &middot;{" "}
                    {model.base_model.split("/").pop()}
                  </p>
                </div>
                <DeploymentStatusBadge status={model.deployment_status} />
              </Link>
            ))}
          </div>
        </div>
      )}

      {/* Info grid */}
      <div className="grid grid-cols-1 md:grid-cols-3 gap-4 mb-8">
        <div className="rounded-lg border border-zinc-800 p-4">
          <p className="text-xs text-zinc-500 uppercase tracking-wider">
            Task Type
          </p>
          <p className="text-white mt-1">{project.task_type || "Not set"}</p>
        </div>
        <div className="rounded-lg border border-zinc-800 p-4">
          <p className="text-xs text-zinc-500 uppercase tracking-wider">
            Created
          </p>
          <p className="text-white mt-1">
            {new Date(project.created_at).toLocaleDateString()}
          </p>
        </div>
        <div className="rounded-lg border border-zinc-800 p-4">
          <p className="text-xs text-zinc-500 uppercase tracking-wider">
            Updated
          </p>
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
            <p className="text-xs text-zinc-600">
              This action cannot be undone.
            </p>
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
