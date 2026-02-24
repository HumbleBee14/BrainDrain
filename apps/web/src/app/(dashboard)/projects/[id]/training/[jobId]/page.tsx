"use client";

import { useParams } from "next/navigation";
import Link from "next/link";
import { toast } from "sonner";
import {
  useTrainingJob,
  useApproveCost,
  useCancelTrainingJob,
} from "@/hooks/use-training";
import { useTrainingMetricsStream } from "@/hooks/use-training-metrics";
import { useMemo, useEffect } from "react";
import { Breadcrumbs } from "@/components/breadcrumbs";

function StatusBadge({ status }: { status: string }) {
  const colors: Record<string, string> = {
    pending:
      "bg-zinc-100 text-zinc-600 border-zinc-300 dark:bg-zinc-800 dark:text-zinc-400 dark:border-zinc-700",
    cost_approval:
      "bg-amber-50 text-amber-700 border-amber-200 dark:bg-amber-900/50 dark:text-amber-400 dark:border-amber-800",
    provisioning:
      "bg-blue-50 text-blue-700 border-blue-200 dark:bg-blue-900/50 dark:text-blue-400 dark:border-blue-800",
    training:
      "bg-violet-50 text-violet-700 border-violet-200 dark:bg-violet-900/50 dark:text-violet-400 dark:border-violet-800 animate-pulse",
    completed:
      "bg-emerald-50 text-emerald-700 border-emerald-200 dark:bg-emerald-900/50 dark:text-emerald-400 dark:border-emerald-800",
    failed:
      "bg-red-50 text-red-700 border-red-200 dark:bg-red-900/50 dark:text-red-400 dark:border-red-800",
    cancelled:
      "bg-zinc-100 text-zinc-500 border-zinc-300 dark:bg-zinc-800 dark:text-zinc-500 dark:border-zinc-700",
  };

  const cls =
    colors[status] ||
    "bg-zinc-100 text-zinc-600 border-zinc-300 dark:bg-zinc-800 dark:text-zinc-400 dark:border-zinc-700";

  return (
    <span
      className={`inline-flex items-center rounded-full border px-2.5 py-0.5 text-sm font-medium ${cls}`}
    >
      {status.replace("_", " ")}
    </span>
  );
}

function MetricsChart({
  metrics,
}: {
  metrics: { step: number; loss: number }[];
}) {
  // Simple ASCII-style chart using CSS bars
  const maxLoss = useMemo(
    () => Math.max(...metrics.map((m) => m.loss), 0.001),
    [metrics],
  );

  if (metrics.length === 0) {
    return (
      <div className="flex items-center justify-center h-40 text-zinc-400 dark:text-zinc-600 text-sm">
        Waiting for training metrics...
      </div>
    );
  }

  // Show last 50 data points for readability
  const displayMetrics = metrics.slice(-50);

  return (
    <div className="space-y-1">
      <div className="flex items-end gap-[2px] h-40">
        {displayMetrics.map((m, i) => {
          const height = Math.max(2, (m.loss / maxLoss) * 100);
          return (
            <div
              key={i}
              className="bg-violet-500 dark:bg-violet-500/80 hover:bg-violet-400 transition rounded-t flex-1 min-w-[3px]"
              style={{ height: `${height}%` }}
              title={`Step ${m.step}: loss=${m.loss.toFixed(4)}`}
            />
          );
        })}
      </div>
      <div className="flex justify-between text-xs text-zinc-400 dark:text-zinc-600">
        <span>Step {displayMetrics[0]?.step ?? 0}</span>
        <span>
          Loss:{" "}
          {displayMetrics[displayMetrics.length - 1]?.loss.toFixed(4) ?? "-"}
        </span>
        <span>Step {displayMetrics[displayMetrics.length - 1]?.step ?? 0}</span>
      </div>
    </div>
  );
}

export default function TrainingJobDetailPage() {
  const params = useParams<{ id: string; jobId: string }>();
  const { data: job, isLoading, error } = useTrainingJob(params.jobId);
  const approveCost = useApproveCost(params.id);
  const cancelJob = useCancelTrainingJob(params.id);

  useEffect(() => {
    if (approveCost.isSuccess)
      toast.success("Cost approved — training started");
  }, [approveCost.isSuccess]);

  useEffect(() => {
    if (approveCost.isError) toast.error(approveCost.error.message);
  }, [approveCost.isError, approveCost.error]);

  useEffect(() => {
    if (cancelJob.isSuccess) toast.success("Training job cancelled");
  }, [cancelJob.isSuccess]);

  useEffect(() => {
    if (cancelJob.isError) toast.error(cancelJob.error.message);
  }, [cancelJob.isError, cancelJob.error]);

  const isActiveTraining =
    job?.status === "training" || job?.status === "provisioning";
  const { metrics: streamedMetrics, connected } = useTrainingMetricsStream(
    params.jobId,
    isActiveTraining,
  );

  const chartMetrics = useMemo(
    () =>
      streamedMetrics
        .filter((m) => m.loss > 0)
        .map((m) => ({ step: Number(m.step), loss: Number(m.loss) })),
    [streamedMetrics],
  );

  if (isLoading) {
    return (
      <div className="flex items-center justify-center py-20">
        <p className="text-zinc-500">Loading training job...</p>
      </div>
    );
  }

  if (error || !job) {
    return (
      <div className="flex flex-col items-center justify-center py-20 gap-4">
        <p className="text-zinc-500">Training job not found</p>
        <Link
          href={`/projects/${params.id}`}
          className="text-sm text-zinc-900 dark:text-white underline hover:no-underline"
        >
          Back to Project
        </Link>
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
            { label: job.base_model.split("/").pop() ?? "Training" },
          ]}
        />
        <div className="flex items-center gap-3">
          <h1 className="text-2xl font-bold text-zinc-900 dark:text-white">
            {job.base_model.split("/").pop()}
          </h1>
          <StatusBadge status={job.status} />
          {isActiveTraining && connected && (
            <span className="inline-flex items-center gap-1 text-xs text-emerald-600 dark:text-emerald-400">
              <span className="w-1.5 h-1.5 rounded-full bg-emerald-600 dark:bg-emerald-400 animate-pulse" />
              Live
            </span>
          )}
        </div>
        <p className="text-zinc-500 mt-1">
          {job.method.toUpperCase()} &middot; {job.mode} mode
          {job.cost_estimate != null &&
            ` \u00b7 Est. $${job.cost_estimate.toFixed(2)}`}
        </p>
      </div>

      {/* Cost approval banner */}
      {job.status === "cost_approval" && (
        <div className="mb-6 rounded-lg border border-amber-200 dark:border-amber-800 bg-amber-50 dark:bg-amber-900/20 p-4">
          <div className="flex items-center justify-between">
            <div>
              <p className="text-sm font-medium text-amber-700 dark:text-amber-400">
                Cost Approval Required
              </p>
              <p className="text-sm text-amber-600 dark:text-amber-300/70 mt-0.5">
                Estimated cost of ${job.cost_estimate?.toFixed(2) ?? "?"}{" "}
                exceeds the approval threshold. Approve to start training or
                cancel.
              </p>
            </div>
            <div className="flex gap-2 ml-4">
              <button
                onClick={() => approveCost.mutate(params.jobId)}
                disabled={approveCost.isPending}
                className="rounded-lg bg-emerald-600 px-4 py-1.5 text-sm font-medium text-white hover:bg-emerald-500 transition disabled:opacity-50"
              >
                {approveCost.isPending ? "Approving..." : "Approve"}
              </button>
              <button
                onClick={() => cancelJob.mutate(params.jobId)}
                disabled={cancelJob.isPending}
                className="rounded-lg bg-zinc-200 dark:bg-zinc-700 px-4 py-1.5 text-sm font-medium text-zinc-700 dark:text-zinc-300 hover:bg-zinc-300 dark:hover:bg-zinc-600 transition disabled:opacity-50"
              >
                {cancelJob.isPending ? "Cancelling..." : "Reject"}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Training progress bar */}
      {isActiveTraining &&
        streamedMetrics.length > 0 &&
        (() => {
          const totalEpochs =
            (job.hyperparams as Record<string, unknown>)?.num_train_epochs ??
            (job.hyperparams as Record<string, unknown>)?.epochs ??
            3;
          const currentEpoch = Number(
            streamedMetrics[streamedMetrics.length - 1]?.epoch ?? 0,
          );
          const pct = Math.min(100, (currentEpoch / Number(totalEpochs)) * 100);
          return (
            <div className="mb-6">
              <div className="flex justify-between text-sm mb-1.5">
                <span className="text-zinc-600 dark:text-zinc-400">
                  Epoch {currentEpoch.toFixed(2)} / {String(totalEpochs)}
                </span>
                <span className="text-zinc-600 dark:text-zinc-400">{pct.toFixed(0)}%</span>
              </div>
              <div className="h-2 rounded-full bg-zinc-100 dark:bg-zinc-800 overflow-hidden">
                <div
                  className="h-full rounded-full bg-violet-500 transition-all duration-500"
                  style={{ width: `${pct}%` }}
                />
              </div>
            </div>
          );
        })()}

      {/* Real-time loss chart */}
      <div className="mb-8">
        <h2 className="text-lg font-semibold text-zinc-900 dark:text-white mb-4">Training Loss</h2>
        <div className="rounded-lg border border-zinc-200 dark:border-zinc-800 p-4">
          <MetricsChart metrics={chartMetrics} />
        </div>
      </div>

      {/* Live metrics table */}
      {streamedMetrics.length > 0 && (
        <div className="mb-8">
          <h2 className="text-lg font-semibold text-zinc-900 dark:text-white mb-4">Metrics Log</h2>
          <div className="rounded-lg border border-zinc-200 dark:border-zinc-800 overflow-hidden">
            <div className="overflow-x-auto">
              <table className="w-full text-sm">
                <thead>
                  <tr className="border-b border-zinc-200 dark:border-zinc-800 bg-zinc-50/50 dark:bg-zinc-900/50">
                    <th className="text-left text-zinc-500 px-4 py-2 font-medium">
                      Step
                    </th>
                    <th className="text-left text-zinc-500 px-4 py-2 font-medium">
                      Epoch
                    </th>
                    <th className="text-left text-zinc-500 px-4 py-2 font-medium">
                      Loss
                    </th>
                    <th className="text-left text-zinc-500 px-4 py-2 font-medium">
                      LR
                    </th>
                    <th className="text-left text-zinc-500 px-4 py-2 font-medium">
                      Grad Norm
                    </th>
                    <th className="text-left text-zinc-500 px-4 py-2 font-medium">
                      Phase
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {streamedMetrics
                    .slice(-20)
                    .reverse()
                    .map((m, i) => (
                      <tr
                        key={i}
                        className="border-b border-zinc-200/50 dark:border-zinc-800/50 last:border-b-0"
                      >
                        <td className="px-4 py-1.5 text-zinc-900 dark:text-white font-mono">
                          {m.step}
                        </td>
                        <td className="px-4 py-1.5 text-zinc-600 dark:text-zinc-400 font-mono">
                          {Number(m.epoch).toFixed(2)}
                        </td>
                        <td className="px-4 py-1.5 text-zinc-600 dark:text-zinc-400 font-mono">
                          {Number(m.loss).toFixed(4)}
                        </td>
                        <td className="px-4 py-1.5 text-zinc-600 dark:text-zinc-400 font-mono">
                          {Number(m.learning_rate).toExponential(2)}
                        </td>
                        <td className="px-4 py-1.5 text-zinc-600 dark:text-zinc-400 font-mono">
                          {Number(m.grad_norm).toFixed(2)}
                        </td>
                        <td className="px-4 py-1.5 text-zinc-500">{m.phase}</td>
                      </tr>
                    ))}
                </tbody>
              </table>
            </div>
          </div>
        </div>
      )}

      {/* Job details grid */}
      <div className="grid grid-cols-1 md:grid-cols-2 gap-4 mb-8">
        <div className="rounded-lg border border-zinc-200 dark:border-zinc-800 p-4">
          <p className="text-xs text-zinc-500 uppercase tracking-wider mb-2">
            Configuration
          </p>
          <div className="space-y-1">
            <div className="flex justify-between text-sm">
              <span className="text-zinc-500">Base Model</span>
              <span className="text-zinc-900 dark:text-white">{job.base_model}</span>
            </div>
            <div className="flex justify-between text-sm">
              <span className="text-zinc-500">Method</span>
              <span className="text-zinc-900 dark:text-white">{job.method.toUpperCase()}</span>
            </div>
            <div className="flex justify-between text-sm">
              <span className="text-zinc-500">Mode</span>
              <span className="text-zinc-900 dark:text-white">{job.mode}</span>
            </div>
            {job.gpu_class && (
              <div className="flex justify-between text-sm">
                <span className="text-zinc-500">GPU Class</span>
                <span className="text-zinc-900 dark:text-white">
                  {job.gpu_class.toUpperCase()}
                </span>
              </div>
            )}
          </div>
        </div>

        <div className="rounded-lg border border-zinc-200 dark:border-zinc-800 p-4">
          <p className="text-xs text-zinc-500 uppercase tracking-wider mb-2">
            Timing & Cost
          </p>
          <div className="space-y-1">
            {job.started_at && (
              <div className="flex justify-between text-sm">
                <span className="text-zinc-500">Started</span>
                <span className="text-zinc-900 dark:text-white">
                  {new Date(job.started_at).toLocaleString()}
                </span>
              </div>
            )}
            {job.completed_at && (
              <div className="flex justify-between text-sm">
                <span className="text-zinc-500">Completed</span>
                <span className="text-zinc-900 dark:text-white">
                  {new Date(job.completed_at).toLocaleString()}
                </span>
              </div>
            )}
            {job.started_at && job.completed_at && (
              <div className="flex justify-between text-sm">
                <span className="text-zinc-500">Duration</span>
                <span className="text-zinc-900 dark:text-white">
                  {(() => {
                    const ms =
                      new Date(job.completed_at).getTime() -
                      new Date(job.started_at).getTime();
                    const mins = Math.floor(ms / 60000);
                    const hrs = Math.floor(mins / 60);
                    if (hrs > 0) return `${hrs}h ${mins % 60}m`;
                    return `${mins}m`;
                  })()}
                </span>
              </div>
            )}
            {job.cost_estimate != null && (
              <div className="flex justify-between text-sm">
                <span className="text-zinc-500">Estimated Cost</span>
                <span className="text-zinc-900 dark:text-white">
                  ${job.cost_estimate.toFixed(2)}
                </span>
              </div>
            )}
            {job.actual_cost != null && (
              <div className="flex justify-between text-sm">
                <span className="text-zinc-500">Actual Cost</span>
                <span className="text-zinc-900 dark:text-white font-medium">
                  ${job.actual_cost.toFixed(2)}
                </span>
              </div>
            )}
            {job.cost_estimate != null && job.actual_cost != null && (
              <div className="flex justify-between text-sm">
                <span className="text-zinc-500">Cost Variance</span>
                {(() => {
                  const diff = job.actual_cost! - job.cost_estimate!;
                  const pct =
                    job.cost_estimate! > 0
                      ? (diff / job.cost_estimate!) * 100
                      : 0;
                  const isOver = diff > 0;
                  return (
                    <span
                      className={isOver ? "text-amber-600 dark:text-amber-400" : "text-emerald-600 dark:text-emerald-400"}
                    >
                      {isOver ? "+" : ""}${diff.toFixed(2)} ({isOver ? "+" : ""}
                      {pct.toFixed(0)}%)
                    </span>
                  );
                })()}
              </div>
            )}
          </div>
        </div>
      </div>

      {/* Hyperparameters */}
      <div className="mb-8">
        <h2 className="text-lg font-semibold text-zinc-900 dark:text-white mb-4">
          Hyperparameters
        </h2>
        <div className="rounded-lg border border-zinc-200 dark:border-zinc-800 p-4">
          <pre className="text-sm text-zinc-600 dark:text-zinc-400 font-mono overflow-x-auto">
            {JSON.stringify(job.hyperparams, null, 2)}
          </pre>
        </div>
      </div>

      {/* Error message */}
      {job.error_message && (
        <div className="mb-8 rounded-lg border border-red-200 dark:border-red-900/50 bg-red-50 dark:bg-red-900/10 p-4">
          <p className="text-sm font-medium text-red-600 dark:text-red-400 mb-1">Error</p>
          <p className="text-sm text-red-600 dark:text-red-300/80 font-mono">
            {job.error_message}
          </p>
        </div>
      )}
    </div>
  );
}
