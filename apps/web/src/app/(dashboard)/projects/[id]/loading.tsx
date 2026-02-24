export default function ProjectLoading() {
  return (
    <div className="animate-pulse space-y-6">
      <div className="h-6 w-32 bg-zinc-800 rounded" />
      <div className="flex items-center gap-3">
        <div className="h-8 w-56 bg-zinc-800 rounded" />
        <div className="h-6 w-20 bg-zinc-800/60 rounded-full" />
      </div>
      <div className="grid grid-cols-1 md:grid-cols-5 gap-4">
        {[1, 2, 3, 4, 5].map((i) => (
          <div key={i} className="h-24 bg-zinc-800/40 rounded-lg" />
        ))}
      </div>
      <div className="space-y-3">
        <div className="h-6 w-32 bg-zinc-800 rounded" />
        <div className="h-48 bg-zinc-800/30 rounded-lg" />
      </div>
    </div>
  );
}
