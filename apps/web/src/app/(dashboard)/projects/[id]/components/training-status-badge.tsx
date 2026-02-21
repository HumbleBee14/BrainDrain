export function TrainingStatusBadge({ status }: { status: string }) {
  const colors: Record<string, string> = {
    pending: "bg-zinc-800 text-zinc-400 border-zinc-700",
    cost_approval: "bg-amber-900/50 text-amber-400 border-amber-800",
    provisioning: "bg-blue-900/50 text-blue-400 border-blue-800",
    training: "bg-violet-900/50 text-violet-400 border-violet-800 animate-pulse",
    completed: "bg-emerald-900/50 text-emerald-400 border-emerald-800",
    failed: "bg-red-900/50 text-red-400 border-red-800",
    cancelled: "bg-zinc-800 text-zinc-500 border-zinc-700",
  };

  const cls = colors[status] || "bg-zinc-800 text-zinc-400 border-zinc-700";

  return (
    <span className={`inline-flex items-center rounded-full border px-2 py-0.5 text-xs font-medium ${cls}`}>
      {status.replace("_", " ")}
    </span>
  );
}
