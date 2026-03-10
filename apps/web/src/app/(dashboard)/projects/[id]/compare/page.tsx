"use client";

import { useParams, useSearchParams } from "next/navigation";
import { useMemo } from "react";
import { useTrainingJob } from "@/hooks/use-training";
import { Breadcrumbs } from "@/components/breadcrumbs";
import type { TrainingJob } from "@/lib/api-client";

function ComparisonRow({
  label,
  values,
  highlight,
}: {
  label: string;
  values: (string | null | undefined)[];
  highlight?: "lower" | "higher";
}) {
  const numericValues = values.map((v) => (v ? parseFloat(v) : NaN));
  const allNumeric = numericValues.every((n) => !isNaN(n));

  return (
    <div className="grid grid-cols-1 md:grid-cols-[200px,1fr,1fr] gap-1 md:gap-4 py-2 border-b border-zinc-200/50 dark:border-zinc-800/50 last:border-b-0">
      <span className="text-sm text-zinc-500">{label}</span>
      {values.map((val, i) => {
        let cls = "text-sm text-zinc-900 dark:text-white font-mono";
        if (allNumeric && highlight && values.length === 2) {
          const isBetter =
            highlight === "lower"
              ? numericValues[i] <= numericValues[1 - i]
              : numericValues[i] >= numericValues[1 - i];
          if (isBetter && numericValues[0] !== numericValues[1]) {
            cls += " text-emerald-400";
          }
        }
        return (
          <span key={i} className={cls}>
            {val ?? "-"}
          </span>
        );
      })}
    </div>
  );
}

function JobColumn({ job }: { job: TrainingJob }) {
  const duration = useMemo(() => {
    if (!job.started_at || !job.completed_at) return null;
    const ms =
      new Date(job.completed_at).getTime() -
      new Date(job.started_at).getTime();
    const mins = Math.floor(ms / 60000);
    const hrs = Math.floor(mins / 60);
    if (hrs > 0) return `${hrs}h ${mins % 60}m`;
    return `${mins}m`;
  }, [job.started_at, job.completed_at]);

  return (
    <div className="text-sm">
      <p className="text-zinc-900 dark:text-white font-semibold">
        {job.base_model.split("/").pop()}
      </p>
      <p className="text-zinc-500">
        {String(job.method).toUpperCase()} / {job.mode}
      </p>
      {duration && <p className="text-zinc-400 dark:text-zinc-600 text-xs">{duration}</p>}
    </div>
  );
}

function StatusBadge({ status }: { status: string }) {
  const colors: Record<string, string> = {
    completed: "bg-emerald-50 text-emerald-700 border-emerald-200 dark:bg-emerald-900/50 dark:text-emerald-400 dark:border-emerald-800",
    failed: "bg-red-50 text-red-700 border-red-200 dark:bg-red-900/50 dark:text-red-400 dark:border-red-800",
    training: "bg-violet-50 text-violet-700 border-violet-200 dark:bg-violet-900/50 dark:text-violet-400 dark:border-violet-800",
    pending: "bg-zinc-100 text-zinc-600 border-zinc-300 dark:bg-zinc-800 dark:text-zinc-400 dark:border-zinc-700",
    cancelled: "bg-zinc-100 text-zinc-500 border-zinc-300 dark:bg-zinc-800 dark:text-zinc-500 dark:border-zinc-700",
  };
  const cls =
    colors[status] || "bg-zinc-100 text-zinc-600 border-zinc-300 dark:bg-zinc-800 dark:text-zinc-400 dark:border-zinc-700";
  return (
    <span
      className={`inline-flex items-center rounded-full border px-2 py-0.5 text-xs font-medium ${cls}`}
    >
      {status.replace("_", " ")}
    </span>
  );
}

function MetricsOverlay({
  jobs,
}: {
  jobs: TrainingJob[];
}) {
  const colors = ["bg-violet-500", "bg-emerald-500"];

  const maxLoss = useMemo(() => {
    let max = 0;
    for (const job of jobs) {
      const loss = (job.metrics as Record<string, unknown>)?.train_loss;
      if (typeof loss === "number" && loss > max) max = loss;
    }
    return max || 1;
  }, [jobs]);

  return (
    <div className="flex items-end gap-4 h-16">
      {jobs.map((job, i) => {
        const loss =
          ((job.metrics as Record<string, unknown>)?.train_loss as number) || 0;
        const pct = Math.max(5, (loss / maxLoss) * 100);
        return (
          <div key={job.id} className="flex items-end gap-1 flex-1">
            <div
              className={`${colors[i]} rounded-t flex-1 transition-all`}
              style={{ height: `${pct}%` }}
              title={`Final loss: ${loss.toFixed(4)}`}
            />
            <span className="text-xs text-zinc-500 font-mono">
              {loss > 0 ? loss.toFixed(4) : "-"}
            </span>
          </div>
        );
      })}
    </div>
  );
}

export default function TrainingComparisonPage() {
  const params = useParams<{ id: string }>();
  const searchParams = useSearchParams();

  const jobIds = useMemo(() => {
    const raw = searchParams.get("jobs") ?? "";
    return raw
      .split(",")
      .map((s) => s.trim())
      .filter(Boolean);
  }, [searchParams]);

  const job1Query = useTrainingJob(jobIds[0] ?? "", jobIds.length >= 1);
  const job2Query = useTrainingJob(jobIds[1] ?? "", jobIds.length >= 2);

  const isLoading = job1Query.isLoading || job2Query.isLoading;
  const jobs = [job1Query.data, job2Query.data].filter(
    (j): j is TrainingJob => !!j,
  );

  if (jobIds.length < 2) {
    return (
      <div className="flex flex-col items-center justify-center py-20 gap-4">
        <p className="text-zinc-500">
          Select two training jobs to compare. Use the compare button on the
          project page.
        </p>
      </div>
    );
  }

  if (isLoading) {
    return (
      <div className="flex items-center justify-center py-20">
        <p className="text-zinc-500">Loading training jobs...</p>
      </div>
    );
  }

  if (jobs.length < 2) {
    return (
      <div className="flex flex-col items-center justify-center py-20 gap-4">
        <p className="text-zinc-500">
          One or both training jobs could not be loaded.
        </p>
      </div>
    );
  }

  const hp = jobs.map(
    (j) => (j.hyperparams ?? {}) as Record<string, unknown>,
  );
  const metrics = jobs.map(
    (j) => (j.metrics ?? {}) as Record<string, unknown>,
  );

  return (
    <div>
      <div className="mb-8">
        <Breadcrumbs
          items={[
            { label: "Projects", href: "/projects" },
            { label: "Project", href: `/projects/${params.id}` },
            { label: "Compare Training Jobs" },
          ]}
        />
        <h1 className="text-xl md:text-2xl font-bold text-zinc-900 dark:text-white">
          Training Job Comparison
        </h1>
        <p className="text-zinc-500 mt-1">
          Side-by-side comparison of configuration, hyperparameters, and results
        </p>
      </div>

      {/* Job Headers */}
      <div className="grid grid-cols-1 md:grid-cols-[200px,1fr,1fr] gap-3 md:gap-4 mb-6">
        <div />
        {jobs.map((job, i) => (
          <div
            key={job.id}
            className={`rounded-lg border p-4 ${i === 0 ? "border-violet-200 dark:border-violet-800 bg-violet-50/10 dark:bg-violet-900/10" : "border-emerald-200 dark:border-emerald-800 bg-emerald-50/10 dark:bg-emerald-900/10"}`}
          >
            <div className="flex items-center gap-2 mb-1">
              <JobColumn job={job} />
              <StatusBadge status={job.status} />
            </div>
            <p className="text-xs text-zinc-400 dark:text-zinc-600 font-mono mt-1">
              {job.id.slice(0, 8)}
            </p>
          </div>
        ))}
      </div>

      {/* Final Loss Overlay */}
      <div className="rounded-lg border border-zinc-200 dark:border-zinc-800 p-4 mb-6">
        <h3 className="text-sm font-medium text-zinc-600 dark:text-zinc-400 mb-3">
          Final Training Loss
        </h3>
        <MetricsOverlay jobs={jobs} />
        <div className="flex gap-4 mt-2">
          <span className="flex items-center gap-1.5 text-xs text-zinc-500">
            <span className="w-2 h-2 rounded-full bg-violet-500" />
            {jobs[0].base_model.split("/").pop()}
          </span>
          <span className="flex items-center gap-1.5 text-xs text-zinc-500">
            <span className="w-2 h-2 rounded-full bg-emerald-500" />
            {jobs[1].base_model.split("/").pop()}
          </span>
        </div>
      </div>

      {/* Configuration */}
      <div className="rounded-lg border border-zinc-200 dark:border-zinc-800 p-4 mb-6">
        <h3 className="text-sm font-medium text-zinc-600 dark:text-zinc-400 mb-3 uppercase tracking-wider">
          Configuration
        </h3>
        <ComparisonRow
          label="Base Model"
          values={jobs.map((j) => j.base_model)}
        />
        <ComparisonRow
          label="Method"
          values={jobs.map((j) => String(j.method).toUpperCase())}
        />
        <ComparisonRow label="Mode" values={jobs.map((j) => j.mode)} />
        <ComparisonRow
          label="GPU Class"
          values={jobs.map((j) =>
            j.gpu_class ? j.gpu_class.toUpperCase() : null,
          )}
        />
        <ComparisonRow
          label="Status"
          values={jobs.map((j) => j.status)}
        />
      </div>

      {/* Hyperparameters */}
      <div className="rounded-lg border border-zinc-200 dark:border-zinc-800 p-4 mb-6">
        <h3 className="text-sm font-medium text-zinc-600 dark:text-zinc-400 mb-3 uppercase tracking-wider">
          Hyperparameters
        </h3>
        <ComparisonRow
          label="Learning Rate"
          values={hp.map((h) =>
            h.learning_rate != null ? String(h.learning_rate) : null,
          )}
        />
        <ComparisonRow
          label="Epochs"
          values={hp.map((h) =>
            h.num_train_epochs != null ? String(h.num_train_epochs) : null,
          )}
        />
        <ComparisonRow
          label="Batch Size"
          values={hp.map((h) =>
            h.per_device_train_batch_size != null
              ? String(h.per_device_train_batch_size)
              : null,
          )}
        />
        <ComparisonRow
          label="LoRA Rank (r)"
          values={hp.map((h) => (h.r != null ? String(h.r) : null))}
        />
        <ComparisonRow
          label="LoRA Alpha"
          values={hp.map((h) =>
            h.lora_alpha != null ? String(h.lora_alpha) : null,
          )}
        />
        <ComparisonRow
          label="Max Seq Length"
          values={hp.map((h) =>
            h.max_seq_length != null ? String(h.max_seq_length) : null,
          )}
        />
        <ComparisonRow
          label="Warmup Steps"
          values={hp.map((h) =>
            h.warmup_steps != null ? String(h.warmup_steps) : null,
          )}
        />
        <ComparisonRow
          label="Gradient Accum."
          values={hp.map((h) =>
            h.gradient_accumulation_steps != null
              ? String(h.gradient_accumulation_steps)
              : null,
          )}
        />
        <ComparisonRow
          label="Optimizer"
          values={hp.map((h) =>
            h.optim != null ? String(h.optim) : null,
          )}
        />
        <ComparisonRow
          label="LR Scheduler"
          values={hp.map((h) =>
            h.lr_scheduler_type != null
              ? String(h.lr_scheduler_type)
              : null,
          )}
        />
      </div>

      {/* Cost & Timing */}
      <div className="rounded-lg border border-zinc-200 dark:border-zinc-800 p-4 mb-6">
        <h3 className="text-sm font-medium text-zinc-600 dark:text-zinc-400 mb-3 uppercase tracking-wider">
          Cost & Timing
        </h3>
        <ComparisonRow
          label="Cost Estimate"
          values={jobs.map((j) =>
            j.cost_estimate != null ? `$${j.cost_estimate.toFixed(2)}` : null,
          )}
          highlight="lower"
        />
        <ComparisonRow
          label="Actual Cost"
          values={jobs.map((j) =>
            j.actual_cost != null ? `$${j.actual_cost.toFixed(2)}` : null,
          )}
          highlight="lower"
        />
        <ComparisonRow
          label="Duration"
          values={jobs.map((j) => {
            if (!j.started_at || !j.completed_at) return null;
            const ms =
              new Date(j.completed_at).getTime() -
              new Date(j.started_at).getTime();
            const mins = Math.floor(ms / 60000);
            const hrs = Math.floor(mins / 60);
            if (hrs > 0) return `${hrs}h ${mins % 60}m`;
            return `${mins}m`;
          })}
          highlight="lower"
        />
        <ComparisonRow
          label="Started"
          values={jobs.map((j) =>
            j.started_at
              ? new Date(j.started_at).toLocaleString()
              : null,
          )}
        />
      </div>

      {/* Training Results */}
      <div className="rounded-lg border border-zinc-200 dark:border-zinc-800 p-4 mb-6">
        <h3 className="text-sm font-medium text-zinc-600 dark:text-zinc-400 mb-3 uppercase tracking-wider">
          Training Results
        </h3>
        <ComparisonRow
          label="Final Loss"
          values={metrics.map((m) =>
            m.train_loss != null
              ? Number(m.train_loss).toFixed(4)
              : null,
          )}
          highlight="lower"
        />
        <ComparisonRow
          label="Total Steps"
          values={metrics.map((m) =>
            m.train_steps != null ? String(m.train_steps) : null,
          )}
        />
        <ComparisonRow
          label="Runtime (s)"
          values={metrics.map((m) =>
            m.train_runtime != null
              ? Number(m.train_runtime).toFixed(1)
              : null,
          )}
          highlight="lower"
        />
        <ComparisonRow
          label="Samples/sec"
          values={metrics.map((m) =>
            m.train_samples_per_second != null
              ? Number(m.train_samples_per_second).toFixed(2)
              : null,
          )}
          highlight="higher"
        />
      </div>
    </div>
  );
}
