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
          ? "border-emerald-200 bg-emerald-50/20 dark:border-emerald-800 dark:bg-emerald-900/20"
          : "border-zinc-200 dark:border-zinc-800"
      }`}
    >
      <p className="text-2xl font-bold text-zinc-900 dark:text-white">
        {count}
      </p>
      <p className="text-xs text-zinc-500 mt-1">{label}</p>
    </div>
  );
}
