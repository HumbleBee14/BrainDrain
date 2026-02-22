import type { DocumentStatus } from "@/lib/generated";

const colors: Record<DocumentStatus, string> = {
  uploaded: "bg-blue-900/50 text-blue-400 border-blue-800",
  scanning: "bg-amber-900/50 text-amber-400 border-amber-800",
  parsing: "bg-amber-900/50 text-amber-400 border-amber-800",
  parsed: "bg-emerald-900/50 text-emerald-400 border-emerald-800",
  failed: "bg-red-900/50 text-red-400 border-red-800",
};

export function DocStatusBadge({ status }: { status: DocumentStatus }) {
  const cls = colors[status] || "bg-zinc-800 text-zinc-400 border-zinc-700";

  return (
    <span className={`inline-flex items-center rounded-full border px-2 py-0.5 text-xs font-medium ${cls}`}>
      {status}
    </span>
  );
}
