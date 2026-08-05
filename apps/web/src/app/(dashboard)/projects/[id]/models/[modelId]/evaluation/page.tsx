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

// The failing activity records the reason under `report.error`.
function evaluationError(evaluation: Evaluation): string | null {
  const report = evaluation.report;
  if (!report || typeof report !== "object" || Array.isArray(report)) return null;
  const error = (report as Record<string, unknown>).error;
  return typeof error === "string" && error.trim() ? error : null;
}

function teacherParityNote(evaluation: Evaluation): string | null {
  const report = evaluation.report;
  if (!report || typeof report !== "object" || Array.isArray(report)) return null;
  const section = (report as Record<string, unknown>).teacher_parity;
  if (!section || typeof section !== "object" || Array.isArray(section)) return null;
  const note = (section as Record<string, unknown>).note;
  return typeof note === "string" && note.trim() ? note : null;
}

const DISTRIBUTION_MATCH_HELP =
  "How closely this model's confidence in each word it writes matches its teacher's, measured on the examples the teacher scored. 0 would mean identical, so unlike every other number here, smaller is better.";

// The only figure in this panel where a smaller number is the good news, so the
// direction is stated next to the value rather than left to the reader.
function DistributionMatch({ value }: { value?: number | null }) {
  if (typeof value !== "number") return null;

  return (
    <div className="border-t border-violet-200/60 dark:border-violet-900/60 pt-3">
      <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1">
        <span
          className="text-xs text-zinc-500 uppercase tracking-wider cursor-help"
          title={DISTRIBUTION_MATCH_HELP}
        >
          Distribution match
        </span>
        <span className="text-lg font-semibold text-zinc-900 dark:text-white">
          {value.toFixed(3)}
        </span>
        <span className="rounded-full border border-violet-300 dark:border-violet-800 px-2 py-0.5 text-[11px] font-medium text-violet-700 dark:text-violet-300">
          Lower is better
        </span>
      </div>
      <p className="mt-1 text-xs text-zinc-500">
        How closely this model matches the teacher&apos;s confidence in each word
        it writes. 0 would mean identical.
      </p>
    </div>
  );
}

function TeacherParitySection({ evaluation }: { evaluation: Evaluation }) {
  const parity = evaluation.scores?.teacher_parity;
  const note = teacherParityNote(evaluation);

  if (parity && typeof parity.parity === "number") {
    const pct = Math.round(parity.parity * 100);
    const winPct = Math.round((parity.win_rate ?? 0) * 100);
    const tiePct = Math.round((parity.tie_rate ?? 0) * 100);
    const lossPct = Math.max(0, 100 - winPct - tiePct);
    return (
      <div className="rounded-lg border border-violet-200 dark:border-violet-900 bg-violet-50/30 dark:bg-violet-900/10 p-6">
        <p className="text-xs text-zinc-500 uppercase tracking-wider mb-2">
          Teacher Parity
        </p>
        <p className="text-xl md:text-2xl font-semibold text-zinc-900 dark:text-white">
          Matches the teacher on {pct}% of held-out tasks.
        </p>
        <div className="mt-4 space-y-3">
          <div
            className="flex h-2.5 overflow-hidden rounded-full bg-zinc-200 dark:bg-zinc-800"
            title="Student vs teacher, judged blind on held-out tasks"
          >
            <div
              className="bg-emerald-500"
              style={{ width: `${winPct}%` }}
              title="Win: an independent judge preferred your small model's answer"
            />
            <div
              className="bg-violet-400"
              style={{ width: `${tiePct}%` }}
              title="Tie: the judge found both answers equally good"
            />
            <div
              className="bg-zinc-400 dark:bg-zinc-600"
              style={{ width: `${lossPct}%` }}
              title="Loss: the judge preferred the teacher's answer"
            />
          </div>
          <div className="flex flex-wrap gap-x-6 gap-y-1 text-xs text-zinc-500">
            <span title="An independent judge preferred your small model's answer">
              Win {winPct}%
            </span>
            <span title="The judge found both answers equally good">
              Tie {tiePct}%
            </span>
            <span title="The judge preferred the teacher's answer">
              Loss {lossPct}%
            </span>
            {typeof parity.agreement === "number" && (
              <span title="Share of answers the judge marked as saying the same thing as the teacher's">
                Answer agreement {Math.round(parity.agreement * 100)}%
              </span>
            )}
            {typeof parity.n === "number" && (
              <span title="Held-out tasks compared">n = {parity.n}</span>
            )}
          </div>
          <DistributionMatch value={parity.teacher_student_kl} />
        </div>
      </div>
    );
  }

  if (note) {
    return (
      <div className="rounded-lg border border-zinc-200 dark:border-zinc-800 p-5">
        <p className="text-xs text-zinc-500 uppercase tracking-wider mb-2">
          Teacher Parity
        </p>
        <p className="text-sm text-zinc-600 dark:text-zinc-400">
          No held-out set was kept for this run, so there&apos;s no teacher
          comparison. Re-run data generation with a held-out share to get a
          parity report.
        </p>
        <div className="mt-4">
          <DistributionMatch value={parity?.teacher_student_kl} />
        </div>
      </div>
    );
  }

  return null;
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
              Running 5 test suites: Domain, Document Knowledge, General, A/B
              Comparison, Safety
            </p>
          </div>
        </div>
      );
    }
    if (evaluation.status === "failed") {
      return (
        <div className="rounded-lg border border-red-200 bg-red-50/40 p-6 dark:border-red-800 dark:bg-red-900/10">
          <p className="text-sm font-medium text-red-700 dark:text-red-400">
            Evaluation failed
          </p>
          <p className="mt-2 break-words text-sm text-red-600 dark:text-red-300">
            {evaluationError(evaluation) ?? "No reason was recorded."}
          </p>
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

      {/* Teacher parity (distill mode) */}
      <TeacherParitySection evaluation={evaluation} />

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
                  {typeof scores.domain.mean === "number"
                    ? `${scores.domain.mean.toFixed(2)}/5`
                    : "—"}
                </span>
              </div>
            </div>
          </div>
        )}

        {/* Document Knowledge (golden holdout) */}
        {scores.doc_knowledge &&
          typeof scores.doc_knowledge.mean === "number" &&
          typeof scores.doc_knowledge.base_mean === "number" &&
          typeof scores.doc_knowledge.knowledge_lift === "number" && (
            <div className="rounded-lg border border-zinc-200 dark:border-zinc-800 p-5">
              <h3 className="text-sm font-semibold text-zinc-900 dark:text-white mb-1">
                Document Knowledge
              </h3>
              <p className="text-xs text-zinc-500 mb-4">
                Measured on held-out document content never seen in training
              </p>
              <div className="text-center mb-4">
                <p
                  className={`text-3xl font-bold ${
                    scores.doc_knowledge.knowledge_lift > 0
                      ? "text-green-600 dark:text-green-400"
                      : "text-amber-600 dark:text-amber-400"
                  }`}
                >
                  {scores.doc_knowledge.knowledge_lift > 0 ? "+" : ""}
                  {scores.doc_knowledge.knowledge_lift.toFixed(2)}
                </p>
                <p className="text-zinc-500 text-xs mt-1">
                  knowledge lift over base model
                </p>
              </div>
              <div className="grid grid-cols-2 gap-3">
                <ScoreCard
                  label="Base Model"
                  value={`${scores.doc_knowledge.base_mean.toFixed(2)}/5`}
                />
                <ScoreCard
                  label="Fine-tuned"
                  value={`${scores.doc_knowledge.mean.toFixed(2)}/5`}
                />
              </div>
              {scores.doc_knowledge.num_samples && (
                <p className="text-xs text-zinc-500 mt-3 text-center">
                  {scores.doc_knowledge.num_samples} held-out questions
                </p>
              )}
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
                value={`${scores.general.base_score.toFixed(1)}%`}
              />
              <ScoreCard
                label="Fine-tuned"
                value={`${scores.general.finetuned_score.toFixed(1)}%`}
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
                          vals as { base: number; finetuned: number }
                        ).finetuned.toFixed(0)}
                        % (base:{" "}
                        {(
                          vals as { base: number; finetuned: number }
                        ).base.toFixed(0)}
                        %)
                      </span>
                    </div>
                  ),
                )}
              </div>
            )}
          </div>
        )}

        {/* A/B Comparison — win_rate is null when the suite was skipped */}
        {scores.ab_comparison && typeof scores.ab_comparison.win_rate === "number" && (
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
  const [judgeThinking, setJudgeThinking] = useState(false);

  const evaluations = evalsData?.data ?? [];
  const latestEval = evaluations[0] ?? null;
  const hasRunningEval = evaluations.some((e) => e.status === "running");

  const handleRunEvaluation = async () => {
    try {
      await createEvaluation.mutateAsync({
        judge_model: judgeModel || undefined,
        judge_thinking: judgeThinking || undefined,
      });
      setShowRunForm(false);
      setJudgeModel("");
      setJudgeThinking(false);
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
          <div>
            <label className="flex items-center gap-2 text-sm text-zinc-900 dark:text-white">
              <input
                type="checkbox"
                checked={judgeThinking}
                onChange={(e) => setJudgeThinking(e.target.checked)}
                className="rounded border-zinc-300 dark:border-zinc-700"
              />
              Judge thinking mode
            </label>
            {judgeThinking && (
              <p className="text-xs text-amber-600 dark:text-amber-500 mt-1">
                The judge will reason before each verdict. Evaluation will take
                significantly longer and use many more tokens.
              </p>
            )}
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
