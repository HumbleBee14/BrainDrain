export function PipelineStageCard({
  label,
  count,
  active,
}: {
  label: string;
  count: number;
  active: boolean;
}) {
  return (
    <div
      className={`rounded-lg border p-4 text-center ${
        active
          ? "border-emerald-800 bg-emerald-900/20"
          : "border-zinc-800"
      }`}
    >
      <p className="text-2xl font-bold text-white">{count}</p>
      <p className="text-xs text-zinc-500 mt-1">{label}</p>
    </div>
  );
}
