"use client";

import { useState } from "react";
import type { CatalogModel } from "@/lib/api-client";
import { useTriggerFullPipeline } from "@/hooks/use-pipeline";
import { Button } from "@/components/ui/button";

const STEPS = [
  { name: "Parse", detail: "extract text from your documents" },
  { name: "Generate", detail: "write training pairs with your LLM provider" },
  { name: "Train", detail: "QLoRA fine-tune on the generated dataset" },
  { name: "Evaluate", detail: "score the tuned model on a held-out set" },
];

/**
 * Confirmation panel for the automatic route: shows exactly what will run
 * and with which model before anything starts.
 */
export function OneClickSetup({
  projectId,
  taskType,
  catalogModels,
  suggestedBaseModel,
  onStarted,
}: {
  projectId: string;
  taskType: string;
  catalogModels: CatalogModel[];
  suggestedBaseModel: string;
  onStarted: () => void;
}) {
  const [baseModel, setBaseModel] = useState(suggestedBaseModel);
  const triggerFullPipeline = useTriggerFullPipeline(projectId);

  const start = () =>
    triggerFullPipeline.mutate(
      {
        task_type: taskType,
        base_model: baseModel,
        training_config: { method: "qlora", mode: "quick" },
      },
      { onSuccess: onStarted },
    );

  return (
    <div className="space-y-4 rounded-lg border border-zinc-200 p-4 dark:border-zinc-800 md:p-5">
      <div>
        <h3 className="text-sm font-semibold text-zinc-900 dark:text-white">
          One-Click Fine-Tune
        </h3>
        <p className="mt-1 text-xs text-zinc-500">
          Runs the whole pipeline unattended. You still review nothing in
          between — use the Guided route if you want to check the dataset
          first.
        </p>
      </div>

      <ol className="grid grid-cols-1 gap-2 sm:grid-cols-2 lg:grid-cols-4">
        {STEPS.map((step, i) => (
          <li
            key={step.name}
            className="rounded-md bg-zinc-50 px-3 py-2 dark:bg-zinc-900"
          >
            <p className="text-xs font-medium text-zinc-900 dark:text-white">
              {i + 1}. {step.name}
            </p>
            <p className="mt-0.5 text-xs text-zinc-500">{step.detail}</p>
          </li>
        ))}
      </ol>

      <div className="max-w-sm">
        <label className="mb-1 block text-xs text-zinc-500">Base Model</label>
        <select
          value={baseModel}
          onChange={(e) => setBaseModel(e.target.value)}
          className="w-full rounded-lg border border-zinc-300 bg-zinc-50 px-3 py-2 text-sm text-zinc-900 dark:border-zinc-700 dark:bg-zinc-900 dark:text-white"
        >
          {catalogModels.map((m) => (
            <option key={m.model_id} value={m.model_id}>
              {m.display_name} &mdash; {m.size}
              {m.gated ? " (gated)" : ""}
            </option>
          ))}
        </select>
        <p className="mt-1 text-xs text-zinc-500">
          Trained with QLoRA in quick mode. If the estimated GPU cost exceeds
          your approval threshold, training pauses for your approval.
        </p>
      </div>

      <div className="flex items-center gap-2">
        <Button
          onClick={start}
          disabled={!baseModel}
          loading={triggerFullPipeline.isPending}
        >
          {triggerFullPipeline.isPending ? "Starting..." : "Run Full Pipeline"}
        </Button>
      </div>

      {triggerFullPipeline.isError && (
        <p className="text-sm text-red-600 dark:text-red-400">
          {triggerFullPipeline.error.message}
        </p>
      )}
    </div>
  );
}
