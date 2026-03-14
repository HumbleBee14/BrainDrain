export default function DashboardLoading() {
  return (
    <div className="animate-pulse space-y-6">
      <div className="h-8 w-48 bg-zinc-100 dark:bg-zinc-800 rounded" />
      <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
        {[1, 2, 3].map((i) => (
          <div key={i} className="h-28 bg-zinc-100/50 dark:bg-zinc-800/50 rounded-lg" />
        ))}
      </div>
      <div className="h-64 bg-zinc-100/30 dark:bg-zinc-800/30 rounded-lg" />
    </div>
  );
}
