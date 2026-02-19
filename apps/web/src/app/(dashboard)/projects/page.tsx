"use client";

import Link from "next/link";
import { useProjects } from "@/hooks/use-projects";

export default function ProjectsPage() {
  const { data, isLoading } = useProjects();
  const projects = data?.data ?? [];

  return (
    <div>
      <div className="flex items-center justify-between mb-8">
        <h1 className="text-2xl font-bold text-white">Projects</h1>
        <Link
          href="/projects/new"
          className="rounded-lg bg-white px-4 py-2 text-sm font-semibold text-zinc-950 hover:bg-zinc-200 transition"
        >
          New Project
        </Link>
      </div>

      {isLoading ? (
        <div className="py-12 text-center">
          <p className="text-zinc-500">Loading projects...</p>
        </div>
      ) : projects.length === 0 ? (
        <div className="rounded-lg border border-dashed border-zinc-700 p-12 text-center">
          <p className="text-zinc-400 mb-4">No projects yet</p>
          <Link
            href="/projects/new"
            className="text-sm text-white underline hover:no-underline"
          >
            Create your first project
          </Link>
        </div>
      ) : (
        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
          {projects.map((project) => (
            <Link
              key={project.id}
              href={`/projects/${project.id}`}
              className="rounded-lg border border-zinc-800 p-5 hover:border-zinc-600 transition group"
            >
              <div className="flex items-center justify-between mb-2">
                <h3 className="font-medium text-white group-hover:text-zinc-100 truncate">
                  {project.name}
                </h3>
                <span className="text-xs text-zinc-600 ml-2 shrink-0">
                  {project.status}
                </span>
              </div>
              {project.description && (
                <p className="text-sm text-zinc-500 line-clamp-2 mb-3">
                  {project.description}
                </p>
              )}
              <div className="flex items-center gap-3 text-xs text-zinc-600">
                {project.task_type && <span>{project.task_type}</span>}
                <span>{new Date(project.created_at).toLocaleDateString()}</span>
              </div>
            </Link>
          ))}
        </div>
      )}
    </div>
  );
}
