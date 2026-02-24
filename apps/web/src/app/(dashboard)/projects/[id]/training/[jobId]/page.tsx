"use client";

import { useParams } from "next/navigation";
import Link from "next/link";
import { useTrainingJob } from "@/hooks/use-training";
import { useTrainingMetricsStream } from "@/hooks/use-training-metrics";
import { useMemo } from "react";

function StatusBadge({ status }: { status: string }) {
  const colors: Record<string, string> = {
    pending: "bg-zinc-800 text-zinc-400 border-zinc-700",
    cost_approval: "bg-amber-900/50 text-amber-400 border-amber-800",
    provisioning: "bg-blue-900/50 text-blue-400 border-blue-800",
    training:
      "bg-violet-900/50 text-violet-400 border-violet-800 animate-pulse",
    completed: "bg-emerald-900/50 text-emerald-400 border-emerald-800",
    failed: "bg-red-900/50 text-red-400 border-red-800",
    cancelled: "bg-zinc-800 text-zinc-500 border-zinc-700",
  };

  const cls = colors[status] || "bg-zinc-800 text-zinc-400 border-zinc-700";

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
      <div className="flex items-center justify-center h-40 text-zinc-600 text-sm">
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
              className="bg-violet-500/80 hover:bg-violet-400 transition rounded-t flex-1 min-w-[3px]"
              style={{ height: `${height}%` }}
              title={`Step ${m.step}: loss=${m.loss.toFixed(4)}`}
            />
          );
        })}
      </div>
      <div className="flex justify-between text-xs text-zinc-600">
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
          className="text-sm text-white underline hover:no-underline"
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
        <Link
          href={`/projects/${params.id}`}
          className="text-sm text-zinc-500 hover:text-zinc-300 transition"
        >
          &larr; Back to Project
        </Link>
        <div className="flex items-center gap-3 mt-2">
          <h1 className="text-2xl font-bold text-white">
            {job.base_model.split("/").pop()}
          </h1>
          <StatusBadge status={job.status} />
          {isActiveTraining && connected && (
            <span className="inline-flex items-center gap-1 text-xs text-emerald-400">
              <span className="w-1.5 h-1.5 rounded-full bg-emerald-400 animate-pulse" />
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

      {/* Real-time loss chart */}
      <div className="mb-8">
        <h2 className="text-lg font-semibold text-white mb-4">Training Loss</h2>
        <div className="rounded-lg border border-zinc-800 p-4">
          <MetricsChart metrics={chartMetrics} />
        </div>
      </div>

      {/* Live metrics table */}
      {streamedMetrics.length > 0 && (
        <div className="mb-8">
          <h2 className="text-lg font-semibold text-white mb-4">Metrics Log</h2>
          <div className="rounded-lg border border-zinc-800 overflow-hidden">
            <div className="overflow-x-auto">
              <table className="w-full text-sm">
                <thead>
                  <tr className="border-b border-zinc-800 bg-zinc-900/50">
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
                        className="border-b border-zinc-800/50 last:border-b-0"
                      >
                        <td className="px-4 py-1.5 text-white font-mono">
                          {m.step}
                        </td>
                        <td className="px-4 py-1.5 text-zinc-400 font-mono">
                          {Number(m.epoch).toFixed(2)}
                        </td>
                        <td className="px-4 py-1.5 text-zinc-400 font-mono">
                          {Number(m.loss).toFixed(4)}
                        </td>
                        <td className="px-4 py-1.5 text-zinc-400 font-mono">
                          {Number(m.learning_rate).toExponential(2)}
                        </td>
                        <td className="px-4 py-1.5 text-zinc-400 font-mono">
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
        <div className="rounded-lg border border-zinc-800 p-4">
          <p className="text-xs text-zinc-500 uppercase tracking-wider mb-2">
            Configuration
          </p>
          <div className="space-y-1">
            <div className="flex justify-between text-sm">
              <span className="text-zinc-500">Base Model</span>
              <span className="text-white">{job.base_model}</span>
            </div>
            <div className="flex justify-between text-sm">
              <span className="text-zinc-500">Method</span>
              <span className="text-white">{job.method.toUpperCase()}</span>
            </div>
            <div className="flex justify-between text-sm">
              <span className="text-zinc-500">Mode</span>
              <span className="text-white">{job.mode}</span>
            </div>
            {job.gpu_class && (
              <div className="flex justify-between text-sm">
                <span className="text-zinc-500">GPU Class</span>
                <span className="text-white">
                  {job.gpu_class.toUpperCase()}
                </span>
              </div>
            )}
          </div>
        </div>

        <div className="rounded-lg border border-zinc-800 p-4">
          <p className="text-xs text-zinc-500 uppercase tracking-wider mb-2">
            Timing & Cost
          </p>
          <div className="space-y-1">
            {job.started_at && (
              <div className="flex justify-between text-sm">
                <span className="text-zinc-500">Started</span>
                <span className="text-white">
                  {new Date(job.started_at).toLocaleString()}
                </span>
              </div>
            )}
            {job.completed_at && (
              <div className="flex justify-between text-sm">
                <span className="text-zinc-500">Completed</span>
                <span className="text-white">
                  {new Date(job.completed_at).toLocaleString()}
                </span>
              </div>
            )}
            {job.cost_estimate != null && (
              <div className="flex justify-between text-sm">
                <span className="text-zinc-500">Estimated Cost</span>
                <span className="text-white">
                  ${job.cost_estimate.toFixed(2)}
                </span>
              </div>
            )}
            {job.actual_cost != null && (
              <div className="flex justify-between text-sm">
                <span className="text-zinc-500">Actual Cost</span>
                <span className="text-white">
                  ${job.actual_cost.toFixed(2)}
                </span>
              </div>
            )}
          </div>
        </div>
      </div>

      {/* Hyperparameters */}
      <div className="mb-8">
        <h2 className="text-lg font-semibold text-white mb-4">
          Hyperparameters
        </h2>
        <div className="rounded-lg border border-zinc-800 p-4">
          <pre className="text-sm text-zinc-400 font-mono overflow-x-auto">
            {JSON.stringify(job.hyperparams, null, 2)}
          </pre>
        </div>
      </div>

      {/* Error message */}
      {job.error_message && (
        <div className="mb-8 rounded-lg border border-red-900/50 bg-red-900/10 p-4">
          <p className="text-sm font-medium text-red-400 mb-1">Error</p>
          <p className="text-sm text-red-300/80 font-mono">
            {job.error_message}
          </p>
        </div>
      )}
    </div>
  );
}
