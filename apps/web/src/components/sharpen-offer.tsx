"use client";

import { useState } from "react";
import { useImproveOffer } from "@/hooks/use-teachers";
import { useModel } from "@/hooks/use-models";
import { useTrainingJob, useCreateTrainingJob } from "@/hooks/use-training";

function parityOf(scores: unknown): number | null {
  const parity = (scores as { teacher_parity?: { parity?: unknown } } | null)
    ?.teacher_parity?.parity;
  return typeof parity === "number" ? parity : null;
}

const PANEL_CLASS =
  "rounded-lg border border-zinc-200 dark:border-zinc-800 bg-zinc-50/50 dark:bg-zinc-900/50 p-4";

/**
 * The improve-pass offer on a model page, and the before/after parity of a model
 * that has already taken one.
 *
 * Renders nothing at all when the model has no teacher it can be sharpened
 * against, which is the ordinary case: on-policy training is not a third choice
 * at setup, it is an offer made only where it applies.
 */
export function SharpenOffer({
  projectId,
  modelId,
  trainingJobId,
  baseModel,
  evalScores,
}: {
  projectId: string;
  modelId: string;
  trainingJobId: string;
  baseModel: string;
  evalScores: unknown;
}) {
  const [started, setStarted] = useState(false);
  const { data: offer } = useImproveOffer(modelId);
  const { data: job } = useTrainingJob(trainingJobId, Boolean(trainingJobId));
  const parentId = job?.parent_model_id ?? null;
  const { data: parent } = useModel(parentId ?? "", Boolean(parentId));
  const createJob = useCreateTrainingJob(projectId);

  const before = parityOf(parent?.eval_scores);
  const after = parityOf(evalScores);

  const improve = () => {
    if (!offer?.dataset_id) return;
    createJob.mutate(
      {
        dataset_id: offer.dataset_id,
        base_model: baseModel,
        mode: "distill",
        distill: { method: "on_policy" },
        parent_model_id: modelId,
      },
      { onSuccess: () => setStarted(true) },
    );
  };

  return (
    <div className="mb-8 space-y-3">
      {before !== null && after !== null && (
        <div className={PANEL_CLASS}>
          <p className="text-sm font-medium text-zinc-900 dark:text-white">
            Sharpening result
          </p>
          <p className="mt-1 text-sm text-zinc-500">
            Matched its teacher on{" "}
            <span className="text-zinc-900 dark:text-white">
              {Math.round(before * 100)}%
            </span>{" "}
            of held-out tasks before this pass, and{" "}
            <span className="text-zinc-900 dark:text-white">
              {Math.round(after * 100)}%
            </span>{" "}
            after.
          </p>
          <p className="mt-1 text-xs text-zinc-400 dark:text-zinc-600">
            Both figures come from the same held-out tasks, which is what makes
            them comparable.
          </p>
        </div>
      )}

      {offer?.eligible && offer.estimate && (
        <div className={PANEL_CLASS}>
          <p className="text-sm font-medium text-zinc-900 dark:text-white">
            Sharpen against the teacher
          </p>
          <p className="mt-1 text-sm text-zinc-500">
            This model retrains on its <em>own</em> answers, corrected
            word-by-word by {offer.teacher_model}. It helps most when answers
            drift over long outputs or multi-step tool use.
          </p>
          <p className="mt-2 text-sm text-zinc-500">
            Estimated{" "}
            <span className="text-zinc-900 dark:text-white">
              ${offer.estimate.est_cost_usd.toFixed(2)}
            </span>{" "}
            of GPU time — the teacher runs beside the trainer on a{" "}
            {offer.estimate.gpu_class.toUpperCase()} for roughly{" "}
            {offer.estimate.est_gpu_hours.toFixed(1)} hours.
          </p>
          {offer.estimate.basis === "approximate" && (
            <p className="mt-1 text-xs text-zinc-400 dark:text-zinc-600">
              That price assumes how long the answers will be, since your model
              has not written them yet. The run is billed for the GPU time it
              actually uses.
            </p>
          )}

          {started ? (
            <p className="mt-3 text-sm text-zinc-500">
              Started. The new model appears in this project when the pass
              finishes, with its parity measured against the same tasks.
            </p>
          ) : (
            <button
              type="button"
              onClick={improve}
              disabled={createJob.isPending}
              className="mt-3 rounded-lg bg-zinc-900 dark:bg-white px-4 py-2 text-sm font-medium text-white dark:text-zinc-900 disabled:opacity-50"
            >
              {createJob.isPending ? "Starting…" : "Improve"}
            </button>
          )}

          {createJob.isError && (
            <p className="mt-2 text-sm text-red-500">
              {createJob.error instanceof Error
                ? createJob.error.message
                : "Could not start the improve pass."}
            </p>
          )}
        </div>
      )}
    </div>
  );
}
