import Link from "next/link";

export default function DashboardPage() {
  return (
    <div>
      <div className="flex items-center justify-between mb-8">
        <h1 className="text-2xl font-bold text-white">Dashboard</h1>
        <Link
          href="/projects/new"
          className="rounded-lg bg-white px-4 py-2 text-sm font-semibold text-zinc-950 hover:bg-zinc-200 transition"
        >
          New Project
        </Link>
      </div>

      <div className="grid grid-cols-1 md:grid-cols-3 gap-4 mb-8">
        <div className="rounded-lg border border-zinc-800 p-6">
          <p className="text-sm text-zinc-500">Projects</p>
          <p className="text-3xl font-bold text-white mt-1">0</p>
        </div>
        <div className="rounded-lg border border-zinc-800 p-6">
          <p className="text-sm text-zinc-500">Models Trained</p>
          <p className="text-3xl font-bold text-white mt-1">0</p>
        </div>
        <div className="rounded-lg border border-zinc-800 p-6">
          <p className="text-sm text-zinc-500">Documents</p>
          <p className="text-3xl font-bold text-white mt-1">0</p>
        </div>
      </div>

      <p className="text-zinc-500">
        Create a project to get started. Upload your documents and we will handle
        the rest.
      </p>
    </div>
  );
}
