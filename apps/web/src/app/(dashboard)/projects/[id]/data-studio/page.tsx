"use client";

import { useEffect, useState } from "react";
import { useParams, useRouter } from "next/navigation";
import Link from "next/link";
import { toast } from "sonner";
import {
  useDataGuide,
  useCreateDataGuide,
  useGenerateFacets,
  useUpdateFacets,
  useGeneratePreview,
  useRateSamples,
  useRefineGuidance,
  useUpdateGuidance,
  useGenerateDataset,
  RUNNING_STATUSES,
} from "@/hooks/use-data-guides";
import { useProject } from "@/hooks/use-projects";
import { ApiClientError } from "@/lib/api-client";
import type { DataGuideStatus, Facet, PreviewSample, SampleRating, TaskType } from "@/lib/api-client";
import { Breadcrumbs } from "@/components/breadcrumbs";

const TASK_TYPE_OPTIONS: { value: TaskType; label: string }[] = [
  { value: "question_answering", label: "Question Answering" },
  { value: "instruction_following", label: "Instruction Following" },
  { value: "reasoning", label: "Reasoning" },
  { value: "custom", label: "Custom" },
];

function validTaskType(value: string | null | undefined): TaskType | undefined {
  return TASK_TYPE_OPTIONS.some((o) => o.value === value)
    ? (value as TaskType)
    : undefined;
}

function StatusPill({ status }: { status: DataGuideStatus }) {
  const style = RUNNING_STATUSES.includes(status)
    ? "border-blue-200 bg-blue-50 text-blue-700 dark:border-blue-700 dark:bg-blue-900/30 dark:text-blue-400"
    : status === "completed" || status === "ready" || status === "facets_ready"
      ? "border-emerald-200 bg-emerald-50 text-emerald-700 dark:border-emerald-700 dark:bg-emerald-900/30 dark:text-emerald-400"
      : status === "failed"
        ? "border-red-200 bg-red-50 text-red-700 dark:border-red-700 dark:bg-red-900/30 dark:text-red-400"
        : "border-zinc-300 bg-zinc-100 text-zinc-600 dark:border-zinc-700 dark:bg-zinc-800 dark:text-zinc-400";

  return (
    <span
      className={`inline-flex items-center rounded-full border px-2.5 py-0.5 text-xs font-medium ${style}`}
    >
      {status.replace(/_/g, " ")}
    </span>
  );
}

function FacetChip({
  facet,
  disabled,
  onToggle,
}: {
  facet: Facet;
  disabled: boolean;
  onToggle: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onToggle}
      disabled={disabled}
      className={`rounded-full border px-3 py-1.5 text-sm transition disabled:cursor-not-allowed disabled:opacity-60 ${
        facet.keep
          ? "border-violet-300 bg-violet-50 text-violet-700 dark:border-violet-700 dark:bg-violet-900/30 dark:text-violet-400"
          : "border-zinc-300 bg-zinc-100 text-zinc-400 line-through dark:border-zinc-700 dark:bg-zinc-800 dark:text-zinc-600"
      }`}
    >
      {facet.label}
    </button>
  );
}

function PreviewCard({
  sample,
  index,
  disabled,
  onRate,
}: {
  sample: PreviewSample;
  index: number;
  disabled: boolean;
  onRate: (rating: SampleRating) => void;
}) {
  return (
    <div className="rounded-lg border border-zinc-200 dark:border-zinc-800 p-4">
      <div className="flex items-center justify-between mb-3">
        <span className="text-xs text-zinc-400 dark:text-zinc-600">
          Sample #{index + 1}
        </span>
        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={() => onRate("realistic")}
            disabled={disabled}
            className={`rounded-lg border px-2.5 py-1 text-xs transition disabled:cursor-not-allowed disabled:opacity-50 ${
              sample.rating === "realistic"
                ? "border-emerald-300 bg-emerald-50 text-emerald-700 dark:border-emerald-700 dark:bg-emerald-900/30 dark:text-emerald-400"
                : "border-zinc-300 dark:border-zinc-700 text-zinc-500 hover:border-emerald-400"
            }`}
          >
            👍 Realistic
          </button>
          <button
            type="button"
            onClick={() => onRate("needs_work")}
            disabled={disabled}
            className={`rounded-lg border px-2.5 py-1 text-xs transition disabled:cursor-not-allowed disabled:opacity-50 ${
              sample.rating === "needs_work"
                ? "border-yellow-300 bg-yellow-50 text-yellow-700 dark:border-yellow-700 dark:bg-yellow-900/30 dark:text-yellow-400"
                : "border-zinc-300 dark:border-zinc-700 text-zinc-500 hover:border-yellow-400"
            }`}
          >
            👎 Needs Work
          </button>
        </div>
      </div>
      <div className="mb-3">
        <p className="text-xs text-zinc-500 uppercase tracking-wider mb-1">
          Prompt
        </p>
        <p className="text-sm text-zinc-900 dark:text-white bg-zinc-50 dark:bg-zinc-900 rounded p-2">
          {sample.prompt}
        </p>
      </div>
      <div>
        <p className="text-xs text-zinc-500 uppercase tracking-wider mb-1">
          Response
        </p>
        <p className="text-sm text-zinc-700 dark:text-zinc-300 bg-zinc-50 dark:bg-zinc-900 rounded p-2 whitespace-pre-wrap">
          {sample.response}
        </p>
      </div>
    </div>
  );
}

export default function DataStudioPage() {
  const params = useParams<{ id: string }>();
  const router = useRouter();
  const projectId = params.id;

  const { data: project } = useProject(projectId);
  const { data: guide, isLoading: loadingGuide, error: guideError } =
    useDataGuide(projectId);

  const createGuide = useCreateDataGuide(projectId);
  const generateFacets = useGenerateFacets(projectId);
  const updateFacets = useUpdateFacets(projectId);
  const generatePreview = useGeneratePreview(projectId);
  const rateSamples = useRateSamples(projectId);
  const refineGuidance = useRefineGuidance(projectId);
  const updateGuidance = useUpdateGuidance(projectId);
  const generateDataset = useGenerateDataset(projectId);

  const [taskType, setTaskType] = useState<TaskType>("question_answering");
  const [initialGuidance, setInitialGuidance] = useState("");
  const [localGuidance, setLocalGuidance] = useState("");
  const [guidanceDirty, setGuidanceDirty] = useState(false);
  const [localFacets, setLocalFacets] = useState<Facet[]>([]);

  const notFound =
    guideError instanceof ApiClientError && guideError.status === 404;

  // Default the task-type picker from the project's own task type, once loaded.
  useEffect(() => {
    const validated = validTaskType(project?.task_type);
    if (validated) setTaskType(validated);
  }, [project?.task_type]);

  // Keep the guidance textarea in sync with the server unless the user is
  // actively editing it (avoids clobbering in-progress edits on refetch).
  const facetsKey = guide?.facets.map((f) => `${f.id}:${f.keep}`).join(",") ?? "";
  useEffect(() => {
    if (guide && !guidanceDirty) setLocalGuidance(guide.guidance);
  }, [guide, guidanceDirty]);

  useEffect(() => {
    if (guide) setLocalFacets(guide.facets);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [guide?.id, facetsKey]);

  useEffect(() => {
    if (updateGuidance.isSuccess) setGuidanceDirty(false);
  }, [updateGuidance.isSuccess]);

  // Toasts for mutation outcomes.
  useEffect(() => {
    if (createGuide.isError) toast.error(createGuide.error.message);
  }, [createGuide.isError, createGuide.error]);

  useEffect(() => {
    if (generateFacets.isSuccess) toast.success("Facet generation started");
  }, [generateFacets.isSuccess]);
  useEffect(() => {
    if (generateFacets.isError) toast.error(generateFacets.error.message);
  }, [generateFacets.isError, generateFacets.error]);

  useEffect(() => {
    if (updateFacets.isSuccess) toast.success("Facet selections saved");
  }, [updateFacets.isSuccess]);
  useEffect(() => {
    if (updateFacets.isError) toast.error(updateFacets.error.message);
  }, [updateFacets.isError, updateFacets.error]);

  useEffect(() => {
    if (generatePreview.isSuccess) toast.success("Preview generation started");
  }, [generatePreview.isSuccess]);
  useEffect(() => {
    if (generatePreview.isError) toast.error(generatePreview.error.message);
  }, [generatePreview.isError, generatePreview.error]);

  useEffect(() => {
    if (rateSamples.isError) toast.error(rateSamples.error.message);
  }, [rateSamples.isError, rateSamples.error]);

  useEffect(() => {
    if (refineGuidance.isSuccess)
      toast.success("Guidance refinement started");
  }, [refineGuidance.isSuccess]);
  useEffect(() => {
    if (refineGuidance.isError) toast.error(refineGuidance.error.message);
  }, [refineGuidance.isError, refineGuidance.error]);

  useEffect(() => {
    if (updateGuidance.isSuccess) toast.success("Guidance saved");
  }, [updateGuidance.isSuccess]);
  useEffect(() => {
    if (updateGuidance.isError) toast.error(updateGuidance.error.message);
  }, [updateGuidance.isError, updateGuidance.error]);

  useEffect(() => {
    if (generateDataset.isSuccess && generateDataset.data) {
      toast.success("Dataset generation started");
      const datasetId = generateDataset.data.dataset_id;
      if (datasetId) {
        router.push(`/projects/${projectId}/dataset?datasetId=${datasetId}`);
      } else {
        router.push(`/projects/${projectId}`);
      }
    }
  }, [generateDataset.isSuccess, generateDataset.data, projectId, router]);
  useEffect(() => {
    if (generateDataset.isError) toast.error(generateDataset.error.message);
  }, [generateDataset.isError, generateDataset.error]);

  const handleStartSession = async () => {
    try {
      const created = await createGuide.mutateAsync({ task_type: taskType });
      if (initialGuidance.trim()) {
        await updateGuidance.mutateAsync({
          id: created.id,
          data: { guidance: initialGuidance.trim() },
        });
      }
    } catch {
      // Surfaced via createGuide.isError / updateGuidance.isError above.
    }
  };

  const breadcrumbItems = [
    { label: "Projects", href: "/projects" },
    { label: project?.name || "Project", href: `/projects/${projectId}` },
    { label: "Data Studio" },
  ];

  if (loadingGuide) {
    return (
      <div className="flex items-center justify-center py-20">
        <p className="text-zinc-500">Loading Data Studio...</p>
      </div>
    );
  }

  if (guideError && !notFound) {
    return (
      <div className="flex flex-col items-center justify-center py-20 gap-4">
        <p className="text-zinc-500">
          Could not load the Data Studio session: {guideError.message}
        </p>
        <Link
          href={`/projects/${projectId}`}
          className="text-sm text-zinc-900 dark:text-white underline hover:no-underline"
        >
          Back to Project
        </Link>
      </div>
    );
  }

  const isRunning = !!guide && RUNNING_STATUSES.includes(guide.status);
  const canGenerateFacets =
    !!guide && ["draft", "facets_ready", "ready"].includes(guide.status);
  const canEditFacets = guide?.status === "facets_ready";
  const facetsDirty =
    JSON.stringify(localFacets) !== JSON.stringify(guide?.facets ?? []);
  // Preview generation reads the server-persisted facets, so gate on those
  // (not the unsaved local chip state) and require a save first if dirty.
  const hasKeptFacet = (guide?.facets ?? []).some((f) => f.keep);
  const canGeneratePreview =
    !!guide &&
    ["facets_ready", "ready"].includes(guide.status) &&
    hasKeptFacet &&
    !facetsDirty;
  const canRate = guide?.status === "ready";
  const hasRatedSample = (guide?.preview_samples ?? []).some(
    (s) => s.rating !== null,
  );
  const canRefine = guide?.status === "ready" && hasRatedSample;
  const canEditGuidance =
    !!guide && ["draft", "facets_ready", "ready"].includes(guide.status);
  const canGenerateDataset = guide?.status === "ready";

  return (
    <div>
      {/* Header */}
      <div className="mb-8">
        <Breadcrumbs items={breadcrumbItems} />
        <div className="flex items-center gap-3">
          <h1 className="text-xl md:text-2xl font-bold text-zinc-900 dark:text-white">
            Data Studio
          </h1>
          {guide && <StatusPill status={guide.status} />}
        </div>
        <p className="text-zinc-500 mt-1">
          Guided synthetic data generation: review facets, preview samples,
          refine guidance, then generate the full dataset.
        </p>
      </div>

      {notFound && !guide && (
        <div className="rounded-lg border border-zinc-200 dark:border-zinc-800 p-6 max-w-xl">
          <h2 className="text-lg font-semibold text-zinc-900 dark:text-white mb-4">
            Start a guided session
          </h2>
          <label className="block text-xs text-zinc-500 mb-1">
            Task Type
          </label>
          <select
            value={taskType}
            onChange={(e) => setTaskType(e.target.value as TaskType)}
            className="w-full rounded-lg border border-zinc-300 dark:border-zinc-700 bg-zinc-50 dark:bg-zinc-900 px-3 py-2 text-sm text-zinc-900 dark:text-white mb-4"
          >
            {TASK_TYPE_OPTIONS.map((opt) => (
              <option key={opt.value} value={opt.value}>
                {opt.label}
              </option>
            ))}
          </select>
          <label className="block text-xs text-zinc-500 mb-1">
            Initial Guidance (optional)
          </label>
          <textarea
            value={initialGuidance}
            onChange={(e) => setInitialGuidance(e.target.value)}
            rows={4}
            placeholder="Describe the kind of training examples you want (tone, style, focus areas)..."
            className="w-full rounded-lg border border-zinc-300 dark:border-zinc-700 bg-zinc-50 dark:bg-zinc-900 px-3 py-2 text-sm text-zinc-900 dark:text-white placeholder:text-zinc-400 dark:placeholder:text-zinc-600 mb-4"
          />
          <button
            onClick={handleStartSession}
            disabled={createGuide.isPending || updateGuidance.isPending}
            className="rounded-lg bg-violet-600 px-4 py-2 text-sm font-medium text-white hover:bg-violet-500 transition disabled:opacity-50 disabled:cursor-not-allowed"
          >
            {createGuide.isPending || updateGuidance.isPending
              ? "Starting..."
              : "Start Guided Session"}
          </button>
        </div>
      )}

      {guide && (
        <div className="space-y-8">
          {guide.status === "failed" && (
            <div className="rounded-lg border border-red-200 dark:border-red-800 bg-red-50 dark:bg-red-900/20 p-4">
              <p className="text-sm text-red-700 dark:text-red-400">
                This guided session failed and cannot be resumed. Start a new
                project or contact support if this persists.
              </p>
            </div>
          )}

          {guide.status === "completed" && (
            <div className="rounded-lg border border-emerald-200 dark:border-emerald-800 bg-emerald-50 dark:bg-emerald-900/20 p-4 flex items-center justify-between">
              <p className="text-sm text-emerald-700 dark:text-emerald-400">
                Dataset generation completed.
              </p>
              {guide.dataset_id && (
                <Link
                  href={`/projects/${projectId}/dataset?datasetId=${guide.dataset_id}`}
                  className="text-sm font-medium text-emerald-700 dark:text-emerald-400 underline hover:no-underline"
                >
                  View Dataset
                </Link>
              )}
            </div>
          )}

          {/* Guidance */}
          <div>
            <h2 className="text-lg font-semibold text-zinc-900 dark:text-white mb-2">
              Guidance
            </h2>
            <textarea
              value={localGuidance}
              onChange={(e) => {
                setLocalGuidance(e.target.value);
                setGuidanceDirty(true);
              }}
              disabled={!canEditGuidance}
              rows={4}
              placeholder="Describe the kind of training examples you want (tone, style, focus areas)..."
              className="w-full rounded-lg border border-zinc-300 dark:border-zinc-700 bg-zinc-50 dark:bg-zinc-900 px-3 py-2 text-sm text-zinc-900 dark:text-white placeholder:text-zinc-400 dark:placeholder:text-zinc-600 disabled:opacity-60 mb-3"
            />
            <div className="flex flex-wrap gap-2 md:gap-3">
              <button
                onClick={() =>
                  updateGuidance.mutate({
                    id: guide.id,
                    data: { guidance: localGuidance },
                  })
                }
                disabled={
                  !canEditGuidance ||
                  !guidanceDirty ||
                  updateGuidance.isPending
                }
                className="rounded-lg bg-zinc-900 dark:bg-white px-4 py-2 text-sm font-medium text-white dark:text-zinc-900 hover:opacity-90 transition disabled:opacity-50 disabled:cursor-not-allowed"
              >
                {updateGuidance.isPending ? "Saving..." : "Save Guidance"}
              </button>
              <button
                onClick={() => refineGuidance.mutate(guide.id)}
                disabled={!canRefine || isRunning || refineGuidance.isPending}
                className="rounded-lg border border-violet-300 dark:border-violet-700 bg-violet-50 dark:bg-violet-900/20 px-4 py-2 text-sm font-medium text-violet-700 dark:text-violet-400 hover:bg-violet-100 dark:hover:bg-violet-900/40 transition disabled:opacity-50 disabled:cursor-not-allowed"
                title="Regenerate guidance from your rated preview samples"
              >
                {refineGuidance.isPending
                  ? "Starting..."
                  : guide.status === "generating_preview"
                    ? "Refining..."
                    : "Refine Guidance"}
              </button>
            </div>
          </div>

          {/* Facets */}
          <div>
            <div className="flex items-center justify-between mb-3">
              <h2 className="text-lg font-semibold text-zinc-900 dark:text-white">
                Facets
              </h2>
              <button
                onClick={() =>
                  generateFacets.mutate({ id: guide.id, data: {} })
                }
                disabled={
                  !canGenerateFacets || isRunning || generateFacets.isPending
                }
                className="rounded-lg bg-blue-600 px-4 py-2 text-sm font-medium text-white hover:bg-blue-500 transition disabled:opacity-50 disabled:cursor-not-allowed"
              >
                {generateFacets.isPending
                  ? "Starting..."
                  : guide.status === "generating_facets"
                    ? "Generating Facets..."
                    : guide.facets.length > 0
                      ? "Regenerate Facets"
                      : "Generate Facets"}
              </button>
            </div>
            {guide.facets.length > 0 ? (
              <>
                <div className="flex flex-wrap gap-2 mb-3">
                  {localFacets.map((facet) => (
                    <FacetChip
                      key={facet.id}
                      facet={facet}
                      disabled={!canEditFacets}
                      onToggle={() =>
                        setLocalFacets((prev) =>
                          prev.map((f) =>
                            f.id === facet.id ? { ...f, keep: !f.keep } : f,
                          ),
                        )
                      }
                    />
                  ))}
                </div>
                <button
                  onClick={() =>
                    updateFacets.mutate({
                      id: guide.id,
                      data: { facets: localFacets },
                    })
                  }
                  disabled={
                    !canEditFacets || !facetsDirty || updateFacets.isPending
                  }
                  className="rounded-lg border border-zinc-300 dark:border-zinc-700 px-4 py-1.5 text-sm text-zinc-600 dark:text-zinc-400 hover:border-zinc-400 dark:hover:border-zinc-500 hover:text-zinc-900 dark:hover:text-white transition disabled:opacity-50 disabled:cursor-not-allowed"
                >
                  {updateFacets.isPending
                    ? "Saving..."
                    : "Save Facet Selections"}
                </button>
                {facetsDirty && (
                  <p className="mt-2 text-xs text-amber-600 dark:text-amber-500">
                    Unsaved changes — save your facet selections before
                    generating a preview.
                  </p>
                )}
              </>
            ) : (
              <div className="rounded-lg border border-dashed border-zinc-300 dark:border-zinc-700 p-8 text-center">
                <p className="text-zinc-500">
                  No facets yet. Generate facets from your parsed documents to
                  get started.
                </p>
              </div>
            )}
          </div>

          {/* Preview */}
          <div>
            <div className="flex items-center justify-between mb-3">
              <h2 className="text-lg font-semibold text-zinc-900 dark:text-white">
                Preview Samples
              </h2>
              <button
                onClick={() =>
                  generatePreview.mutate({ id: guide.id, data: {} })
                }
                disabled={
                  !canGeneratePreview || isRunning || generatePreview.isPending
                }
                className="rounded-lg bg-emerald-600 px-4 py-2 text-sm font-medium text-white hover:bg-emerald-500 transition disabled:opacity-50 disabled:cursor-not-allowed"
                title={
                  facetsDirty
                    ? "Save your facet selections before generating a preview"
                    : !hasKeptFacet
                      ? "Keep at least one facet before generating a preview"
                      : undefined
                }
              >
                {generatePreview.isPending
                  ? "Starting..."
                  : guide.status === "generating_preview"
                    ? "Generating Preview..."
                    : guide.preview_samples.length > 0
                      ? "Regenerate Preview"
                      : "Generate Preview"}
              </button>
            </div>
            {guide.preview_samples.length > 0 ? (
              <div className="space-y-4">
                {guide.preview_samples.map((sample, i) => (
                  <PreviewCard
                    key={sample.id}
                    sample={sample}
                    index={i}
                    disabled={!canRate || rateSamples.isPending}
                    onRate={(rating) =>
                      rateSamples.mutate({
                        id: guide.id,
                        data: { ratings: [{ sample_id: sample.id, rating }] },
                      })
                    }
                  />
                ))}
              </div>
            ) : (
              <div className="rounded-lg border border-dashed border-zinc-300 dark:border-zinc-700 p-8 text-center">
                <p className="text-zinc-500">
                  No preview samples yet. Keep some facets, then generate a
                  preview.
                </p>
              </div>
            )}
          </div>

          {/* Generate dataset */}
          <div className="rounded-lg border border-zinc-200 dark:border-zinc-800 p-6">
            <h2 className="text-lg font-semibold text-zinc-900 dark:text-white mb-2">
              Generate Full Dataset
            </h2>
            <p className="text-sm text-zinc-500 mb-4">
              Once you&apos;re happy with the guidance and preview samples,
              generate the full training dataset.
            </p>
            <button
              onClick={() => generateDataset.mutate(guide.id)}
              disabled={
                !canGenerateDataset || isRunning || generateDataset.isPending
              }
              className="rounded-lg bg-violet-600 px-4 py-2 text-sm font-medium text-white hover:bg-violet-500 transition disabled:opacity-50 disabled:cursor-not-allowed"
            >
              {generateDataset.isPending
                ? "Starting..."
                : guide.status === "generating"
                  ? "Generating Dataset..."
                  : "Generate Full Dataset"}
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
