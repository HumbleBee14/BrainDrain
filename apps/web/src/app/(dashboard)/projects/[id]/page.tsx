"use client";

import { useParams, useRouter } from "next/navigation";
import Link from "next/link";
import { useProject, useDeleteProject } from "@/hooks/use-projects";
import { useState } from "react";

function StatusBadge({ status }: { status: string }) {
  const colors: Record<string, string> = {
    active: "bg-emerald-900/50 text-emerald-400 border-emerald-800",
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

export default function ProjectDetailPage() {
  const params = useParams<{ id: string }>();
  const router = useRouter();
  const { data: project, isLoading, error } = useProject(params.id);
  const deleteProject = useDeleteProject();
  const [confirmDelete, setConfirmDelete] = useState(false);

  const handleDelete = async () => {
    if (!confirmDelete) {
      setConfirmDelete(true);
      return;
    }
    await deleteProject.mutateAsync(params.id);
    router.push("/projects");
  };

  if (isLoading) {
    return (
      <div className="flex items-center justify-center py-20">
        <p className="text-zinc-500">Loading project...</p>
      </div>
    );
  }

  if (error || !project) {
    return (
      <div className="flex flex-col items-center justify-center py-20 gap-4">
        <p className="text-zinc-500">Project not found</p>
        <Link href="/projects" className="text-sm text-white underline hover:no-underline">
          Back to Projects
        </Link>
      </div>
    );
  }

  return (
    <div>
      {/* Header */}
      <div className="mb-8">
        <Link
          href="/projects"
          className="text-sm text-zinc-500 hover:text-zinc-300 transition"
        >
          &larr; Back to Projects
        </Link>
        <div className="flex items-center gap-3 mt-2">
          <h1 className="text-2xl font-bold text-white">{project.name}</h1>
          <StatusBadge status={project.status} />
        </div>
        {project.description && (
          <p className="text-zinc-500 mt-1">{project.description}</p>
        )}
      </div>

      {/* Info grid */}
      <div className="grid grid-cols-1 md:grid-cols-3 gap-4 mb-8">
        <div className="rounded-lg border border-zinc-800 p-4">
          <p className="text-xs text-zinc-500 uppercase tracking-wider">Task Type</p>
          <p className="text-white mt-1">{project.task_type || "Not set"}</p>
        </div>
        <div className="rounded-lg border border-zinc-800 p-4">
          <p className="text-xs text-zinc-500 uppercase tracking-wider">Created</p>
          <p className="text-white mt-1">
            {new Date(project.created_at).toLocaleDateString()}
          </p>
        </div>
        <div className="rounded-lg border border-zinc-800 p-4">
          <p className="text-xs text-zinc-500 uppercase tracking-wider">Updated</p>
          <p className="text-white mt-1">
            {new Date(project.updated_at).toLocaleDateString()}
          </p>
        </div>
      </div>

      {/* Documents section */}
      <div className="mb-8">
        <div className="flex items-center justify-between mb-4">
          <h2 className="text-lg font-semibold text-white">Documents</h2>
        </div>
        <div className="rounded-lg border border-dashed border-zinc-700 p-8 text-center">
          <p className="text-zinc-500 mb-2">Drop files here or click to upload</p>
          <p className="text-xs text-zinc-600">
            Supports PDF, DOCX, TXT, CSV, JSON, JSONL (max 500 MB)
          </p>
          <label className="mt-4 inline-block cursor-pointer rounded-lg bg-zinc-800 px-4 py-2 text-sm text-white hover:bg-zinc-700 transition">
            Choose Files
            <input type="file" multiple className="hidden" accept=".pdf,.docx,.txt,.csv,.json,.jsonl,.parquet,.md" />
          </label>
        </div>
      </div>

      {/* Pipeline stages (placeholder) */}
      <div className="mb-8">
        <h2 className="text-lg font-semibold text-white mb-4">Pipeline</h2>
        <div className="grid grid-cols-2 md:grid-cols-5 gap-3">
          {["Upload", "Parse", "Refine", "Train", "Evaluate"].map((stage, i) => (
            <div key={stage} className="rounded-lg border border-zinc-800 p-4 text-center">
              <p className="text-xs text-zinc-600 mb-1">Stage {i + 1}</p>
              <p className="text-sm text-zinc-400">{stage}</p>
            </div>
          ))}
        </div>
      </div>

      {/* Danger zone */}
      <div className="rounded-lg border border-zinc-800 p-6">
        <h3 className="text-sm font-medium text-zinc-400 mb-4">Danger Zone</h3>
        <div className="flex items-center justify-between">
          <div>
            <p className="text-sm text-white">Delete this project</p>
            <p className="text-xs text-zinc-600">This action cannot be undone.</p>
          </div>
          <button
            onClick={handleDelete}
            disabled={deleteProject.isPending}
            className={`rounded-lg px-4 py-2 text-sm font-medium transition ${
              confirmDelete
                ? "bg-red-600 text-white hover:bg-red-500"
                : "border border-red-800 text-red-400 hover:bg-red-900/30"
            } disabled:opacity-50`}
          >
            {deleteProject.isPending
              ? "Deleting..."
              : confirmDelete
                ? "Confirm Delete"
                : "Delete Project"}
          </button>
        </div>
      </div>
    </div>
  );
}
