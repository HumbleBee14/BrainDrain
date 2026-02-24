export default function DatasetLoading() {
  return (
    <div className="animate-pulse space-y-6">
      <div className="h-6 w-40 bg-zinc-100 dark:bg-zinc-800 rounded" />
      <div className="flex items-center gap-3">
        <div className="h-8 w-48 bg-zinc-100 dark:bg-zinc-800 rounded" />
        <div className="h-6 w-24 bg-zinc-100/60 dark:bg-zinc-800/60 rounded-full" />
      </div>
      <div className="h-8 w-full bg-zinc-100/30 dark:bg-zinc-800/30 rounded" />
      <div className="space-y-2">
        {[1, 2, 3, 4, 5].map((i) => (
          <div key={i} className="h-16 bg-zinc-100/20 dark:bg-zinc-800/20 rounded" />
        ))}
      </div>
    </div>
  );
}
