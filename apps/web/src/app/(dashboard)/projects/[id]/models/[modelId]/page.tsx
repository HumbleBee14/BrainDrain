"use client";

import { useParams } from "next/navigation";
import Link from "next/link";
import { useEffect, useState } from "react";
import { toast } from "sonner";
import { useModel, useModelVersions, useRollbackModel } from "@/hooks/use-models";
import { useOnboarding } from "@/hooks/use-onboarding";
import {
  useDeploymentStatus,
  useDeployModel,
  useUndeployModel,
} from "@/hooks/use-deployments";
import {
  useApiKeys,
  useCreateApiKey,
  useRevokeApiKey,
} from "@/hooks/use-api-keys";
import { useEvaluations } from "@/hooks/use-evaluations";
import {
  useModelExports,
  useCreateExport,
  useExportDownload,
  useOllamaRecipe,
} from "@/hooks/use-exports";
import { Breadcrumbs } from "@/components/breadcrumbs";
import { AdapterDownloadButton } from "@/components/adapter-download-button";

const API_URL = process.env.NEXT_PUBLIC_API_URL || "http://localhost:8000";

function DeploymentBadge({ status }: { status: string }) {
  const colors: Record<string, string> = {
    undeployed:
      "bg-zinc-100 text-zinc-600 border-zinc-300 dark:bg-zinc-800 dark:text-zinc-400 dark:border-zinc-700",
    deploying:
      "bg-blue-50 text-blue-700 border-blue-200 dark:bg-blue-900/50 dark:text-blue-400 dark:border-blue-800 animate-pulse",
    active:
      "bg-emerald-50 text-emerald-700 border-emerald-200 dark:bg-emerald-900/50 dark:text-emerald-400 dark:border-emerald-800",
    inactive:
      "bg-amber-50 text-amber-700 border-amber-200 dark:bg-amber-900/50 dark:text-amber-400 dark:border-amber-800",
  };

  const cls =
    colors[status] ||
    "bg-zinc-100 text-zinc-600 border-zinc-300 dark:bg-zinc-800 dark:text-zinc-400 dark:border-zinc-700";

  return (
    <span
      className={`inline-flex items-center rounded-full border px-2.5 py-0.5 text-xs font-medium ${cls}`}
    >
      {status}
    </span>
  );
}

function ExportStatusBadge({ status }: { status: string }) {
  const colors: Record<string, string> = {
    pending:
      "bg-zinc-100 text-zinc-600 border-zinc-300 dark:bg-zinc-800 dark:text-zinc-400 dark:border-zinc-700",
    processing:
      "bg-blue-50 text-blue-700 border-blue-200 dark:bg-blue-900/50 dark:text-blue-400 dark:border-blue-800 animate-pulse",
    completed:
      "bg-emerald-50 text-emerald-700 border-emerald-200 dark:bg-emerald-900/50 dark:text-emerald-400 dark:border-emerald-800",
    failed:
      "bg-red-50 text-red-700 border-red-200 dark:bg-red-900/50 dark:text-red-400 dark:border-red-800",
  };

  const cls =
    colors[status] ||
    "bg-zinc-100 text-zinc-600 border-zinc-300 dark:bg-zinc-800 dark:text-zinc-400 dark:border-zinc-700";

  return (
    <span
      className={`inline-flex items-center rounded-full border px-2 py-0.5 text-xs font-medium ${cls}`}
    >
      {status}
    </span>
  );
}

function EvalStatusBadge({ status }: { status: string }) {
  const colors: Record<string, string> = {
    running:
      "bg-blue-50 text-blue-700 border-blue-200 dark:bg-blue-900/50 dark:text-blue-400 dark:border-blue-800 animate-pulse",
    completed:
      "bg-emerald-50 text-emerald-700 border-emerald-200 dark:bg-emerald-900/50 dark:text-emerald-400 dark:border-emerald-800",
    failed:
      "bg-red-50 text-red-700 border-red-200 dark:bg-red-900/50 dark:text-red-400 dark:border-red-800",
  };

  const cls =
    colors[status] ||
    "bg-zinc-100 text-zinc-600 border-zinc-300 dark:bg-zinc-800 dark:text-zinc-400 dark:border-zinc-700";

  return (
    <span
      className={`inline-flex items-center rounded-full border px-2 py-0.5 text-xs font-medium ${cls}`}
    >
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
  const { data: exports } = useModelExports(params.modelId);
  const { data: versions } = useModelVersions(params.modelId);
  const createExport = useCreateExport(params.modelId);
  const downloadExport = useExportDownload();
  const ollamaRecipe = useOllamaRecipe();
  const deployModel = useDeployModel(params.modelId);
  const undeployModel = useUndeployModel(params.modelId);
  const createApiKey = useCreateApiKey(params.modelId);
  const revokeApiKey = useRevokeApiKey(params.modelId);
  const rollbackModel = useRollbackModel(params.modelId);

  const { markStepComplete } = useOnboarding();

  // Mark "view_results" onboarding step when viewing a model
  useEffect(() => {
    if (model) markStepComplete("view_results");
  }, [model, markStepComplete]);

  // Toast notifications for mutations
  useEffect(() => {
    if (deployModel.isSuccess) toast.success("Model deployment started");
  }, [deployModel.isSuccess]);
  useEffect(() => {
    if (deployModel.isError) toast.error(deployModel.error.message);
  }, [deployModel.isError, deployModel.error]);

  useEffect(() => {
    if (undeployModel.isSuccess) toast.success("Model undeployed");
  }, [undeployModel.isSuccess]);
  useEffect(() => {
    if (undeployModel.isError) toast.error(undeployModel.error.message);
  }, [undeployModel.isError, undeployModel.error]);

  useEffect(() => {
    if (revokeApiKey.isSuccess) toast.success("API key revoked");
  }, [revokeApiKey.isSuccess]);
  useEffect(() => {
    if (revokeApiKey.isError) toast.error(revokeApiKey.error.message);
  }, [revokeApiKey.isError, revokeApiKey.error]);

  useEffect(() => {
    if (createExport.isSuccess) toast.success("GGUF export started");
  }, [createExport.isSuccess]);
  useEffect(() => {
    if (createExport.isError) toast.error(createExport.error.message);
  }, [createExport.isError, createExport.error]);

  useEffect(() => {
    if (rollbackModel.isSuccess) toast.success("Model rolled back successfully");
  }, [rollbackModel.isSuccess]);
  useEffect(() => {
    if (rollbackModel.isError) toast.error(rollbackModel.error.message);
  }, [rollbackModel.isError, rollbackModel.error]);

  const [showKeyForm, setShowKeyForm] = useState(false);
  const [keyName, setKeyName] = useState("");
  const [createdKey, setCreatedKey] = useState<string | null>(null);
  const [copiedKey, setCopiedKey] = useState(false);
  const [copiedSnippet, setCopiedSnippet] = useState<string | null>(null);
  const [exportQuantType, setExportQuantType] = useState("Q5_K_M");
  const [downloadError, setDownloadError] = useState<string | null>(null);
  const [ollamaForExport, setOllamaForExport] = useState<string | null>(null);
  const [copiedModelfile, setCopiedModelfile] = useState(false);

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
        <Link
          href={`/projects/${params.id}`}
          className="text-sm text-zinc-900 dark:text-white underline hover:no-underline"
        >
          Back to Project
        </Link>
      </div>
    );
  }

  const handleCreateKey = async () => {
    if (!keyName.trim()) return;
    try {
      const result = await createApiKey.mutateAsync({ name: keyName.trim() });
      setCreatedKey(result.key);
      setKeyName("");
      setShowKeyForm(false);
    } catch {
      // Error is captured by React Query and surfaced via createApiKey.isError
    }
  };

  const handleCopyKey = async (key: string) => {
    await navigator.clipboard.writeText(key);
    setCopiedKey(true);
    setTimeout(() => setCopiedKey(false), 2000);
  };

  const handleCopySnippet = async (id: string, text: string) => {
    await navigator.clipboard.writeText(text);
    setCopiedSnippet(id);
    setTimeout(() => setCopiedSnippet((cur) => (cur === id ? null : cur)), 2000);
  };

  const endpointUrl = `${API_URL}/v1/chat/completions`;
  const curlSnippet = `curl ${endpointUrl} \\
  -H "Authorization: Bearer $API_KEY" \\
  -H "Content-Type: application/json" \\
  -d '{
    "model": "${model.name}",
    "messages": [{"role": "user", "content": "Hello!"}]
  }'`;

  return (
    <div>
      {/* Header */}
      <div className="mb-8">
        <Breadcrumbs
          items={[
            { label: "Projects", href: "/projects" },
            { label: "Project", href: `/projects/${params.id}` },
            { label: model.name },
          ]}
        />
        <div className="flex items-center gap-3">
          <h1 className="text-xl md:text-2xl font-bold text-zinc-900 dark:text-white truncate">{model.name}</h1>
          <DeploymentBadge status={model.deployment_status} />
        </div>
        <p className="text-zinc-500 mt-1">
          v{model.version} &middot; {model.base_model}
        </p>
        <div className="mt-4">
          <AdapterDownloadButton
            modelId={params.modelId as string}
            sizeBytes={model.adapter_size_bytes}
            available={model.has_adapter}
          />
        </div>
      </div>

      {/* Model info grid */}
      <div className="grid grid-cols-1 sm:grid-cols-3 gap-3 md:gap-4 mb-6 md:mb-8">
        <div className="rounded-lg border border-zinc-200 dark:border-zinc-800 p-4">
          <p className="text-xs text-zinc-500 uppercase tracking-wider">
            Base Model
          </p>
          <p className="text-zinc-900 dark:text-white mt-1 text-sm">
            {model.base_model.split("/").pop()}
          </p>
        </div>
        <div className="rounded-lg border border-zinc-200 dark:border-zinc-800 p-4">
          <p className="text-xs text-zinc-500 uppercase tracking-wider">
            Version
          </p>
          <p className="text-zinc-900 dark:text-white mt-1 text-sm">v{model.version}</p>
        </div>
        <div className="rounded-lg border border-zinc-200 dark:border-zinc-800 p-4">
          <p className="text-xs text-zinc-500 uppercase tracking-wider">
            Created
          </p>
          <p className="text-zinc-900 dark:text-white mt-1 text-sm">
            {new Date(model.created_at).toLocaleDateString()}
          </p>
        </div>
      </div>

      {/* Version History */}
      {versions && versions.length > 1 && (
        <div className="mb-8">
          <h2 className="text-lg font-semibold text-zinc-900 dark:text-white mb-4">
            Version History ({versions.length} versions)
          </h2>
          <div className="rounded-lg border border-zinc-200 dark:border-zinc-800">
            {versions.map((v) => {
              const isCurrent = v.id === model.id;
              const isActive = v.deployment_status === "active";
              return (
                <div
                  key={v.id}
                  className={`flex items-center justify-between py-3 px-4 border-b border-zinc-200 dark:border-zinc-800 last:border-b-0 ${
                    isCurrent ? "bg-zinc-50/50 dark:bg-zinc-900/50" : ""
                  }`}
                >
                  <div>
                    <div className="flex items-center gap-2">
                      <p className="text-sm text-zinc-900 dark:text-white font-medium">
                        v{v.version}
                      </p>
                      {isCurrent && (
                        <span className="text-xs text-zinc-500 bg-zinc-100 dark:bg-zinc-800 rounded px-1.5 py-0.5">
                          current
                        </span>
                      )}
                      {isActive && (
                        <span className="text-xs text-emerald-600 dark:text-emerald-400 bg-emerald-50 dark:bg-emerald-900/50 border border-emerald-200 dark:border-emerald-800 rounded-full px-2 py-0.5">
                          deployed
                        </span>
                      )}
                    </div>
                    <p className="text-xs text-zinc-400 dark:text-zinc-600 mt-0.5">
                      {v.name} &middot;{" "}
                      {new Date(v.created_at).toLocaleDateString()}
                      {v.eval_scores &&
                        typeof v.eval_scores === "object" &&
                        typeof (v.eval_scores as Record<string, unknown>)
                          .overall === "number" &&
                        ` \u00b7 Score: ${(v.eval_scores as Record<string, unknown>).overall}/100`}
                    </p>
                  </div>
                  <div className="flex items-center gap-2">
                    <DeploymentBadge status={v.deployment_status} />
                    {!isCurrent && (
                      <button
                        onClick={() => rollbackModel.mutate(v.id)}
                        disabled={rollbackModel.isPending}
                        className="rounded-lg border border-amber-200 dark:border-amber-800 px-3 py-1.5 text-xs text-amber-600 dark:text-amber-400 hover:bg-amber-50 dark:hover:bg-amber-900/30 transition disabled:opacity-50"
                      >
                        {rollbackModel.isPending
                          ? "Rolling back..."
                          : "Rollback"}
                      </button>
                    )}
                  </div>
                </div>
              );
            })}
          </div>
          {rollbackModel.isError && (
            <p className="text-sm text-red-600 dark:text-red-400 mt-2">
              {rollbackModel.error.message}
            </p>
          )}
        </div>
      )}

      {/* Eval scores summary (if available) */}
      {model.eval_scores &&
        typeof model.eval_scores === "object" &&
        Object.keys(model.eval_scores).length > 0 && (
          <div className="mb-8">
            <div className="flex items-center justify-between mb-4">
              <h2 className="text-lg font-semibold text-zinc-900 dark:text-white">
                Evaluation Scores
              </h2>
              <Link
                href={`/projects/${params.id}/models/${params.modelId}/evaluation`}
                className="text-sm text-blue-400 hover:text-blue-300 transition"
              >
                View Details &rarr;
              </Link>
            </div>
            <div className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-5 gap-2 md:gap-3">
              {typeof model.eval_scores.overall === "number" && (
                <div className="rounded-lg border border-zinc-200 dark:border-zinc-800 p-4 text-center">
                  <p className="text-2xl font-bold text-zinc-900 dark:text-white">
                    {model.eval_scores.overall as number}/100
                  </p>
                  <p className="text-xs text-zinc-500 mt-1">Overall</p>
                </div>
              )}
            </div>
          </div>
        )}

      {/* Deployment section */}
      <div className="mb-8">
        <h2 className="text-lg font-semibold text-zinc-900 dark:text-white mb-4">Deployment</h2>
        <div className="rounded-lg border border-zinc-200 dark:border-zinc-800 p-6">
          <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3">
            <div>
              <p className="text-sm text-zinc-900 dark:text-white">
                {isActive
                  ? "Model is actively serving requests"
                  : isDeploying
                    ? "Model is being deployed..."
                    : "Model is not deployed"}
              </p>
              <p className="text-xs text-zinc-400 dark:text-zinc-600 mt-1">
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
                  className="rounded-lg border border-red-200 dark:border-red-800 px-4 py-2 text-sm text-red-600 dark:text-red-400 hover:bg-red-50 dark:hover:bg-red-900/30 transition disabled:opacity-50"
                >
                  {undeployModel.isPending ? "Undeploying..." : "Undeploy"}
                </button>
              ) : (
                <button
                  onClick={() => deployModel.mutate()}
                  disabled={deployModel.isPending || isDeploying}
                  className="rounded-lg bg-emerald-600 px-4 py-2 text-sm font-medium text-white hover:bg-emerald-500 transition disabled:opacity-50"
                >
                  {deployModel.isPending || isDeploying
                    ? "Deploying..."
                    : "Deploy Model"}
                </button>
              )}
            </div>
          </div>
          {deployModel.isError && (
            <p className="text-sm text-red-600 dark:text-red-400 mt-3">
              {deployModel.error.message}
            </p>
          )}
          {undeployModel.isError && (
            <p className="text-sm text-red-600 dark:text-red-400 mt-3">
              {undeployModel.error.message}
            </p>
          )}
        </div>
      </div>

      {/* API Keys section */}
      <div className="mb-8">
        <div className="flex items-center justify-between mb-4">
          <h2 className="text-lg font-semibold text-zinc-900 dark:text-white">API Keys</h2>
          <button
            onClick={() => setShowKeyForm(!showKeyForm)}
            className="rounded-lg bg-zinc-100 dark:bg-zinc-800 px-4 py-2 text-sm text-zinc-900 dark:text-white hover:bg-zinc-200 dark:hover:bg-zinc-700 transition"
          >
            Create Key
          </button>
        </div>

        {/* Created key display (only shown once) */}
        {createdKey && (
          <div className="rounded-lg border border-emerald-200 dark:border-emerald-800 bg-emerald-50 dark:bg-emerald-900/20 p-4 mb-4">
            <p className="text-sm text-emerald-600 dark:text-emerald-400 mb-2">
              API key created. Copy it now — it won&apos;t be shown again.
            </p>
            <div className="flex items-center gap-2">
              <code className="flex-1 rounded bg-zinc-50 dark:bg-zinc-900 px-3 py-2 text-sm text-zinc-900 dark:text-white font-mono break-all">
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
          <div className="rounded-lg border border-zinc-200 dark:border-zinc-800 p-4 mb-4">
            <div className="flex gap-2">
              <input
                value={keyName}
                onChange={(e) => setKeyName(e.target.value)}
                placeholder="Key name (e.g., production, testing)"
                className="flex-1 rounded-lg border border-zinc-300 dark:border-zinc-700 bg-white dark:bg-zinc-900 px-3 py-2 text-sm text-zinc-900 dark:text-white"
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
                className="rounded-lg border border-zinc-300 dark:border-zinc-700 px-4 py-2 text-sm text-zinc-600 dark:text-zinc-400 hover:border-zinc-400 dark:hover:border-zinc-600 transition"
              >
                Cancel
              </button>
            </div>
            {createApiKey.isError && (
              <p className="text-sm text-red-600 dark:text-red-400 mt-2">
                {createApiKey.error.message}
              </p>
            )}
          </div>
        )}

        {/* Keys list */}
        {keys.length > 0 ? (
          <div className="rounded-lg border border-zinc-200 dark:border-zinc-800">
            {keys.map((k) => (
              <div
                key={k.id}
                className="flex items-center justify-between py-3 px-4 border-b border-zinc-200 dark:border-zinc-800 last:border-b-0"
              >
                <div>
                  <p className="text-sm text-zinc-900 dark:text-white">{k.name}</p>
                  <p className="text-xs text-zinc-400 dark:text-zinc-600">
                    <code className="text-zinc-500">{k.key_prefix}...</code>
                    {" \u00b7 "}
                    {k.rate_limit} req/min
                    {k.last_used_at &&
                      ` \u00b7 Last used ${new Date(k.last_used_at).toLocaleDateString()}`}
                    {k.expires_at &&
                      ` \u00b7 Expires ${new Date(k.expires_at).toLocaleDateString()}`}
                  </p>
                </div>
                <div className="flex items-center gap-2">
                  {k.is_active ? (
                    <button
                      onClick={() => revokeApiKey.mutate(k.id)}
                      disabled={revokeApiKey.isPending}
                      className="text-xs text-red-600 dark:text-red-400 hover:text-red-500 dark:hover:text-red-300 transition"
                    >
                      Revoke
                    </button>
                  ) : (
                    <span className="text-xs text-zinc-400 dark:text-zinc-600">Revoked</span>
                  )}
                </div>
              </div>
            ))}
          </div>
        ) : (
          <p className="text-sm text-zinc-400 dark:text-zinc-600">No API keys yet.</p>
        )}
      </div>

      {/* Inference endpoint */}
      {isActive && (
        <div className="mb-8">
          <h2 className="text-lg font-semibold text-zinc-900 dark:text-white mb-4">
            Inference Endpoint
          </h2>
          <div className="rounded-lg border border-zinc-200 dark:border-zinc-800 p-6 space-y-5">
            <div>
              <p className="text-xs text-zinc-500 uppercase tracking-wider mb-2">
                Endpoint URL
              </p>
              <div className="flex items-center gap-2">
                <code className="flex-1 rounded bg-zinc-50 dark:bg-zinc-900 px-3 py-2 text-sm text-zinc-900 dark:text-white font-mono break-all">
                  {endpointUrl}
                </code>
                <button
                  onClick={() => handleCopySnippet("url", endpointUrl)}
                  className="rounded-lg bg-zinc-100 dark:bg-zinc-800 px-3 py-2 text-sm text-zinc-700 dark:text-zinc-300 hover:bg-zinc-200 dark:hover:bg-zinc-700 transition shrink-0"
                >
                  {copiedSnippet === "url" ? "Copied!" : "Copy"}
                </button>
              </div>
              <p className="text-xs text-zinc-400 dark:text-zinc-600 mt-2">
                OpenAI-compatible Chat Completions API. Authenticate with an API
                key above; the key selects this model, so the{" "}
                <code className="font-mono">model</code> field is optional.
              </p>
            </div>
            <div>
              <div className="flex items-center justify-between mb-2">
                <p className="text-xs text-zinc-500 uppercase tracking-wider">
                  Example request
                </p>
                <button
                  onClick={() => handleCopySnippet("curl", curlSnippet)}
                  className="rounded-lg bg-zinc-100 dark:bg-zinc-800 px-3 py-1.5 text-xs text-zinc-700 dark:text-zinc-300 hover:bg-zinc-200 dark:hover:bg-zinc-700 transition"
                >
                  {copiedSnippet === "curl" ? "Copied!" : "Copy"}
                </button>
              </div>
              <pre className="rounded-lg bg-zinc-50 dark:bg-zinc-900 p-4 text-xs text-zinc-800 dark:text-zinc-200 font-mono overflow-x-auto">
                {curlSnippet}
              </pre>
            </div>
          </div>
        </div>
      )}

      {/* Evaluations section */}
      <div className="mb-8">
        <div className="flex items-center justify-between mb-4">
          <h2 className="text-lg font-semibold text-zinc-900 dark:text-white">
            Evaluations{" "}
            {evaluations.length > 0 &&
              `(${evalsData?.total ?? evaluations.length})`}
          </h2>
          <Link
            href={`/projects/${params.id}/models/${params.modelId}/evaluation`}
            className="rounded-lg bg-violet-600 px-4 py-2 text-sm font-medium text-white hover:bg-violet-500 transition"
          >
            Evaluate Model
          </Link>
        </div>

        {evaluations.length > 0 ? (
          <div className="rounded-lg border border-zinc-200 dark:border-zinc-800">
            {evaluations.map((ev) => (
              <Link
                key={ev.id}
                href={`/projects/${params.id}/models/${params.modelId}/evaluation`}
                className="flex items-center justify-between py-3 px-4 border-b border-zinc-200 dark:border-zinc-800 last:border-b-0 hover:bg-zinc-50/50 dark:hover:bg-zinc-900/50 transition"
              >
                <div>
                  <p className="text-sm text-zinc-900 dark:text-white">
                    Evaluation {new Date(ev.created_at).toLocaleString()}
                  </p>
                  <p className="text-xs text-zinc-400 dark:text-zinc-600">
                    {ev.scores?.overall != null &&
                      `Score: ${ev.scores.overall}/100 \u00b7 `}
                    {ev.completed_at
                      ? `Completed ${new Date(ev.completed_at).toLocaleDateString()}`
                      : "In progress..."}
                  </p>
                </div>
                <EvalStatusBadge status={ev.status} />
              </Link>
            ))}
          </div>
        ) : (
          <p className="text-sm text-zinc-400 dark:text-zinc-600">
            No evaluations yet. Run one to measure model quality.
          </p>
        )}
      </div>

      {/* Production feedback section */}
      <div className="mb-8">
        <div className="flex items-center justify-between mb-4">
          <h2 className="text-lg font-semibold text-zinc-900 dark:text-white">
            Production Feedback
          </h2>
          <Link
            href={`/projects/${params.id}/models/${params.modelId}/feedback`}
            className="text-sm text-blue-400 hover:text-blue-300 transition"
          >
            Review Samples &rarr;
          </Link>
        </div>
        <p className="text-sm text-zinc-400 dark:text-zinc-600">
          {model.capture_traffic
            ? "Traffic capture is on — inference requests are being recorded for review."
            : "Capture live inference traffic and rate responses to improve the next training run."}
        </p>
      </div>

      {/* GGUF Export section */}
      <div className="mb-8">
        <h2 className="text-lg font-semibold text-zinc-900 dark:text-white mb-4">GGUF Export</h2>
        <div className="rounded-lg border border-zinc-200 dark:border-zinc-800 p-6 mb-4">
          <div className="flex flex-col sm:flex-row sm:items-center gap-2 sm:gap-3">
            <select
              value={exportQuantType}
              onChange={(e) => setExportQuantType(e.target.value)}
              className="rounded-lg border border-zinc-300 dark:border-zinc-700 bg-white dark:bg-zinc-900 px-3 py-2 text-sm text-zinc-900 dark:text-white"
            >
              <option value="Q4_K_M">Q4_K_M (smallest)</option>
              <option value="Q5_K_M">Q5_K_M (balanced)</option>
              <option value="Q6_K">Q6_K (high quality)</option>
              <option value="Q8_0">Q8_0 (highest quality)</option>
            </select>
            <button
              onClick={() =>
                createExport.mutate({ quant_type: exportQuantType })
              }
              disabled={createExport.isPending}
              className="rounded-lg bg-orange-600 px-4 py-2 text-sm font-medium text-white hover:bg-orange-500 transition disabled:opacity-50"
            >
              {createExport.isPending ? "Starting..." : "Export GGUF"}
            </button>
          </div>
          <p className="text-xs text-zinc-400 dark:text-zinc-600 mt-2">
            Merge LoRA adapter into base model and export as quantized GGUF for
            local inference (llama.cpp, Ollama, LM Studio).
          </p>
          {createExport.isError && (
            <p className="text-sm text-red-600 dark:text-red-400 mt-2">
              {createExport.error.message}
            </p>
          )}
          {downloadError && (
            <p className="text-sm text-red-600 dark:text-red-400 mt-2">{downloadError}</p>
          )}
        </div>

        {exports && exports.length > 0 && (
          <div className="rounded-lg border border-zinc-200 dark:border-zinc-800">
            {exports.map((exp) => (
              <div
                key={exp.id}
                className="flex items-center justify-between py-3 px-4 border-b border-zinc-200 dark:border-zinc-800 last:border-b-0"
              >
                <div>
                  <p className="text-sm text-zinc-900 dark:text-white">
                    {exp.quant_type} &middot; {exp.format.toUpperCase()}
                  </p>
                  <p className="text-xs text-zinc-400 dark:text-zinc-600">
                    {exp.status === "completed" && exp.file_size_bytes
                      ? `${(exp.file_size_bytes / 1024 / 1024 / 1024).toFixed(1)} GB \u00b7 `
                      : ""}
                    {exp.completed_at
                      ? `Completed ${new Date(exp.completed_at).toLocaleDateString()}`
                      : `Created ${new Date(exp.created_at).toLocaleDateString()}`}
                    {exp.error && ` \u00b7 ${exp.error}`}
                  </p>
                </div>
                <div className="flex items-center gap-2">
                  <ExportStatusBadge status={exp.status} />
                  {exp.status === "completed" && (
                    <>
                      <button
                        onClick={() => {
                          setDownloadError(null);
                          setCopiedModelfile(false);
                          setOllamaForExport(exp.id);
                          ollamaRecipe.mutate(exp.id);
                        }}
                        disabled={ollamaRecipe.isPending}
                        className="text-xs text-blue-400 hover:text-blue-300 transition"
                      >
                        Run locally
                      </button>
                      <button
                        onClick={async () => {
                          try {
                            setDownloadError(null);
                            const result = await downloadExport.mutateAsync(
                              exp.id,
                            );
                            window.open(result.download_url, "_blank");
                          } catch (e) {
                            setDownloadError(
                              e instanceof Error ? e.message : "Download failed",
                            );
                          }
                        }}
                        disabled={downloadExport.isPending}
                        className="text-xs text-blue-400 hover:text-blue-300 transition"
                      >
                        Download
                      </button>
                    </>
                  )}
                </div>
              </div>
            ))}
          </div>
        )}

        {/* Run-locally (Ollama) recipe panel */}
        {ollamaForExport && ollamaRecipe.data && (
          <div className="mt-4 rounded-lg border border-zinc-200 dark:border-zinc-800 p-4">
            <div className="flex items-center justify-between mb-3">
              <h3 className="text-sm font-semibold text-zinc-900 dark:text-white">
                Run locally with Ollama
              </h3>
              <button
                onClick={() => setOllamaForExport(null)}
                className="text-xs text-zinc-400 hover:text-zinc-300 transition"
              >
                Close
              </button>
            </div>
            <div className="mb-3">
              <div className="flex items-center justify-between mb-1">
                <span className="text-xs font-medium text-zinc-500 dark:text-zinc-400">
                  Modelfile
                </span>
                <button
                  onClick={() => {
                    navigator.clipboard.writeText(ollamaRecipe.data!.modelfile);
                    setCopiedModelfile(true);
                    toast.success("Modelfile copied");
                  }}
                  className="text-xs text-blue-400 hover:text-blue-300 transition"
                >
                  {copiedModelfile ? "Copied" : "Copy"}
                </button>
              </div>
              <pre className="text-xs bg-zinc-100 dark:bg-zinc-900 rounded p-3 overflow-x-auto text-zinc-800 dark:text-zinc-200 whitespace-pre-wrap">
                {ollamaRecipe.data.modelfile}
              </pre>
            </div>
            <ol className="list-decimal list-inside space-y-1 text-xs text-zinc-600 dark:text-zinc-400">
              {ollamaRecipe.data.instructions.map((step, i) => (
                <li key={i}>{step}</li>
              ))}
            </ol>
          </div>
        )}
        {ollamaForExport && ollamaRecipe.isError && (
          <p className="text-sm text-red-600 dark:text-red-400 mt-2">
            {ollamaRecipe.error.message}
          </p>
        )}
      </div>

      {/* Quick links */}
      {isActive && (
        <div className="rounded-lg border border-zinc-200 dark:border-zinc-800 p-6">
          <h3 className="text-sm font-medium text-zinc-600 dark:text-zinc-400 mb-4">
            Quick Links
          </h3>
          <div className="flex flex-col sm:flex-row gap-2 sm:gap-3">
            <Link
              href={`/projects/${params.id}/models/${params.modelId}/playground`}
              className="rounded-lg bg-blue-600 px-4 py-2 text-sm font-medium text-white hover:bg-blue-500 transition"
            >
              Open Playground
            </Link>
            <Link
              href={`/projects/${params.id}/models/${params.modelId}/evaluation`}
              className="rounded-lg border border-zinc-300 dark:border-zinc-700 px-4 py-2 text-sm text-zinc-600 dark:text-zinc-400 hover:border-zinc-400 dark:hover:border-zinc-600 transition"
            >
              View Evaluation
            </Link>
          </div>
        </div>
      )}
    </div>
  );
}
