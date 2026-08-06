"use client";

import type { ProjectPipelineStatus } from "@/lib/generated/ProjectPipelineStatus";

export type StepState = "done" | "active" | "current" | "upcoming";

export interface PipelineStep {
  key: string;
  label: string;
  detail: string;
  state: StepState;
}

/** Derive the four pipeline stages from raw status counts. */
export function computePipelineSteps(
  status: ProjectPipelineStatus,
  scoringCount: number,
): PipelineStep[] {
  const docs = status.documents;
  const ds = status.datasets;
  const jobs = status.training_jobs;
  const models = status.models;

  const documents: PipelineStep = {
    key: "documents",
    label: "Documents",
    state: "upcoming",
    detail: "none yet",
  };
  if (docs.parsing > 0) {
    documents.state = "active";
    documents.detail = `parsing ${docs.parsing}…`;
  } else if (docs.parsed > 0) {
    documents.state = "done";
    documents.detail = `${docs.parsed} parsed`;
  } else if (docs.uploaded > 0) {
    documents.state = "current";
    documents.detail = `${docs.uploaded} uploaded — not parsed`;
  } else if (docs.failed > 0) {
    documents.state = "current";
    documents.detail = `${docs.failed} failed`;
  }

  const dataset: PipelineStep = {
    key: "dataset",
    label: "Dataset",
    state: "upcoming",
    detail: "none yet",
  };
  if (ds.generating > 0) {
    dataset.state = "active";
    dataset.detail = `generating ${ds.generating}…`;
  } else if (ds.approved > 0) {
    dataset.state = "done";
    dataset.detail = `${ds.approved} approved`;
  } else if (ds.review_pending > 0) {
    dataset.state = "current";
    dataset.detail = `${ds.review_pending} awaiting review`;
  } else if (ds.failed > 0) {
    dataset.state = "current";
    dataset.detail = `${ds.failed} failed`;
  } else if (docs.parsed > 0) {
    dataset.state = "current";
    dataset.detail = "ready to generate";
  }

  const training: PipelineStep = {
    key: "training",
    label: "Training",
    state: "upcoming",
    detail: "not started",
  };
  if (scoringCount > 0) {
    training.state = "active";
    training.detail = "scoring with teacher…";
  } else if (jobs.training > 0) {
    training.state = "active";
    training.detail = `${jobs.training} running…`;
  } else if (jobs.pending > 0) {
    training.state = "active";
    training.detail = `${jobs.pending} queued`;
  } else if (jobs.completed > 0) {
    training.state = "done";
    training.detail = `${jobs.completed} completed`;
  } else if (ds.approved > 0) {
    training.state = "current";
    training.detail = "ready to train";
  } else if (jobs.failed > 0) {
    training.state = "current";
    training.detail = `${jobs.failed} failed`;
  }

  const model: PipelineStep = {
    key: "model",
    label: "Model",
    state: "upcoming",
    detail: "none yet",
  };
  if (models.total > 0) {
    model.state = "done";
    model.detail =
      models.active > 0
        ? `${models.total} built · ${models.active} deployed`
        : `${models.total} built`;
  }

  return [documents, dataset, training, model];
}

function StepMarker({ state, index }: { state: StepState; index: number }) {
  if (state === "done") {
    return (
      <span className="flex h-6 w-6 shrink-0 items-center justify-center rounded-full bg-emerald-500 text-white">
        <svg viewBox="0 0 16 16" className="h-3.5 w-3.5" fill="none">
          <path
            d="M3.5 8.5l3 3 6-7"
            stroke="currentColor"
            strokeWidth="2"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
        </svg>
      </span>
    );
  }
  if (state === "active") {
    return (
      <span className="flex h-6 w-6 shrink-0 items-center justify-center rounded-full border-2 border-violet-500">
        <span className="h-2 w-2 animate-pulse rounded-full bg-violet-500" />
      </span>
    );
  }
  if (state === "current") {
    return (
      <span className="flex h-6 w-6 shrink-0 items-center justify-center rounded-full border-2 border-violet-500 text-xs font-semibold text-violet-600 dark:text-violet-400">
        {index + 1}
      </span>
    );
  }
  return (
    <span className="flex h-6 w-6 shrink-0 items-center justify-center rounded-full border-2 border-zinc-200 text-xs font-semibold text-zinc-400 dark:border-zinc-700 dark:text-zinc-600">
      {index + 1}
    </span>
  );
}

export function PipelineStepper({ steps }: { steps: PipelineStep[] }) {
  return (
    <ol className="flex items-start gap-2 overflow-x-auto py-1">
      {steps.map((step, i) => (
        <li key={step.key} className="flex min-w-0 flex-1 items-start gap-2">
          <div className="flex min-w-0 flex-col items-start gap-1">
            <div className="flex items-center gap-2">
              <StepMarker state={step.state} index={i} />
              <span
                className={`text-sm font-medium ${
                  step.state === "upcoming"
                    ? "text-zinc-400 dark:text-zinc-600"
                    : "text-zinc-900 dark:text-white"
                }`}
              >
                {step.label}
              </span>
            </div>
            <span
              className={`pl-8 text-xs ${
                step.state === "active"
                  ? "text-violet-600 dark:text-violet-400"
                  : "text-zinc-400 dark:text-zinc-600"
              }`}
            >
              {step.detail}
            </span>
          </div>
          {i < steps.length - 1 && (
            <div
              aria-hidden
              className={`mt-3 h-px min-w-4 flex-1 ${
                step.state === "done"
                  ? "bg-emerald-300 dark:bg-emerald-800"
                  : "bg-zinc-200 dark:bg-zinc-800"
              }`}
            />
          )}
        </li>
      ))}
    </ol>
  );
}
