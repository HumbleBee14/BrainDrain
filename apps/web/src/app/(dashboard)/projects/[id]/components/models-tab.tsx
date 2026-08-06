"use client";

import Link from "next/link";
import type { Model } from "@/lib/api-client";
import { DeploymentStatusBadge } from "./deployment-status-badge";

export function ModelsTab({
  projectId,
  models,
}: {
  projectId: string;
  models: Model[];
}) {
  const activeCount = models.filter(
    (m) => m.deployment_status === "active",
  ).length;

  return (
    <div>
      {activeCount >= 1 && (
        <div className="mb-4 flex justify-end">
          <Link
            href={`/projects/${projectId}/playground`}
            className="rounded-lg border border-zinc-200 bg-white px-4 py-2 text-sm font-medium text-zinc-700 transition hover:bg-zinc-50 dark:border-zinc-800 dark:bg-zinc-900 dark:text-zinc-200 dark:hover:bg-zinc-800"
          >
            A/B Playground
          </Link>
        </div>
      )}
      {models.length > 0 ? (
        <div className="rounded-lg border border-zinc-200 dark:border-zinc-800">
          {models.map((model) => (
            <Link
              key={model.id}
              href={`/projects/${projectId}/models/${model.id}`}
              className="flex items-center justify-between border-b border-zinc-200 px-4 py-3 transition last:border-b-0 hover:bg-zinc-50/50 dark:border-zinc-800 dark:hover:bg-zinc-900/50"
            >
              <div>
                <p className="text-sm text-zinc-900 dark:text-white">
                  {model.name}
                </p>
                <p className="text-xs text-zinc-400 dark:text-zinc-600">
                  v{model.version} &middot; {model.base_model.split("/").pop()}
                </p>
              </div>
              <DeploymentStatusBadge status={model.deployment_status} />
            </Link>
          ))}
        </div>
      ) : (
        <p className="text-sm text-zinc-400 dark:text-zinc-600">
          No models in this project yet — each completed fine-tuning run here
          produces one. Models trained in another project stay with that
          project.
        </p>
      )}
    </div>
  );
}
