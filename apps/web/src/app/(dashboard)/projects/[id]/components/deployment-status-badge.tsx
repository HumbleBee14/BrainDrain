import type { DeploymentStatus } from "@/lib/generated";

const colors: Record<DeploymentStatus, string> = {
  undeployed:
    "bg-zinc-100 text-zinc-600 border-zinc-300 dark:bg-zinc-800 dark:text-zinc-500 dark:border-zinc-700",
  deploying:
    "bg-amber-50 text-amber-700 border-amber-200 dark:bg-amber-900/50 dark:text-amber-400 dark:border-amber-800",
  active:
    "bg-emerald-50 text-emerald-700 border-emerald-200 dark:bg-emerald-900/50 dark:text-emerald-400 dark:border-emerald-800",
  inactive:
    "bg-red-50 text-red-700 border-red-200 dark:bg-red-900/50 dark:text-red-400 dark:border-red-800",
};

export function DeploymentStatusBadge({
  status,
}: {
  status: DeploymentStatus;
}) {
  const cls =
    colors[status] ??
    "bg-zinc-100 text-zinc-600 border-zinc-300 dark:bg-zinc-800 dark:text-zinc-400 dark:border-zinc-700";

  return (
    <span
      className={`inline-flex items-center rounded-full border px-2 py-0.5 text-xs font-medium ${cls}`}
    >
      {status}
    </span>
  );
}
