"use client";

const MODES = [
  {
    name: "Quick",
    pipeline: "SFT",
    detail:
      "Supervised fine-tuning on your dataset. The fastest way to a working model.",
    bestFor: "first runs, style & knowledge transfer",
  },
  {
    name: "Aligned",
    pipeline: "SFT + DPO",
    detail:
      "Fine-tune, then align with preference pairs so the model favors better answers.",
    bestFor: "tone, helpfulness, response quality",
  },
  {
    name: "Reasoning",
    pipeline: "SFT + GRPO",
    detail:
      "Fine-tune, then reinforce step-by-step reasoning with reward-scored rollouts.",
    bestFor: "math, logic, multi-step tasks",
  },
  {
    name: "Iterative",
    pipeline: "multi-round SFT",
    detail:
      "Train in rounds with a held-out evaluation after each, keeping the best checkpoint.",
    bestFor: "squeezing quality from small datasets",
  },
  {
    name: "Distill",
    pipeline: "teacher → student",
    detail:
      "A large teacher model trains a small one you own — from its answers, its token distributions, or live on-policy feedback.",
    bestFor: "matching a big model at a fraction of the cost",
  },
] as const;

export function TrainingModesExplainer() {
  return (
    <div className="mt-6">
      <h3 className="text-sm font-semibold text-zinc-900 dark:text-white">
        Training modes this platform supports
      </h3>
      <div className="mt-3 grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
        {MODES.map((mode) => (
          <div
            key={mode.name}
            className="rounded-lg border border-zinc-200 p-4 dark:border-zinc-800"
          >
            <div className="flex items-baseline justify-between gap-2">
              <h4 className="font-medium text-zinc-900 dark:text-white">
                {mode.name}
              </h4>
              <span className="font-mono text-xs text-violet-600 dark:text-violet-400">
                {mode.pipeline}
              </span>
            </div>
            <p className="mt-1.5 text-sm text-zinc-600 dark:text-zinc-400">
              {mode.detail}
            </p>
            <p className="mt-2 text-xs text-zinc-400 dark:text-zinc-600">
              Best for {mode.bestFor}
            </p>
          </div>
        ))}
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
