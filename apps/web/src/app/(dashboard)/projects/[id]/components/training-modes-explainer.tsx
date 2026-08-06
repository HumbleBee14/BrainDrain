"use client";

import { useEffect, useState } from "react";

const COLLAPSE_KEY = "ft-modes-explainer-collapsed";

export type TrainingModeKey =
  | "quick"
  | "aligned"
  | "reasoning"
  | "iterative"
  | "distill";

const MODES = [
  {
    key: "quick",
    name: "Quick",
    pipeline: "SFT",
    detail:
      "Supervised fine-tuning on your dataset. The fastest way to a working model.",
    bestFor: "first runs, style & knowledge transfer",
  },
  {
    key: "aligned",
    name: "Aligned",
    pipeline: "SFT + DPO",
    detail:
      "Fine-tune, then align with preference pairs so the model favors better answers.",
    bestFor: "tone, helpfulness, response quality",
  },
  {
    key: "reasoning",
    name: "Reasoning",
    pipeline: "SFT + GRPO",
    detail:
      "Fine-tune, then reinforce step-by-step reasoning with reward-scored rollouts.",
    bestFor: "math, logic, multi-step tasks",
  },
  {
    key: "iterative",
    name: "Iterative",
    pipeline: "Multi-Round SFT",
    detail:
      "Train in rounds with a held-out evaluation after each, keeping the best checkpoint.",
    bestFor: "squeezing quality from small datasets",
  },
  {
    key: "distill",
    name: "Distill",
    pipeline: "Distillation",
    detail:
      "A large teacher model trains a small one you own — from its answers, its token distributions, or live on-policy feedback.",
    bestFor: "matching a big model at a fraction of the cost",
  },
] as const satisfies ReadonlyArray<{ key: TrainingModeKey; [k: string]: string }>;

export function TrainingModesExplainer({
  defaultCollapsed = false,
  selected,
  onSelect,
  canTrain,
  canDistill,
}: {
  defaultCollapsed?: boolean;
  selected: TrainingModeKey | null;
  onSelect: (mode: TrainingModeKey) => void;
  canTrain: boolean;
  canDistill: boolean;
}) {
  const [collapsed, setCollapsed] = useState(defaultCollapsed);

  // localStorage is read after mount so server and first client render agree;
  // an explicit user choice overrides the default.
  useEffect(() => {
    const stored = localStorage.getItem(COLLAPSE_KEY);
    if (stored !== null) setCollapsed(stored === "1");
  }, []);

  const toggle = (next: boolean) => {
    setCollapsed(next);
    localStorage.setItem(COLLAPSE_KEY, next ? "1" : "0");
  };

  if (collapsed) {
    return (
      <button
        onClick={() => toggle(false)}
        className="text-sm font-medium text-violet-600 underline-offset-2 hover:underline dark:text-violet-400"
      >
        Show the fine-tuning modes
      </button>
    );
  }

  return (
    <div>
      <div className="flex items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold text-zinc-900 dark:text-white">
          Pick a mode to set up a run
        </h3>
        <button
          onClick={() => toggle(true)}
          className="text-xs text-zinc-400 underline-offset-2 hover:text-zinc-600 hover:underline dark:text-zinc-600 dark:hover:text-zinc-400"
        >
          Hide
        </button>
      </div>
      <div className="mt-3 grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
        {MODES.map((mode) => {
          const enabled = mode.key === "distill" ? canDistill : canTrain;
          const isSelected = selected === mode.key;
          return (
            <button
              key={mode.key}
              type="button"
              onClick={() => onSelect(mode.key)}
              disabled={!enabled}
              title={
                enabled
                  ? undefined
                  : mode.key === "distill"
                    ? "Upload documents first — the teacher writes its training data from them"
                    : "Approve a dataset first to start fine-tuning"
              }
              className={`rounded-lg border p-4 text-left transition disabled:cursor-not-allowed disabled:opacity-50 ${
                isSelected
                  ? "border-violet-500 bg-violet-50/50 ring-1 ring-violet-500 dark:bg-violet-950/20"
                  : "border-zinc-200 enabled:hover:border-zinc-400 dark:border-zinc-800 dark:enabled:hover:border-zinc-600"
              }`}
            >
              <div className="flex items-baseline justify-between gap-2">
                <h4 className="font-mono text-base font-bold text-violet-700 dark:text-violet-400">
                  {mode.pipeline}
                </h4>
                <span className="text-xs text-zinc-400 dark:text-zinc-600">
                  {isSelected ? "Selected" : mode.name}
                </span>
              </div>
              <p className="mt-1.5 text-sm text-zinc-600 dark:text-zinc-400">
                {mode.detail}
              </p>
              <p className="mt-2 text-xs text-zinc-400 dark:text-zinc-600">
                Best for {mode.bestFor}
              </p>
            </button>
          );
        })}
        <div className="rounded-lg border border-dashed border-zinc-200 p-4 dark:border-zinc-800">
          <h4 className="font-medium text-zinc-900 dark:text-white">
            Every mode, three methods
          </h4>
          <p className="mt-1.5 text-sm text-zinc-600 dark:text-zinc-400">
            <span className="font-medium">QLoRA</span> (4-bit, cheapest),{" "}
            <span className="font-medium">LoRA</span> (16-bit adapters), or{" "}
            <span className="font-medium">full fine-tune</span> — picked in the
            training form, with a cost estimate before anything runs.
          </p>
        </div>
      </div>
    </div>
  );
}
