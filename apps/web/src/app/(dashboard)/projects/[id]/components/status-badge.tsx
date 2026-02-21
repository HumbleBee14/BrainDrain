export function StatusBadge({ status }: { status: string }) {
  const colors: Record<string, string> = {
    active: "bg-emerald-900/50 text-emerald-400 border-emerald-800",
    created: "bg-blue-900/50 text-blue-400 border-blue-800",
    archived: "bg-zinc-800 text-zinc-400 border-zinc-700",
    draft: "bg-amber-900/50 text-amber-400 border-amber-800",
  };

  const cls = colors[status] || "bg-zinc-800 text-zinc-400 border-zinc-700";

  return (
    <span className={`inline-flex items-center rounded-full border px-2.5 py-0.5 text-xs font-medium ${cls}`}>
      {status}
    </span>
  );
}
