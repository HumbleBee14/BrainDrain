"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import { useState } from "react";
import type { Project } from "@/lib/api-client";
import { useDeleteProject } from "@/hooks/use-projects";

export function SettingsTab({ project }: { project: Project }) {
  const router = useRouter();
  const deleteProject = useDeleteProject();
  const [confirmDelete, setConfirmDelete] = useState(false);

  const handleDelete = async () => {
    if (!confirmDelete) {
      setConfirmDelete(true);
      return;
    }
    try {
      await deleteProject.mutateAsync(project.id);
      router.push("/projects");
    } catch {
      setConfirmDelete(false);
    }
  };

  return (
    <div className="space-y-6">
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-3 md:gap-4">
        <div className="rounded-lg border border-zinc-200 p-4 dark:border-zinc-800">
          <p className="text-xs uppercase tracking-wider text-zinc-500">
            Task Type
          </p>
          <p className="mt-1 text-zinc-900 dark:text-white">
            {project.task_type || "Not set"}
          </p>
        </div>
        <div className="rounded-lg border border-zinc-200 p-4 dark:border-zinc-800">
          <p className="text-xs uppercase tracking-wider text-zinc-500">
            Created
          </p>
          <p className="mt-1 text-zinc-900 dark:text-white">
            {new Date(project.created_at).toLocaleDateString()}
          </p>
        </div>
        <div className="rounded-lg border border-zinc-200 p-4 dark:border-zinc-800">
          <p className="text-xs uppercase tracking-wider text-zinc-500">
            Updated
          </p>
          <p className="mt-1 text-zinc-900 dark:text-white">
            {new Date(project.updated_at).toLocaleDateString()}
          </p>
        </div>
      </div>

      <div className="rounded-lg border border-zinc-200 p-4 dark:border-zinc-800">
        <p className="text-xs uppercase tracking-wider text-zinc-500">
          Data Lineage
        </p>
        <p className="mt-1 text-sm text-zinc-600 dark:text-zinc-400">
          Trace every model back through its training data to the source
          documents.{" "}
          <Link
            href={`/projects/${project.id}/lineage`}
            className="font-medium text-violet-600 underline-offset-2 hover:underline dark:text-violet-400"
          >
            View lineage
          </Link>
        </p>
      </div>

      <div className="rounded-lg border border-zinc-200 p-6 dark:border-zinc-800">
        <h3 className="mb-4 text-sm font-medium text-zinc-600 dark:text-zinc-400">
          Danger Zone
        </h3>
        <div className="flex flex-col justify-between gap-3 sm:flex-row sm:items-center">
          <div>
            <p className="text-sm text-zinc-900 dark:text-white">
              Delete this project
            </p>
            <p className="text-xs text-zinc-400 dark:text-zinc-600">
              Stops any running job, takes deployed models offline, and
              permanently erases every document, dataset, adapter and export.
              This cannot be undone.
            </p>
          </div>
          <button
            onClick={handleDelete}
            disabled={deleteProject.isPending}
            className={`rounded-lg px-4 py-2 text-sm font-medium transition ${
              confirmDelete
                ? "bg-red-600 text-white hover:bg-red-500"
                : "border border-red-300 text-red-600 hover:bg-red-50 dark:border-red-800 dark:text-red-400 dark:hover:bg-red-900/30"
            } disabled:opacity-50`}
          >
            {deleteProject.isPending
              ? "Deleting..."
              : confirmDelete
                ? "Confirm — erase everything"
                : "Delete Project"}
          </button>
        </div>
        {deleteProject.isError && (
          <p className="mt-3 text-sm text-red-600 dark:text-red-400">
            {deleteProject.error.message}
          </p>
        )}
      </div>
    </div>
  );
}
