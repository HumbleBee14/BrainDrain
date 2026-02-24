"use client";

import { useState } from "react";
import { useRouter } from "next/navigation";
import Link from "next/link";
import { toast } from "sonner";
import { useCreateProject } from "@/hooks/use-projects";
import { useFormValidation } from "@/hooks/use-form-validation";
import { useOnboarding } from "@/hooks/use-onboarding";
import { Breadcrumbs } from "@/components/breadcrumbs";
import {
  createProjectSchema,
  type CreateProjectInput,
} from "@/lib/validations";

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
  const { markStepComplete } = useOnboarding();
  const { errors, validate, clearFieldError } =
    useFormValidation<CreateProjectInput>(createProjectSchema);

  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [taskType, setTaskType] = useState("");

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    const data = validate({
      name,
      description,
      task_type: taskType || undefined,
    });
    if (!data) return;

    try {
      await createProject.mutateAsync(data);
      markStepComplete("create_project");
      toast.success("Project created");
      router.push("/projects");
    } catch (err) {
      toast.error(
        err instanceof Error ? err.message : "Failed to create project",
      );
    }
  };

  return (
    <div className="max-w-xl">
      <div className="mb-8">
        <Breadcrumbs
          items={[
            { label: "Projects", href: "/projects" },
            { label: "New Project" },
          ]}
        />
        <h1 className="text-2xl font-bold text-zinc-900 dark:text-white">Create Project</h1>
        <p className="text-zinc-500 mt-1">
          A project groups your documents, datasets, and fine-tuned models.
        </p>
      </div>

      <form onSubmit={handleSubmit} className="flex flex-col gap-6">
        <div>
          <label
            htmlFor="name"
            className="block text-sm font-medium text-zinc-700 dark:text-zinc-300 mb-1"
          >
            Project Name
          </label>
          <input
            id="name"
            type="text"
            required
            value={name}
            onChange={(e) => setName(e.target.value)}
            onBlur={() => clearFieldError("name")}
            placeholder="e.g. Customer Support Bot"
            className="w-full rounded-lg border border-zinc-300 dark:border-zinc-700 bg-zinc-50 dark:bg-zinc-900 px-4 py-2 text-zinc-900 dark:text-white placeholder:text-zinc-400 dark:placeholder:text-zinc-600 focus:border-zinc-500 focus:outline-none focus:ring-1 focus:ring-zinc-500"
          />
          {errors.name && (
            <p className="text-sm text-red-400 mt-1">{errors.name}</p>
          )}
        </div>

        <div>
          <label
            htmlFor="description"
            className="block text-sm font-medium text-zinc-700 dark:text-zinc-300 mb-1"
          >
            Description
            <span className="text-zinc-400 dark:text-zinc-600 font-normal ml-1">(optional)</span>
          </label>
          <textarea
            id="description"
            rows={3}
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            onBlur={() => clearFieldError("description")}
            placeholder="What is this model for?"
            className="w-full rounded-lg border border-zinc-300 dark:border-zinc-700 bg-zinc-50 dark:bg-zinc-900 px-4 py-2 text-zinc-900 dark:text-white placeholder:text-zinc-400 dark:placeholder:text-zinc-600 focus:border-zinc-500 focus:outline-none focus:ring-1 focus:ring-zinc-500 resize-none"
          />
          {errors.description && (
            <p className="text-sm text-red-400 mt-1">{errors.description}</p>
          )}
        </div>

        <div>
          <label
            htmlFor="task_type"
            className="block text-sm font-medium text-zinc-700 dark:text-zinc-300 mb-1"
          >
            Task Type
            <span className="text-zinc-400 dark:text-zinc-600 font-normal ml-1">(optional)</span>
          </label>
          <select
            id="task_type"
            value={taskType}
            onChange={(e) => setTaskType(e.target.value)}
            className="w-full rounded-lg border border-zinc-300 dark:border-zinc-700 bg-zinc-50 dark:bg-zinc-900 px-4 py-2 text-zinc-900 dark:text-white focus:border-zinc-500 focus:outline-none focus:ring-1 focus:ring-zinc-500"
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
            disabled={createProject.isPending}
            className="rounded-lg bg-zinc-900 text-white hover:bg-zinc-800 dark:bg-white px-6 py-2 text-sm font-semibold dark:text-zinc-950 dark:hover:bg-zinc-200 transition disabled:opacity-50 disabled:cursor-not-allowed"
          >
            {createProject.isPending ? "Creating..." : "Create Project"}
          </button>
          <Link
            href="/projects"
            className="text-sm text-zinc-500 hover:text-zinc-700 dark:hover:text-zinc-300 transition"
          >
            Cancel
          </Link>
        </div>

        {createProject.isError && (
          <p className="text-sm text-red-400">{createProject.error.message}</p>
        )}
      </form>
    </div>
  );
}
