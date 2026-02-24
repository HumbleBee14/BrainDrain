import type { DatasetStatus } from "@/lib/generated";

const colors: Record<DatasetStatus, string> = {
  generating: "bg-amber-900/50 text-amber-400 border-amber-800",
  review_pending: "bg-blue-900/50 text-blue-400 border-blue-800",
  approved: "bg-emerald-900/50 text-emerald-400 border-emerald-800",
  archived: "bg-zinc-800 text-zinc-500 border-zinc-700",
};

export function DatasetStatusBadge({ status }: { status: DatasetStatus }) {
  const cls = colors[status] ?? "bg-zinc-800 text-zinc-400 border-zinc-700";

  return (
    <span
      className={`inline-flex items-center rounded-full border px-2 py-0.5 text-xs font-medium ${cls}`}
    >
      {status.replace("_", " ")}
    </span>
  );
}
