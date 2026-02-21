"use client";

import { useParams } from "next/navigation";
import Link from "next/link";
import { useEffect, useState } from "react";
import { useModel } from "@/hooks/use-models";
import { useOnboarding } from "@/hooks/use-onboarding";
import { useDeploymentStatus, useDeployModel, useUndeployModel } from "@/hooks/use-deployments";
import { useApiKeys, useCreateApiKey, useRevokeApiKey } from "@/hooks/use-api-keys";
import { useEvaluations } from "@/hooks/use-evaluations";

function DeploymentBadge({ status }: { status: string }) {
  const colors: Record<string, string> = {
    undeployed: "bg-zinc-800 text-zinc-400 border-zinc-700",
    deploying: "bg-blue-900/50 text-blue-400 border-blue-800 animate-pulse",
    active: "bg-emerald-900/50 text-emerald-400 border-emerald-800",
    inactive: "bg-amber-900/50 text-amber-400 border-amber-800",
  };

  const cls = colors[status] || "bg-zinc-800 text-zinc-400 border-zinc-700";

  return (
    <span className={`inline-flex items-center rounded-full border px-2.5 py-0.5 text-xs font-medium ${cls}`}>
      {status}
    </span>
  );
}

function EvalStatusBadge({ status }: { status: string }) {
  const colors: Record<string, string> = {
    running: "bg-blue-900/50 text-blue-400 border-blue-800 animate-pulse",
    completed: "bg-emerald-900/50 text-emerald-400 border-emerald-800",
    failed: "bg-red-900/50 text-red-400 border-red-800",
  };

  const cls = colors[status] || "bg-zinc-800 text-zinc-400 border-zinc-700";

  return (
    <span className={`inline-flex items-center rounded-full border px-2 py-0.5 text-xs font-medium ${cls}`}>
      {status}
    </span>
  );
}

export default function ModelDetailPage() {
  const params = useParams<{ id: string; modelId: string }>();
  const { data: model, isLoading, error } = useModel(params.modelId);
  const { data: deployment } = useDeploymentStatus(params.modelId);
  const { data: apiKeys } = useApiKeys(params.modelId);
  const { data: evalsData } = useEvaluations(params.modelId);
  const deployModel = useDeployModel(params.modelId);
  const undeployModel = useUndeployModel(params.modelId);
  const createApiKey = useCreateApiKey(params.modelId);
  const revokeApiKey = useRevokeApiKey(params.modelId);

  const { markStepComplete } = useOnboarding();

  // Mark "view_results" onboarding step when viewing a model
  useEffect(() => {
    if (model) markStepComplete("view_results");
  }, [model, markStepComplete]);

  const [showKeyForm, setShowKeyForm] = useState(false);
  const [keyName, setKeyName] = useState("");
  const [createdKey, setCreatedKey] = useState<string | null>(null);
  const [copiedKey, setCopiedKey] = useState(false);

  const evaluations = evalsData?.data ?? [];
  const keys = apiKeys ?? [];
  const isActive = deployment?.deployment_status === "active";
  const isDeploying = deployment?.deployment_status === "deploying";

  if (isLoading) {
    return (
      <div className="flex items-center justify-center py-20">
        <p className="text-zinc-500">Loading model...</p>
      </div>
    );
  }

  if (error || !model) {
    return (
      <div className="flex flex-col items-center justify-center py-20 gap-4">
        <p className="text-zinc-500">Model not found</p>
        <Link href={`/projects/${params.id}`} className="text-sm text-white underline hover:no-underline">
          Back to Project
        </Link>
      </div>
    );
  }

  const handleCreateKey = async () => {
    if (!keyName.trim()) return;
    const result = await createApiKey.mutateAsync({ name: keyName.trim() });
    setCreatedKey(result.key);
    setKeyName("");
    setShowKeyForm(false);
  };

  const handleCopyKey = async (key: string) => {
    await navigator.clipboard.writeText(key);
    setCopiedKey(true);
    setTimeout(() => setCopiedKey(false), 2000);
  };

  return (
    <div>
      {/* Header */}
      <div className="mb-8">
        <Link
          href={`/projects/${params.id}`}
          className="text-sm text-zinc-500 hover:text-zinc-300 transition"
        >
          &larr; Back to Project
        </Link>
        <div className="flex items-center gap-3 mt-2">
          <h1 className="text-2xl font-bold text-white">{model.name}</h1>
          <DeploymentBadge status={model.deployment_status} />
        </div>
        <p className="text-zinc-500 mt-1">
          v{model.version} &middot; {model.base_model}
        </p>
      </div>

      {/* Model info grid */}
      <div className="grid grid-cols-1 md:grid-cols-3 gap-4 mb-8">
        <div className="rounded-lg border border-zinc-800 p-4">
          <p className="text-xs text-zinc-500 uppercase tracking-wider">Base Model</p>
          <p className="text-white mt-1 text-sm">{model.base_model.split("/").pop()}</p>
        </div>
        <div className="rounded-lg border border-zinc-800 p-4">
          <p className="text-xs text-zinc-500 uppercase tracking-wider">Version</p>
          <p className="text-white mt-1 text-sm">v{model.version}</p>
        </div>
        <div className="rounded-lg border border-zinc-800 p-4">
          <p className="text-xs text-zinc-500 uppercase tracking-wider">Created</p>
          <p className="text-white mt-1 text-sm">{new Date(model.created_at).toLocaleDateString()}</p>
        </div>
      </div>

      {/* Eval scores summary (if available) */}
      {model.eval_scores && typeof model.eval_scores === "object" && Object.keys(model.eval_scores).length > 0 && (
        <div className="mb-8">
          <div className="flex items-center justify-between mb-4">
            <h2 className="text-lg font-semibold text-white">Evaluation Scores</h2>
            <Link
              href={`/projects/${params.id}/models/${params.modelId}/evaluation`}
              className="text-sm text-blue-400 hover:text-blue-300 transition"
            >
              View Details &rarr;
            </Link>
          </div>
          <div className="grid grid-cols-2 md:grid-cols-5 gap-3">
            {typeof model.eval_scores.overall === "number" && (
              <div className="rounded-lg border border-zinc-800 p-4 text-center">
                <p className="text-2xl font-bold text-white">{model.eval_scores.overall as number}/100</p>
                <p className="text-xs text-zinc-500 mt-1">Overall</p>
              </div>
            )}
          </div>
        </div>
      )}

      {/* Deployment section */}
      <div className="mb-8">
        <h2 className="text-lg font-semibold text-white mb-4">Deployment</h2>
        <div className="rounded-lg border border-zinc-800 p-6">
          <div className="flex items-center justify-between">
            <div>
              <p className="text-sm text-white">
                {isActive
                  ? "Model is actively serving requests"
                  : isDeploying
                    ? "Model is being deployed..."
                    : "Model is not deployed"}
              </p>
              <p className="text-xs text-zinc-600 mt-1">
                {isActive
                  ? "Inference API is available. Create API keys to start using it."
                  : "Deploy the model to make it available for inference."}
              </p>
            </div>
            <div className="flex gap-2">
              {isActive ? (
                <button
                  onClick={() => undeployModel.mutate()}
                  disabled={undeployModel.isPending}
                  className="rounded-lg border border-red-800 px-4 py-2 text-sm text-red-400 hover:bg-red-900/30 transition disabled:opacity-50"
                >
                  {undeployModel.isPending ? "Undeploying..." : "Undeploy"}
                </button>
              ) : (
                <button
                  onClick={() => deployModel.mutate()}
                  disabled={deployModel.isPending || isDeploying}
                  className="rounded-lg bg-emerald-600 px-4 py-2 text-sm font-medium text-white hover:bg-emerald-500 transition disabled:opacity-50"
                >
                  {deployModel.isPending || isDeploying ? "Deploying..." : "Deploy Model"}
                </button>
              )}
            </div>
          </div>
          {deployModel.isError && (
            <p className="text-sm text-red-400 mt-3">{deployModel.error.message}</p>
          )}
          {undeployModel.isError && (
            <p className="text-sm text-red-400 mt-3">{undeployModel.error.message}</p>
          )}
        </div>
      </div>

      {/* API Keys section */}
      <div className="mb-8">
        <div className="flex items-center justify-between mb-4">
          <h2 className="text-lg font-semibold text-white">API Keys</h2>
          <button
            onClick={() => setShowKeyForm(!showKeyForm)}
            className="rounded-lg bg-zinc-800 px-4 py-2 text-sm text-white hover:bg-zinc-700 transition"
          >
            Create Key
          </button>
        </div>

        {/* Created key display (only shown once) */}
        {createdKey && (
          <div className="rounded-lg border border-emerald-800 bg-emerald-900/20 p-4 mb-4">
            <p className="text-sm text-emerald-400 mb-2">
              API key created. Copy it now — it won&apos;t be shown again.
            </p>
            <div className="flex items-center gap-2">
              <code className="flex-1 rounded bg-zinc-900 px-3 py-2 text-sm text-white font-mono break-all">
                {createdKey}
              </code>
              <button
                onClick={() => handleCopyKey(createdKey)}
                className="rounded-lg bg-emerald-600 px-3 py-2 text-sm text-white hover:bg-emerald-500 transition shrink-0"
              >
                {copiedKey ? "Copied!" : "Copy"}
              </button>
            </div>
            <button
              onClick={() => setCreatedKey(null)}
              className="text-xs text-zinc-500 mt-2 hover:text-zinc-400 transition"
            >
              Dismiss
            </button>
          </div>
        )}

        {/* Create key form */}
        {showKeyForm && (
          <div className="rounded-lg border border-zinc-800 p-4 mb-4">
            <div className="flex gap-2">
              <input
                value={keyName}
                onChange={(e) => setKeyName(e.target.value)}
                placeholder="Key name (e.g., production, testing)"
                className="flex-1 rounded-lg border border-zinc-700 bg-zinc-900 px-3 py-2 text-sm text-white"
              />
              <button
                onClick={handleCreateKey}
                disabled={!keyName.trim() || createApiKey.isPending}
                className="rounded-lg bg-blue-600 px-4 py-2 text-sm text-white hover:bg-blue-500 transition disabled:opacity-50"
              >
                {createApiKey.isPending ? "Creating..." : "Create"}
              </button>
              <button
                onClick={() => setShowKeyForm(false)}
                className="rounded-lg border border-zinc-700 px-4 py-2 text-sm text-zinc-400 hover:border-zinc-600 transition"
              >
                Cancel
              </button>
            </div>
            {createApiKey.isError && (
              <p className="text-sm text-red-400 mt-2">{createApiKey.error.message}</p>
            )}
          </div>
        )}

        {/* Keys list */}
        {keys.length > 0 ? (
          <div className="rounded-lg border border-zinc-800">
            {keys.map((k) => (
              <div
                key={k.id}
                className="flex items-center justify-between py-3 px-4 border-b border-zinc-800 last:border-b-0"
              >
                <div>
                  <p className="text-sm text-white">{k.name}</p>
                  <p className="text-xs text-zinc-600">
                    <code className="text-zinc-500">{k.key_prefix}...</code>
                    {" \u00b7 "}
                    {k.rate_limit} req/min
                    {k.last_used_at && ` \u00b7 Last used ${new Date(k.last_used_at).toLocaleDateString()}`}
                    {k.expires_at && ` \u00b7 Expires ${new Date(k.expires_at).toLocaleDateString()}`}
                  </p>
                </div>
                <div className="flex items-center gap-2">
                  {k.is_active ? (
                    <button
                      onClick={() => revokeApiKey.mutate(k.id)}
                      disabled={revokeApiKey.isPending}
                      className="text-xs text-red-400 hover:text-red-300 transition"
                    >
                      Revoke
                    </button>
                  ) : (
                    <span className="text-xs text-zinc-600">Revoked</span>
                  )}
                </div>
              </div>
            ))}
          </div>
        ) : (
          <p className="text-sm text-zinc-600">No API keys yet.</p>
        )}
      </div>

      {/* Evaluations section */}
      <div className="mb-8">
        <div className="flex items-center justify-between mb-4">
          <h2 className="text-lg font-semibold text-white">
            Evaluations {evaluations.length > 0 && `(${evalsData?.total ?? evaluations.length})`}
          </h2>
          <Link
            href={`/projects/${params.id}/models/${params.modelId}/evaluation`}
            className="rounded-lg bg-violet-600 px-4 py-2 text-sm font-medium text-white hover:bg-violet-500 transition"
          >
            Evaluate Model
          </Link>
        </div>

        {evaluations.length > 0 ? (
          <div className="rounded-lg border border-zinc-800">
            {evaluations.map((ev) => (
              <Link
                key={ev.id}
                href={`/projects/${params.id}/models/${params.modelId}/evaluation`}
                className="flex items-center justify-between py-3 px-4 border-b border-zinc-800 last:border-b-0 hover:bg-zinc-900/50 transition"
              >
                <div>
                  <p className="text-sm text-white">
                    Evaluation {new Date(ev.created_at).toLocaleString()}
                  </p>
                  <p className="text-xs text-zinc-600">
                    {ev.scores?.overall != null && `Score: ${ev.scores.overall}/100 \u00b7 `}
                    {ev.completed_at ? `Completed ${new Date(ev.completed_at).toLocaleDateString()}` : "In progress..."}
                  </p>
                </div>
                <EvalStatusBadge status={ev.status} />
              </Link>
            ))}
          </div>
        ) : (
          <p className="text-sm text-zinc-600">No evaluations yet. Run one to measure model quality.</p>
        )}
      </div>

      {/* Quick links */}
      {isActive && (
        <div className="rounded-lg border border-zinc-800 p-6">
          <h3 className="text-sm font-medium text-zinc-400 mb-4">Quick Links</h3>
          <div className="flex gap-3">
            <Link
              href={`/projects/${params.id}/models/${params.modelId}/playground`}
              className="rounded-lg bg-blue-600 px-4 py-2 text-sm font-medium text-white hover:bg-blue-500 transition"
            >
              Open Playground
            </Link>
            <Link
              href={`/projects/${params.id}/models/${params.modelId}/evaluation`}
              className="rounded-lg border border-zinc-700 px-4 py-2 text-sm text-zinc-400 hover:border-zinc-600 transition"
            >
              View Evaluation
            </Link>
          </div>
        </div>
      )}
    </div>
  );
}
