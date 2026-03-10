"use client";

import { useParams } from "next/navigation";
import { useEffect, useState } from "react";
import { toast } from "sonner";
import { useModel } from "@/hooks/use-models";
import { useEvaluations, useCreateEvaluation } from "@/hooks/use-evaluations";
import type { Evaluation } from "@/lib/api-client";
import { Breadcrumbs } from "@/components/breadcrumbs";

function StatusBadge({ status }: { status: string }) {
  const colors: Record<string, string> = {
    running: "bg-blue-50 text-blue-700 border-blue-200 dark:bg-blue-900/50 dark:text-blue-400 dark:border-blue-800 animate-pulse",
    completed: "bg-emerald-50 text-emerald-700 border-emerald-200 dark:bg-emerald-900/50 dark:text-emerald-400 dark:border-emerald-800",
    failed: "bg-red-50 text-red-700 border-red-200 dark:bg-red-900/50 dark:text-red-400 dark:border-red-800",
  };

  const cls = colors[status] || "bg-zinc-100 text-zinc-600 border-zinc-300 dark:bg-zinc-800 dark:text-zinc-400 dark:border-zinc-700";

  return (
    <span
      className={`inline-flex items-center rounded-full border px-2.5 py-0.5 text-xs font-medium ${cls}`}
    >
      {status}
    </span>
  );
}

function ScoreBar({
  label,
  value,
  max = 5,
}: {
  label: string;
  value: number;
  max?: number;
}) {
  const pct = Math.min(100, (value / max) * 100);
  const color =
    pct >= 80
      ? "bg-emerald-500"
      : pct >= 60
        ? "bg-blue-500"
        : pct >= 40
          ? "bg-amber-500"
          : "bg-red-500";

  return (
    <div className="space-y-1">
      <div className="flex justify-between text-sm">
        <span className="text-zinc-600 dark:text-zinc-400">{label}</span>
        <span className="text-zinc-900 dark:text-white font-medium">
          {value.toFixed(2)}
          {max !== 100 ? `/${max}` : ""}
        </span>
      </div>
      <div className="h-2 rounded-full bg-zinc-100 dark:bg-zinc-800">
        <div
          className={`h-2 rounded-full ${color} transition-all`}
          style={{ width: `${pct}%` }}
        />
      </div>
    </div>
  );
}

function ScoreCard({
  label,
  value,
  subtitle,
  alert,
}: {
  label: string;
  value: string;
  subtitle?: string;
  alert?: boolean;
}) {
  return (
    <div
      className={`rounded-lg border p-4 ${alert ? "border-red-200 dark:border-red-800 bg-red-50/10 dark:bg-red-900/10" : "border-zinc-200 dark:border-zinc-800"}`}
    >
      <p className="text-xs text-zinc-500 uppercase tracking-wider">{label}</p>
      <p
        className={`text-xl font-bold mt-1 ${alert ? "text-red-600 dark:text-red-400" : "text-zinc-900 dark:text-white"}`}
      >
        {value}
      </p>
      {subtitle && <p className="text-xs text-zinc-400 dark:text-zinc-600 mt-1">{subtitle}</p>}
    </div>
  );
}

function EvaluationDetail({ evaluation }: { evaluation: Evaluation }) {
  const scores = evaluation.scores;

  if (!scores) {
    if (evaluation.status === "running") {
      return (
        <div className="rounded-lg border border-blue-200 dark:border-blue-800 bg-blue-50/10 dark:bg-blue-900/10 p-8 text-center">
          <div className="animate-pulse">
            <p className="text-blue-400 text-lg font-medium">
              Evaluation in progress...
            </p>
            <p className="text-zinc-500 text-sm mt-2">
              Running 4 test suites: Domain, General, A/B Comparison, Safety
            </p>
          </div>
        </div>
      );
    }
    return (
      <div className="rounded-lg border border-zinc-200 dark:border-zinc-800 p-8 text-center">
        <p className="text-zinc-500">No scores available.</p>
      </div>
    );
  }

  return (
    <div className="space-y-8">
      {/* Overall score */}
      <div className="rounded-lg border border-zinc-200 dark:border-zinc-800 p-6 text-center">
        <p className="text-xs text-zinc-500 uppercase tracking-wider mb-2">
          Overall Score
        </p>
        <p className="text-3xl md:text-5xl font-bold text-zinc-900 dark:text-white">{scores.overall}</p>
        <p className="text-zinc-500 text-sm mt-1">out of 100</p>
      </div>

      {/* Suite scores grid */}
      <div className="grid grid-cols-1 md:grid-cols-2 gap-4 md:gap-6">
        {/* Domain Evaluation */}
        {scores.domain && (
          <div className="rounded-lg border border-zinc-200 dark:border-zinc-800 p-5">
            <h3 className="text-sm font-semibold text-zinc-900 dark:text-white mb-4">
              Domain Evaluation
            </h3>
            <div className="space-y-3">
              <ScoreBar label="Accuracy" value={scores.domain.accuracy} />
              <ScoreBar
                label="Completeness"
                value={scores.domain.completeness}
              />
              <ScoreBar
                label="Faithfulness"
                value={scores.domain.faithfulness}
              />
            </div>
            <div className="mt-4 pt-3 border-t border-zinc-200 dark:border-zinc-800">
              <div className="flex justify-between text-sm">
                <span className="text-zinc-500">Mean</span>
                <span className="text-zinc-900 dark:text-white font-medium">
                  {scores.domain.mean.toFixed(2)}/5
                </span>
              </div>
            </div>
          </div>
        )}

        {/* General Capability */}
        {scores.general && (
          <div className="rounded-lg border border-zinc-200 dark:border-zinc-800 p-5">
            <h3 className="text-sm font-semibold text-zinc-900 dark:text-white mb-4">
              General Capability
            </h3>
            <div className="grid grid-cols-1 sm:grid-cols-2 gap-3 mb-4">
              <ScoreCard
                label="Base Model"
                value={`${(scores.general.base_score * 100).toFixed(1)}%`}
              />
              <ScoreCard
                label="Fine-tuned"
                value={`${(scores.general.finetuned_score * 100).toFixed(1)}%`}
              />
            </div>
            <ScoreCard
              label="Change"
              value={`${scores.general.delta_pct > 0 ? "+" : ""}${scores.general.delta_pct.toFixed(1)}%`}
              subtitle={
                scores.general.forgetting_alert
                  ? "Catastrophic forgetting detected!"
                  : "Capability preserved"
              }
              alert={scores.general.forgetting_alert}
            />
            {scores.general.per_category && (
              <div className="mt-4 space-y-2">
                {Object.entries(scores.general.per_category).map(
                  ([cat, vals]) => (
                    <div key={cat} className="flex justify-between text-xs">
                      <span className="text-zinc-500 capitalize">
                        {cat.replace(/_/g, " ")}
                      </span>
                      <span className="text-zinc-600 dark:text-zinc-400">
                        {(
                          (vals as { base: number; finetuned: number })
                            .finetuned * 100
                        ).toFixed(0)}
                        % (base:{" "}
                        {(
                          (vals as { base: number; finetuned: number }).base *
                          100
                        ).toFixed(0)}
                        %)
                      </span>
                    </div>
                  ),
                )}
              </div>
            )}
          </div>
        )}

        {/* A/B Comparison */}
        {scores.ab_comparison && (
          <div className="rounded-lg border border-zinc-200 dark:border-zinc-800 p-5">
            <h3 className="text-sm font-semibold text-zinc-900 dark:text-white mb-4">
              A/B Comparison
            </h3>
            <div className="text-center mb-4">
              <p className="text-3xl font-bold text-zinc-900 dark:text-white">
                {(scores.ab_comparison.win_rate * 100).toFixed(1)}%
              </p>
              <p className="text-xs text-zinc-500 mt-1">
                Win Rate vs Base Model
              </p>
            </div>
            <div className="flex justify-between text-xs text-zinc-500">
              <span>
                95% CI: {(scores.ab_comparison.confidence_low * 100).toFixed(1)}
                %
              </span>
              <span>
                {(scores.ab_comparison.confidence_high * 100).toFixed(1)}%
              </span>
            </div>
            <div className="h-2 rounded-full bg-zinc-100 dark:bg-zinc-800 mt-2 relative">
              <div
                className="absolute h-2 rounded-full bg-blue-500"
                style={{
                  left: `${scores.ab_comparison.confidence_low * 100}%`,
                  width: `${(scores.ab_comparison.confidence_high - scores.ab_comparison.confidence_low) * 100}%`,
                }}
              />
              <div
                className="absolute top-1/2 -translate-y-1/2 w-2 h-2 rounded-full bg-white"
                style={{ left: `${scores.ab_comparison.win_rate * 100}%` }}
              />
            </div>
            <p className="text-xs text-zinc-400 dark:text-zinc-600 mt-3 text-center">
              {scores.ab_comparison.total ?? 0} blind comparisons
            </p>
          </div>
        )}

        {/* Safety */}
        {scores.safety && (
          <div className="rounded-lg border border-zinc-200 dark:border-zinc-800 p-5">
            <h3 className="text-sm font-semibold text-zinc-900 dark:text-white mb-4">
              Safety Check
            </h3>
            <div className="grid grid-cols-1 sm:grid-cols-2 gap-3 mb-4">
              <ScoreCard
                label="Refusal Rate"
                value={`${(scores.safety.refusal_rate * 100).toFixed(0)}%`}
              />
              <ScoreCard
                label="Base Rate"
                value={`${(scores.safety.base_refusal_rate * 100).toFixed(0)}%`}
              />
            </div>
            {scores.safety.degraded ? (
              <div className="rounded-lg border border-red-200 dark:border-red-800 bg-red-50/10 dark:bg-red-900/10 p-3 text-center">
                <p className="text-sm text-red-600 dark:text-red-400 font-medium">
                  Safety Degraded
                </p>
                <p className="text-xs text-zinc-500 mt-1">
                  Refusal rate dropped significantly from base model
                </p>
              </div>
            ) : (
              <div className="rounded-lg border border-emerald-200 dark:border-emerald-800 bg-emerald-50/10 dark:bg-emerald-900/10 p-3 text-center">
                <p className="text-sm text-emerald-600 dark:text-emerald-400 font-medium">
                  Safety Preserved
                </p>
              </div>
            )}
          </div>
        )}
      </div>

      {/* Recommendations */}
      {evaluation.report &&
        typeof evaluation.report === "object" &&
        Array.isArray(
          (evaluation.report as Record<string, unknown>).recommendations,
        ) && (
          <div className="rounded-lg border border-zinc-200 dark:border-zinc-800 p-5">
            <h3 className="text-sm font-semibold text-zinc-900 dark:text-white mb-3">
              Recommendations
            </h3>
            <ul className="space-y-2">
              {(
                (evaluation.report as Record<string, unknown>)
                  .recommendations as string[]
              ).map((rec, i) => (
                <li key={i} className="flex gap-2 text-sm">
                  <span className="text-zinc-400 dark:text-zinc-600 shrink-0">&bull;</span>
                  <span className="text-zinc-600 dark:text-zinc-400">{rec}</span>
                </li>
              ))}
            </ul>
          </div>
        )}
    </div>
  );
}

export default function EvaluationPage() {
  const params = useParams<{ id: string; modelId: string }>();
  const { data: model } = useModel(params.modelId);
  const { data: evalsData, isLoading } = useEvaluations(params.modelId);
  const createEvaluation = useCreateEvaluation(params.modelId);

  useEffect(() => {
    if (createEvaluation.isSuccess) toast.success("Evaluation started");
  }, [createEvaluation.isSuccess]);
  useEffect(() => {
    if (createEvaluation.isError) toast.error(createEvaluation.error.message);
  }, [createEvaluation.isError, createEvaluation.error]);

  const [showRunForm, setShowRunForm] = useState(false);
  const [judgeModel, setJudgeModel] = useState("");

  const evaluations = evalsData?.data ?? [];
  const latestEval = evaluations[0] ?? null;
  const hasRunningEval = evaluations.some((e) => e.status === "running");

  const handleRunEvaluation = async () => {
    try {
      await createEvaluation.mutateAsync({
        judge_model: judgeModel || undefined,
      });
      setShowRunForm(false);
      setJudgeModel("");
    } catch {
      // Error is captured by React Query and surfaced via createEvaluation.isError
    }
  };

  if (isLoading) {
    return (
      <div className="flex items-center justify-center py-20">
        <p className="text-zinc-500">Loading evaluation...</p>
      </div>
    );
  }

  return (
    <div>
      {/* Header */}
      <div className="mb-8">
        <Breadcrumbs
          items={[
            { label: "Projects", href: "/projects" },
            { label: "Project", href: `/projects/${params.id}` },
            {
              label: model?.name || "Model",
              href: `/projects/${params.id}/models/${params.modelId}`,
            },
            { label: "Evaluation" },
          ]}
        />
        <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3">
          <div className="flex items-center gap-3">
            <h1 className="text-xl md:text-2xl font-bold text-zinc-900 dark:text-white">Evaluation</h1>
            {latestEval && <StatusBadge status={latestEval.status} />}
          </div>
          <div className="flex gap-2">
            {!showRunForm && (
              <button
                onClick={() => setShowRunForm(true)}
                disabled={hasRunningEval}
                className="rounded-lg bg-violet-600 px-4 py-2 text-sm font-medium text-white hover:bg-violet-500 transition disabled:opacity-50"
              >
                {hasRunningEval ? "Evaluation Running..." : "Run Evaluation"}
              </button>
            )}
          </div>
        </div>
      </div>

      {/* Run evaluation form */}
      {showRunForm && (
        <div className="rounded-lg border border-zinc-200 dark:border-zinc-800 p-4 mb-8 space-y-3">
          <p className="text-sm text-zinc-900 dark:text-white">Configure evaluation run</p>
          <div>
            <label className="block text-xs text-zinc-500 mb-1">
              Judge Model (optional)
            </label>
            <input
              value={judgeModel}
              onChange={(e) => setJudgeModel(e.target.value)}
              placeholder="e.g., gpt-4o, claude-sonnet-4-20250514 (uses default if empty)"
              className="w-full rounded-lg border border-zinc-300 dark:border-zinc-700 bg-zinc-50 dark:bg-zinc-900 px-3 py-2 text-sm text-zinc-900 dark:text-white"
            />
            <p className="text-xs text-zinc-400 dark:text-zinc-600 mt-1">
              The judge model scores responses for quality. Leave empty to use
              the worker&apos;s default.
            </p>
          </div>
          <div className="flex gap-2">
            <button
              onClick={handleRunEvaluation}
              disabled={createEvaluation.isPending}
              className="rounded-lg bg-violet-600 px-4 py-2 text-sm font-medium text-white hover:bg-violet-500 transition disabled:opacity-50"
            >
              {createEvaluation.isPending ? "Starting..." : "Start Evaluation"}
            </button>
            <button
              onClick={() => setShowRunForm(false)}
              className="rounded-lg border border-zinc-300 dark:border-zinc-700 px-4 py-2 text-sm text-zinc-600 dark:text-zinc-400 hover:border-zinc-400 dark:hover:border-zinc-600 transition"
            >
              Cancel
            </button>
          </div>
          {createEvaluation.isError && (
            <p className="text-sm text-red-400">
              {createEvaluation.error.message}
            </p>
          )}
        </div>
      )}

      {/* Latest evaluation results */}
      {latestEval ? (
        <EvaluationDetail evaluation={latestEval} />
      ) : (
        <div className="rounded-lg border border-zinc-200 dark:border-zinc-800 p-8 text-center">
          <p className="text-zinc-500 mb-2">
            No evaluations have been run yet.
          </p>
          <p className="text-xs text-zinc-400 dark:text-zinc-600">
            Run an evaluation to measure domain accuracy, general capability,
            A/B comparison, and safety.
          </p>
        </div>
      )}

      {/* Previous evaluations */}
      {evaluations.length > 1 && (
        <div className="mt-8">
          <h3 className="text-sm font-medium text-zinc-600 dark:text-zinc-400 mb-3">
            Previous Evaluations
          </h3>
          <div className="rounded-lg border border-zinc-200 dark:border-zinc-800">
            {evaluations.slice(1).map((ev) => (
              <div
                key={ev.id}
                className="flex items-center justify-between py-3 px-4 border-b border-zinc-200 dark:border-zinc-800 last:border-b-0"
              >
                <div>
                  <p className="text-sm text-zinc-900 dark:text-white">
                    {new Date(ev.created_at).toLocaleString()}
                  </p>
                  <p className="text-xs text-zinc-400 dark:text-zinc-600">
                    {ev.scores?.overall != null
                      ? `Score: ${ev.scores.overall}/100`
                      : "No scores"}
                  </p>
                </div>
                <StatusBadge status={ev.status} />
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
