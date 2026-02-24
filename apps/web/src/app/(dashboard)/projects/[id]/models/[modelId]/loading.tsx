export default function ModelLoading() {
  return (
    <div className="animate-pulse space-y-6">
      <div className="h-6 w-40 bg-zinc-800 rounded" />
      <div className="flex items-center gap-3">
        <div className="h-8 w-48 bg-zinc-800 rounded" />
        <div className="h-6 w-24 bg-zinc-800/60 rounded-full" />
      </div>
      <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
        {[1, 2].map((i) => (
          <div key={i} className="h-32 bg-zinc-800/40 rounded-lg" />
        ))}
      </div>
      <div className="h-48 bg-zinc-800/30 rounded-lg" />
    </div>
  );
}
