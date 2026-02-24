"use client";

import { useState } from "react";
import { useParams, useSearchParams } from "next/navigation";
import Link from "next/link";
import {
  useDataset,
  useDatasetPreview,
  useApproveDataset,
  useRejectDataset,
} from "@/hooks/use-datasets";

function PairCard({
  pair,
  index,
}: {
  pair: Record<string, unknown>;
  index: number;
}) {
  const messages =
    (pair.messages as Array<{ role: string; content: string }>) || [];
  const userMsg = messages.find((m) => m.role === "user");
  const assistantMsg = messages.find((m) => m.role === "assistant");
  const systemMsg = messages.find((m) => m.role === "system");

  const responseLen = assistantMsg?.content?.length ?? 0;
  const msgCount = messages.length;
  const isShortResponse = responseLen > 0 && responseLen < 50;

  return (
    <div
      className={`rounded-lg border p-4 ${
        isShortResponse ? "border-yellow-700/50" : "border-zinc-800"
      }`}
    >
      <div className="flex items-center justify-between mb-3">
        <span className="text-xs text-zinc-600">Pair #{index + 1}</span>
        <div className="flex items-center gap-2">
          <span className="text-xs text-zinc-600">{msgCount} messages</span>
          {assistantMsg && (
            <span
              className={`text-xs ${isShortResponse ? "text-yellow-500" : "text-zinc-600"}`}
            >
              {responseLen} chars
            </span>
          )}
          {isShortResponse && (
            <span className="text-xs text-yellow-500 font-medium">Short</span>
          )}
        </div>
      </div>
      {systemMsg && (
        <div className="mb-3">
          <p className="text-xs text-zinc-500 uppercase tracking-wider mb-1">
            System
          </p>
          <p className="text-sm text-zinc-400 bg-zinc-900 rounded p-2">
            {systemMsg.content}
          </p>
        </div>
      )}
      {userMsg && (
        <div className="mb-3">
          <p className="text-xs text-zinc-500 uppercase tracking-wider mb-1">
            Instruction
          </p>
          <p className="text-sm text-white bg-zinc-900 rounded p-2">
            {userMsg.content}
          </p>
        </div>
      )}
      {assistantMsg && (
        <div>
          <p className="text-xs text-zinc-500 uppercase tracking-wider mb-1">
            Response
          </p>
          <p className="text-sm text-zinc-300 bg-zinc-900 rounded p-2 whitespace-pre-wrap">
            {assistantMsg.content}
          </p>
        </div>
      )}
    </div>
  );
}

export default function DatasetReviewPage() {
  const params = useParams<{ id: string }>();
  const searchParams = useSearchParams();
  const datasetId = searchParams.get("datasetId") || "";
  const [maxRows, setMaxRows] = useState(20);

  const { data: dataset, isLoading: loadingDataset } = useDataset(datasetId);
  const { data: preview, isLoading: loadingPreview } = useDatasetPreview(
    datasetId,
    maxRows,
  );
  const approveDataset = useApproveDataset();
  const rejectDataset = useRejectDataset();

  const isReviewPending = dataset?.status === "review_pending";
  const isMutating = approveDataset.isPending || rejectDataset.isPending;

  if (!datasetId) {
    return (
      <div className="flex flex-col items-center justify-center py-20 gap-4">
        <p className="text-zinc-500">No dataset selected</p>
        <Link
          href={`/projects/${params.id}`}
          className="text-sm text-white underline hover:no-underline"
        >
          Back to Project
        </Link>
      </div>
    );
  }

  if (loadingDataset || loadingPreview) {
    return (
      <div className="flex items-center justify-center py-20">
        <p className="text-zinc-500">Loading dataset...</p>
      </div>
    );
  }

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
          <h1 className="text-2xl font-bold text-white">
            {dataset?.name || "Dataset"}
          </h1>
          {dataset && (
            <span
              className={`inline-flex items-center rounded-full border px-2.5 py-0.5 text-xs font-medium ${
                dataset.status === "approved"
                  ? "border-emerald-700 bg-emerald-900/30 text-emerald-400"
                  : dataset.status === "review_pending"
                    ? "border-yellow-700 bg-yellow-900/30 text-yellow-400"
                    : dataset.status === "archived"
                      ? "border-red-700 bg-red-900/30 text-red-400"
                      : "border-zinc-700 bg-zinc-800 text-zinc-400"
              }`}
            >
              {dataset.status}
            </span>
          )}

          {/* Approve/Reject buttons */}
          {isReviewPending && (
            <div className="flex items-center gap-2 ml-auto">
              <button
                onClick={() => approveDataset.mutate(datasetId)}
                disabled={isMutating}
                className="rounded-lg bg-emerald-600 px-4 py-1.5 text-sm font-medium text-white hover:bg-emerald-500 transition disabled:opacity-50 disabled:cursor-not-allowed"
              >
                {approveDataset.isPending ? "Approving..." : "Approve Dataset"}
              </button>
              <button
                onClick={() => rejectDataset.mutate(datasetId)}
                disabled={isMutating}
                className="rounded-lg bg-red-600 px-4 py-1.5 text-sm font-medium text-white hover:bg-red-500 transition disabled:opacity-50 disabled:cursor-not-allowed"
              >
                {rejectDataset.isPending ? "Rejecting..." : "Reject Dataset"}
              </button>
            </div>
          )}
        </div>
        {dataset && (
          <p className="text-zinc-500 mt-1">
            {dataset.pair_count != null
              ? `${dataset.pair_count} pairs`
              : "Generating"}
            {" \u00b7 "}
            {dataset.format}
          </p>
        )}
        {(approveDataset.isError || rejectDataset.isError) && (
          <p className="text-sm text-red-400 mt-2">
            {approveDataset.error?.message || rejectDataset.error?.message}
          </p>
        )}
      </div>

      {/* Stats */}
      {dataset?.stats &&
        typeof dataset.stats === "object" &&
        Object.keys(dataset.stats).length > 0 && (
          <div className="grid grid-cols-2 md:grid-cols-4 gap-3 mb-8">
            {Object.entries(dataset.stats).map(([key, value]) => (
              <div key={key} className="rounded-lg border border-zinc-800 p-4">
                <p className="text-xs text-zinc-500 uppercase tracking-wider">
                  {key.replace(/_/g, " ")}
                </p>
                <p className="text-white mt-1 text-lg font-semibold">
                  {typeof value === "number"
                    ? value.toLocaleString()
                    : String(value)}
                </p>
              </div>
            ))}
          </div>
        )}

      {/* Preview pairs */}
      <div>
        <h2 className="text-lg font-semibold text-white mb-4">
          Preview {preview && `(${preview.length} samples)`}
        </h2>
        {preview && preview.length > 0 ? (
          <>
            <div className="space-y-4">
              {preview.map((pair, i) => (
                <PairCard key={i} pair={pair} index={i} />
              ))}
            </div>
            {/* Load More pagination */}
            {preview.length >= maxRows && (
              <div className="mt-6 text-center">
                <button
                  onClick={() => setMaxRows((prev) => prev + 20)}
                  className="rounded-lg border border-zinc-700 px-6 py-2 text-sm text-zinc-400 hover:border-zinc-500 hover:text-white transition"
                >
                  Load More
                </button>
              </div>
            )}
          </>
        ) : (
          <div className="rounded-lg border border-dashed border-zinc-700 p-8 text-center">
            <p className="text-zinc-500">No preview data available yet</p>
          </div>
        )}
      </div>
    </div>
  );
}
