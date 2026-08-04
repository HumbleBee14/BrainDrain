"use client";

import { useState } from "react";
import type { CatalogModel } from "@/lib/api-client";
import { useTriggerFullPipeline } from "@/hooks/use-pipeline";
import { useTeacherPolicy } from "@/hooks/use-teachers";
import {
  EMPTY_TEACHER_DRAFT,
  TeacherStep,
  teacherDraftBlocked,
  teacherFromDraft,
  type TeacherDraft,
} from "./teacher-step";

/**
 * Distillation setup: teacher → student → go. Runs the full pipeline —
 * the teacher writes the training examples from this project's documents,
 * then the student trains on them and gets a teacher-parity report.
 */
export function DistillSetup({
  projectId,
  taskType,
  catalogModels,
  suggestedBaseModel,
  disabled,
  onStarted,
}: {
  projectId: string;
  taskType: string;
  catalogModels: CatalogModel[];
  suggestedBaseModel: string;
  disabled: boolean;
  onStarted: () => void;
}) {
  const [teacherDraft, setTeacherDraft] =
    useState<TeacherDraft>(EMPTY_TEACHER_DRAFT);
  const [baseModel, setBaseModel] = useState(suggestedBaseModel);
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [pairsPerChunk, setPairsPerChunk] = useState(5);
  const [holdoutPct, setHoldoutPct] = useState(10);
  const triggerFullPipeline = useTriggerFullPipeline(projectId);

  const { data: policyData } = useTeacherPolicy(
    teacherDraft.api_base_url,
    teacherDraft.model,
  );
  const teacher = teacherFromDraft(teacherDraft);
  const blocked =
    teacherDraftBlocked(teacherDraft, policyData?.policy) || !baseModel;

  const start = () => {
    if (!teacher) return;
    triggerFullPipeline.mutate(
      {
        task_type: taskType,
        base_model: baseModel,
        training_config: {
          method: "qlora",
          mode: "distill",
          pairs_per_chunk: pairsPerChunk,
          golden_holdout_ratio: holdoutPct / 100,
        },
        teacher,
      },
      { onSuccess: onStarted },
    );
  };

  return (
    <div className="rounded-lg border border-zinc-200 dark:border-zinc-800 p-4 md:p-5 space-y-4">
      <div>
        <h3 className="text-sm font-semibold text-zinc-900 dark:text-white">
          Distill a larger model
        </h3>
        <p className="text-xs text-zinc-500 mt-1">
          Use a big, expensive model to teach a small one you own. You get a
          small model that behaves like the big one on your task — plus a
          report proving how close it got.
        </p>
      </div>

      <TeacherStep
        draft={teacherDraft}
        onChange={setTeacherDraft}
        showCotToggle={taskType === "reasoning"}
      />

      <div>
        <label className="block text-xs text-zinc-500 mb-1">
          Student — the small model you&apos;ll own
        </label>
        <select
          value={baseModel}
          onChange={(e) => setBaseModel(e.target.value)}
          className="w-full rounded-lg border border-zinc-300 dark:border-zinc-700 bg-zinc-50 dark:bg-zinc-900 px-3 py-2 text-sm text-zinc-900 dark:text-white"
        >
          {catalogModels.map((m) => (
            <option key={m.model_id} value={m.model_id}>
              {m.display_name} ({m.size})
            </option>
          ))}
        </select>
      </div>

      <div>
        <button
          type="button"
          onClick={() => setShowAdvanced(!showAdvanced)}
          aria-expanded={showAdvanced}
          className="text-xs text-zinc-500 hover:text-zinc-700 dark:hover:text-zinc-300"
        >
          {showAdvanced ? "▾" : "▸"} Advanced options
        </button>
        {showAdvanced && (
          <div className="mt-2 grid grid-cols-1 sm:grid-cols-2 gap-3">
            <div>
              <label className="block text-xs text-zinc-500 mb-1">
                Examples per chunk
              </label>
              <input
                type="number"
                min={1}
                max={50}
                value={pairsPerChunk}
                onChange={(e) =>
                  setPairsPerChunk(
                    Math.max(1, Math.min(50, Number(e.target.value) || 1)),
                  )
                }
                className="w-full rounded-lg border border-zinc-300 dark:border-zinc-700 bg-zinc-50 dark:bg-zinc-900 px-3 py-2 text-sm text-zinc-900 dark:text-white"
              />
            </div>
            <div>
              <label
                className="block text-xs text-zinc-500 mb-1"
                title="Held-out examples power the teacher-parity report — the model never trains on them"
              >
                Held-out share for the parity report (%)
              </label>
              <input
                type="number"
                min={0}
                max={25}
                value={holdoutPct}
                onChange={(e) =>
                  setHoldoutPct(
                    Math.max(0, Math.min(25, Number(e.target.value) || 0)),
                  )
                }
                className="w-full rounded-lg border border-zinc-300 dark:border-zinc-700 bg-zinc-50 dark:bg-zinc-900 px-3 py-2 text-sm text-zinc-900 dark:text-white"
              />
            </div>
          </div>
        )}
      </div>

      <div className="flex items-center gap-3">
        <button
          type="button"
          onClick={start}
          disabled={disabled || blocked || triggerFullPipeline.isPending}
          className="rounded-lg bg-violet-600 px-4 py-2 text-sm font-medium text-white hover:bg-violet-500 transition disabled:opacity-50 disabled:cursor-not-allowed"
        >
          {triggerFullPipeline.isPending
            ? "Starting..."
            : "Generate from teacher & train"}
        </button>
        {!teacher && teacherDraft.source && (
          <p className="text-xs text-red-400">
            Pick the model that will teach yours — this mode needs one.
          </p>
        )}
      </div>
      {triggerFullPipeline.isError && (
        <p className="text-sm text-red-400">
          {triggerFullPipeline.error.message}
        </p>
      )}
    </div>
  );
}
