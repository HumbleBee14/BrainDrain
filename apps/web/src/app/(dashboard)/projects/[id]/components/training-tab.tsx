"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import { useEffect, useMemo, useState } from "react";
import { toast } from "sonner";
import type {
  CreateTrainingJobInput,
  Dataset,
  TrainingJob,
} from "@/lib/api-client";
import {
  useCreateTrainingJob,
  useCancelTrainingJob,
  useEstimateTrainingCost,
} from "@/hooks/use-training";
import { useModelCatalog } from "@/hooks/use-catalog";
import { useOnboarding } from "@/hooks/use-onboarding";
import { Button } from "@/components/ui/button";
import { TrainingStatusBadge } from "./training-status-badge";
import { TrainingModesExplainer } from "./training-modes-explainer";
import { DistillSetup } from "./distill-setup";

export function TrainingTab({
  projectId,
  taskType,
  canDistill,
  datasets,
  allTrainingJobs,
  showTrainForm,
  setShowTrainForm,
}: {
  projectId: string;
  taskType: string;
  canDistill: boolean;
  datasets: Dataset[];
  allTrainingJobs: TrainingJob[];
  showTrainForm: boolean;
  setShowTrainForm: (open: boolean) => void;
}) {
  const router = useRouter();
  const createTrainingJob = useCreateTrainingJob(projectId);
  const cancelTrainingJob = useCancelTrainingJob(projectId);
  const { markStepComplete } = useOnboarding();

  const [trainForm, setTrainForm] = useState<CreateTrainingJobInput>({
    dataset_id: "",
    base_model: "",
    method: "qlora",
    mode: "quick",
  });
  const [jobStatusFilter, setJobStatusFilter] = useState<string>("all");
  const [compareIds, setCompareIds] = useState<string[]>([]);
  const [showDistillSetup, setShowDistillSetup] = useState(false);

  const { data: costEstimate } = useEstimateTrainingCost(projectId, trainForm);
  const {
    data: catalogData,
    isLoading: catalogLoading,
    isError: catalogIsError,
    error: catalogError,
    refetch: refetchCatalog,
  } = useModelCatalog();
  const catalogModels = useMemo(
    () => catalogData?.models ?? [],
    [catalogData],
  );

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

  // Default the base model to the catalog's suggestion once it loads.
  useEffect(() => {
    if (!trainForm.base_model && catalogModels.length > 0) {
      setTrainForm((prev) => ({
        ...prev,
        base_model: catalogData?.suggested ?? catalogModels[0].model_id,
      }));
    }
  }, [catalogModels, catalogData?.suggested, trainForm.base_model]);

  const selectedCatalogModel = useMemo(
    () => catalogModels.find((m) => m.model_id === trainForm.base_model),
    [catalogModels, trainForm.base_model],
  );

  const trainingPresets = useMemo(() => {
    if (catalogModels.length === 0) return [];
    const bySize = [...catalogModels].sort(
      (a, b) => a.vram_4bit_gb - b.vram_4bit_gb,
    );
    const smallest = bySize[0];
    const midRange =
      catalogModels.find(
        (m) => !m.gated && m.model_id !== smallest.model_id,
      ) ?? bySize[Math.min(1, bySize.length - 1)];
    const productionModel =
      [...catalogModels]
        .filter((m) => m.gated)
        .sort((a, b) => b.vram_4bit_gb - a.vram_4bit_gb)[0] ??
      bySize[bySize.length - 1];
    const reasoningModel =
      catalogModels.find((m) => m.recommended_for.includes("reasoning")) ??
      productionModel;

    return [
      {
        label: "Quick Experiment",
        method: "qlora" as const,
        mode: "quick" as const,
        base_model: smallest.model_id,
        desc: `Fastest, ${smallest.display_name}, QLoRA`,
      },
      {
        label: "Balanced",
        method: "qlora" as const,
        mode: "aligned" as const,
        base_model: midRange.model_id,
        desc: `SFT + DPO, ${midRange.display_name}`,
      },
      {
        label: "Production",
        method: "lora" as const,
        mode: "aligned" as const,
        base_model: productionModel.model_id,
        gpu_class: "a10g" as const,
        desc: `${productionModel.display_name}, LoRA, A10G GPU`,
      },
      {
        label: "Max Quality",
        method: "lora" as const,
        mode: "reasoning" as const,
        base_model: reasoningModel.model_id,
        gpu_class: "l40s" as const,
        desc: `${reasoningModel.display_name}, GRPO reasoning, L40S`,
      },
    ];
  }, [catalogModels]);

  const approvedDatasets = datasets.filter((ds) => ds.status === "approved");
  const hasApprovedDatasets = approvedDatasets.length > 0;

  const trainingJobs = allTrainingJobs.filter(
    (job) => jobStatusFilter === "all" || job.status === jobStatusFilter,
  );

  return (
    <div>
      <div className="mb-6">
        <TrainingModesExplainer
          defaultCollapsed={allTrainingJobs.length > 0}
        />
      </div>

      <div className="mb-4 flex flex-col justify-between gap-3 sm:flex-row sm:items-center">
        <p className="text-sm text-zinc-500">
          {hasApprovedDatasets
            ? `${approvedDatasets.length} approved dataset(s) ready to fine-tune on.`
            : "Approve a dataset first — fine-tuning runs on approved datasets only."}
        </p>
        <div className="flex shrink-0 items-center gap-2">
          {compareIds.length >= 2 && (
            <Button
              variant="secondary"
              onClick={() =>
                router.push(
                  `/projects/${projectId}/compare?jobs=${compareIds.slice(0, 2).join(",")}`,
                )
              }
            >
              Compare ({compareIds.length})
            </Button>
          )}
          <Button
            variant="secondary"
            onClick={() => setShowDistillSetup(!showDistillSetup)}
            disabled={!canDistill}
            title={
              canDistill
                ? "Use a big, expensive model to teach a small one you own — with a report proving how close it got"
                : "Upload documents first — the teacher writes its training data from them"
            }
          >
            Distill a Larger Model
          </Button>
          <Button
            onClick={() => setShowTrainForm(!showTrainForm)}
            disabled={!hasApprovedDatasets}
            title={
              hasApprovedDatasets
                ? undefined
                : "Approve a dataset first to start fine-tuning"
            }
          >
            New Fine-Tuning Job
          </Button>
        </div>
      </div>

      {showDistillSetup && (
        <div className="mb-4">
          <DistillSetup
            projectId={projectId}
            taskType={taskType}
            catalogModels={catalogModels}
            suggestedBaseModel={
              catalogData?.suggested ?? catalogModels[0]?.model_id ?? ""
            }
            disabled={!canDistill}
            onStarted={() => setShowDistillSetup(false)}
          />
        </div>
      )}

      {allTrainingJobs.length > 3 && (
        <div className="mb-3 flex gap-2">
          <select
            value={jobStatusFilter}
            onChange={(e) => setJobStatusFilter(e.target.value)}
            className="rounded-lg border border-zinc-300 bg-zinc-50 px-3 py-1.5 text-sm text-zinc-900 dark:border-zinc-700 dark:bg-zinc-900 dark:text-white"
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

      {showTrainForm && (
        <div className="mb-4 space-y-3 rounded-lg border border-zinc-200 p-4 dark:border-zinc-800">
          {trainingPresets.length > 0 && (
            <div>
              <label className="mb-2 block text-xs text-zinc-500">
                Quick Presets
              </label>
              <div className="flex flex-wrap gap-2">
                {trainingPresets.map((preset) => (
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
                    className="rounded-lg border border-zinc-300 px-3 py-1.5 text-xs text-zinc-600 transition hover:border-zinc-500 hover:text-zinc-900 dark:border-zinc-700 dark:text-zinc-400 dark:hover:text-white"
                    title={preset.desc}
                  >
                    {preset.label}
                  </button>
                ))}
              </div>
            </div>
          )}
          <div className="grid grid-cols-1 gap-2 sm:grid-cols-2 md:gap-3 lg:grid-cols-3">
            <div>
              <label className="mb-1 block text-xs text-zinc-500">
                Dataset
              </label>
              <select
                value={trainForm.dataset_id}
                onChange={(e) =>
                  setTrainForm({ ...trainForm, dataset_id: e.target.value })
                }
                className="w-full rounded-lg border border-zinc-300 bg-zinc-50 px-3 py-2 text-sm text-zinc-900 dark:border-zinc-700 dark:bg-zinc-900 dark:text-white"
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
              <label className="mb-1 block text-xs text-zinc-500">
                Base Model
              </label>
              {catalogIsError ? (
                <div className="rounded-lg border border-red-300 bg-red-50 px-3 py-2 text-xs text-red-600 dark:border-red-900 dark:bg-red-950/30 dark:text-red-400">
                  <p>
                    Couldn&apos;t load the model catalog
                    {catalogError instanceof Error
                      ? `: ${catalogError.message}`
                      : "."}
                  </p>
                  <button
                    type="button"
                    onClick={() => refetchCatalog()}
                    className="mt-1 font-medium underline underline-offset-2 hover:no-underline"
                  >
                    Retry
                  </button>
                </div>
              ) : (
                <select
                  value={trainForm.base_model}
                  onChange={(e) =>
                    setTrainForm({
                      ...trainForm,
                      base_model: e.target.value,
                    })
                  }
                  disabled={catalogLoading || catalogModels.length === 0}
                  className="w-full rounded-lg border border-zinc-300 bg-zinc-50 px-3 py-2 text-sm text-zinc-900 disabled:opacity-50 dark:border-zinc-700 dark:bg-zinc-900 dark:text-white"
                >
                  {catalogLoading ? (
                    <option value="">Loading models...</option>
                  ) : (
                    <>
                      <option value="" disabled>
                        Select a base model...
                      </option>
                      {catalogModels.map((m) => (
                        <option key={m.model_id} value={m.model_id}>
                          {m.display_name} &mdash; {m.size}
                          {m.gated ? " (gated)" : ""}
                        </option>
                      ))}
                    </>
                  )}
                </select>
              )}
              {selectedCatalogModel && (
                <p className="mt-1 text-xs text-zinc-500">
                  {selectedCatalogModel.best_for.join(" · ")} &mdash; ~
                  {selectedCatalogModel.vram_4bit_gb}GB VRAM (4-bit)
                  {selectedCatalogModel.gated && " · requires HF token"}
                </p>
              )}
            </div>
            <div>
              <label className="mb-1 block text-xs text-zinc-500">
                Method
              </label>
              <select
                value={trainForm.method}
                onChange={(e) =>
                  setTrainForm({
                    ...trainForm,
                    method: e.target.value as CreateTrainingJobInput["method"],
                  })
                }
                className="w-full rounded-lg border border-zinc-300 bg-zinc-50 px-3 py-2 text-sm text-zinc-900 dark:border-zinc-700 dark:bg-zinc-900 dark:text-white"
              >
                <option value="qlora">QLoRA (4-bit, fastest)</option>
                <option value="lora">LoRA (16-bit)</option>
                <option value="full">Full Fine-tune</option>
              </select>
            </div>
            <div>
              <label className="mb-1 block text-xs text-zinc-500">Mode</label>
              <select
                value={trainForm.mode}
                onChange={(e) =>
                  setTrainForm({
                    ...trainForm,
                    mode: e.target.value as CreateTrainingJobInput["mode"],
                  })
                }
                className="w-full rounded-lg border border-zinc-300 bg-zinc-50 px-3 py-2 text-sm text-zinc-900 dark:border-zinc-700 dark:bg-zinc-900 dark:text-white"
              >
                <option value="quick">Quick (SFT only)</option>
                <option value="aligned">Aligned (SFT + DPO)</option>
                <option value="reasoning">Reasoning (SFT + GRPO)</option>
                <option value="iterative">Iterative (Multi-round)</option>
              </select>
            </div>
            <div>
              <label className="mb-1 block text-xs text-zinc-500">
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
                className="w-full rounded-lg border border-zinc-300 bg-zinc-50 px-3 py-2 text-sm text-zinc-900 dark:border-zinc-700 dark:bg-zinc-900 dark:text-white"
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

          {costEstimate && (
            <div className="rounded-lg border border-zinc-300 bg-zinc-50/50 p-3 text-sm dark:border-zinc-700 dark:bg-zinc-900/50">
              <p className="mb-1 font-medium text-zinc-600 dark:text-zinc-400">
                Estimated Cost
              </p>
              <p className="text-lg font-semibold text-zinc-900 dark:text-white">
                ${costEstimate.cost_estimate.toFixed(2)}
              </p>
              <div className="mt-1 space-y-0.5 text-xs text-zinc-500">
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
            <Button
              onClick={() => {
                if (!trainForm.dataset_id || !trainForm.base_model) return;
                // A refused launch (spend cap, incompatible pair) is reported
                // inside this form, so it stays open until a job exists.
                createTrainingJob.mutate(trainForm, {
                  onSuccess: () => setShowTrainForm(false),
                });
              }}
              disabled={!trainForm.dataset_id || !trainForm.base_model}
              loading={createTrainingJob.isPending}
            >
              {createTrainingJob.isPending
                ? "Starting..."
                : costEstimate
                  ? `Start Fine-Tuning (~$${costEstimate.cost_estimate.toFixed(2)})`
                  : "Start Fine-Tuning"}
            </Button>
            <Button variant="secondary" onClick={() => setShowTrainForm(false)}>
              Cancel
            </Button>
          </div>
          {createTrainingJob.isError && (
            <div className="rounded-lg border border-red-200 bg-red-50 p-3 dark:border-red-900/50 dark:bg-red-900/10">
              <p className="text-sm text-red-600 dark:text-red-400">
                {createTrainingJob.error.message}
              </p>
            </div>
          )}
        </div>
      )}

      {trainingJobs.length > 0 && (
        <div className="rounded-lg border border-zinc-200 dark:border-zinc-800">
          {trainingJobs.map((job) => (
            <div
              key={job.id}
              className="flex items-center border-b border-zinc-200 last:border-b-0 dark:border-zinc-800"
            >
              {allTrainingJobs.length >= 2 && (
                <label
                  className="flex cursor-pointer items-center pl-4"
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
                    className="h-3.5 w-3.5 rounded border-zinc-300 bg-white text-violet-500 focus:ring-violet-500 focus:ring-offset-0 dark:border-zinc-600 dark:bg-zinc-900"
                  />
                </label>
              )}
              <Link
                href={`/projects/${projectId}/training/${job.id}`}
                className="flex flex-1 items-center justify-between px-4 py-3 transition hover:bg-zinc-50/50 dark:hover:bg-zinc-900/50"
              >
                <div>
                  <p className="text-sm text-zinc-900 dark:text-white">
                    {job.base_model.split("/").pop()} &mdash; {job.mode}
                  </p>
                  <p className="text-xs text-zinc-400 dark:text-zinc-600">
                    {job.method.toUpperCase()}
                    {job.cost_estimate != null &&
                      ` · ~$${job.cost_estimate.toFixed(2)}`}
                    {" · "}
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
                      className="text-xs text-red-500 transition hover:text-red-400"
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
        <p className="text-sm text-zinc-400 dark:text-zinc-600">
          {allTrainingJobs.length > 0 && jobStatusFilter !== "all"
            ? "No fine-tuning jobs match the current filter."
            : hasApprovedDatasets
              ? 'No fine-tuning jobs yet. Click "New Fine-Tuning Job" to begin.'
              : "Approve a dataset first to start fine-tuning."}
        </p>
      )}
    </div>
  );
}
