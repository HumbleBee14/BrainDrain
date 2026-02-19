"use client";

import { useState } from "react";
import { useRouter } from "next/navigation";
import Link from "next/link";
import { useCreateProject } from "@/hooks/use-projects";

const TASK_TYPES = [
  { value: "chat", label: "Chat / Conversational" },
  { value: "instruct", label: "Instruction Following" },
  { value: "classify", label: "Classification" },
  { value: "extract", label: "Extraction / NER" },
  { value: "summarize", label: "Summarization" },
  { value: "code", label: "Code Generation" },
  { value: "custom", label: "Custom" },
];

export default function NewProjectPage() {
  const router = useRouter();
  const createProject = useCreateProject();

  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [taskType, setTaskType] = useState("");

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!name.trim()) return;

    await createProject.mutateAsync({
      name: name.trim(),
      description: description.trim() || undefined,
      task_type: taskType || undefined,
    });

    router.push("/projects");
  };

  return (
    <div className="max-w-xl">
      <div className="mb-8">
        <Link
          href="/projects"
          className="text-sm text-zinc-500 hover:text-zinc-300 transition"
        >
          &larr; Back to Projects
        </Link>
        <h1 className="text-2xl font-bold text-white mt-2">Create Project</h1>
        <p className="text-zinc-500 mt-1">
          A project groups your documents, datasets, and fine-tuned models.
        </p>
      </div>

      <form onSubmit={handleSubmit} className="flex flex-col gap-6">
        <div>
          <label htmlFor="name" className="block text-sm font-medium text-zinc-300 mb-1">
            Project Name
          </label>
          <input
            id="name"
            type="text"
            required
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="e.g. Customer Support Bot"
            className="w-full rounded-lg border border-zinc-700 bg-zinc-900 px-4 py-2 text-white placeholder:text-zinc-600 focus:border-zinc-500 focus:outline-none focus:ring-1 focus:ring-zinc-500"
          />
        </div>

        <div>
          <label htmlFor="description" className="block text-sm font-medium text-zinc-300 mb-1">
            Description
            <span className="text-zinc-600 font-normal ml-1">(optional)</span>
          </label>
          <textarea
            id="description"
            rows={3}
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            placeholder="What is this model for?"
            className="w-full rounded-lg border border-zinc-700 bg-zinc-900 px-4 py-2 text-white placeholder:text-zinc-600 focus:border-zinc-500 focus:outline-none focus:ring-1 focus:ring-zinc-500 resize-none"
          />
        </div>

        <div>
          <label htmlFor="task_type" className="block text-sm font-medium text-zinc-300 mb-1">
            Task Type
            <span className="text-zinc-600 font-normal ml-1">(optional)</span>
          </label>
          <select
            id="task_type"
            value={taskType}
            onChange={(e) => setTaskType(e.target.value)}
            className="w-full rounded-lg border border-zinc-700 bg-zinc-900 px-4 py-2 text-white focus:border-zinc-500 focus:outline-none focus:ring-1 focus:ring-zinc-500"
          >
            <option value="">Select a task type</option>
            {TASK_TYPES.map((t) => (
              <option key={t.value} value={t.value}>
                {t.label}
              </option>
            ))}
          </select>
        </div>

        <div className="flex items-center gap-3 pt-2">
          <button
            type="submit"
            disabled={!name.trim() || createProject.isPending}
            className="rounded-lg bg-white px-6 py-2 text-sm font-semibold text-zinc-950 hover:bg-zinc-200 transition disabled:opacity-50 disabled:cursor-not-allowed"
          >
            {createProject.isPending ? "Creating..." : "Create Project"}
          </button>
          <Link
            href="/projects"
            className="text-sm text-zinc-500 hover:text-zinc-300 transition"
          >
            Cancel
          </Link>
        </div>

        {createProject.isError && (
          <p className="text-sm text-red-400">
            {createProject.error.message}
          </p>
        )}
      </form>
    </div>
  );
}
