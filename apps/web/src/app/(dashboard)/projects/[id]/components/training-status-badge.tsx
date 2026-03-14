export function TrainingStatusBadge({ status }: { status: string }) {
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
      "bg-zinc-100 text-zinc-600 border-zinc-300 dark:bg-zinc-800 dark:text-zinc-500 dark:border-zinc-700",
  };

  const cls =
    colors[status] ||
    "bg-zinc-100 text-zinc-600 border-zinc-300 dark:bg-zinc-800 dark:text-zinc-400 dark:border-zinc-700";

  return (
    <span
      className={`inline-flex items-center rounded-full border px-2 py-0.5 text-xs font-medium ${cls}`}
    >
      {status.replace("_", " ")}
    </span>
  );
}
