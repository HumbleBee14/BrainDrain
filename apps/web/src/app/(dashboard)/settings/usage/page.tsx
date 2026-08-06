"use client";

import { useAuth } from "@clerk/nextjs";
import { useQuery } from "@tanstack/react-query";
import { api, type InferenceUsageDay } from "@/lib/api-client";
import { ErrorState } from "@/components/error-state";

function formatNumber(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return n.toLocaleString();
}

function formatCost(n: number): string {
  return `$${n.toFixed(4)}`;
}

const OPERATION_LABELS: Record<string, string> = {
  training: "Model training (GPU)",
  inference: "Inference (tokens)",
  teacher_serving: "Teacher model serving (GPU)",
  extraction: "Teacher scoring (GPU)",
  deploy: "Deployments",
};

function BarChart({
  data,
  maxValue,
  label,
}: {
  data: { key: string; value: number }[];
  maxValue: number;
  label: string;
}) {
  if (maxValue === 0) return null;

  return (
    <div>
      <p className="text-xs text-zinc-500 mb-2">{label}</p>
      <div className="flex items-end gap-[2px] h-24">
        {data.map((d) => {
          const height = maxValue > 0 ? (d.value / maxValue) * 100 : 0;
          return (
            <div
              key={d.key}
              className="flex-1 bg-emerald-600 rounded-t-sm min-w-[2px] transition-all"
              style={{ height: `${Math.max(height, 1)}%` }}
              title={`${d.value.toLocaleString()}`}
            />
          );
        })}
      </div>
    </div>
  );
}

export default function UsagePage() {
  const { getToken } = useAuth();

  const {
    data: inferenceUsage,
    isLoading: loadingInference,
    isError: errorInference,
    refetch: refetchInference,
  } = useQuery<InferenceUsageDay[]>({
    queryKey: ["inference-usage"],
    queryFn: async () => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.dashboard.getInferenceUsage(token);
    },
  });

  const {
    data: dashUsage,
    isLoading: loadingDash,
    isError: errorDash,
    isFetching: fetchingDash,
    refetch: refetchDash,
  } = useQuery({
    queryKey: ["dashboard-usage"],
    queryFn: async () => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.dashboard.getUsage(token);
    },
  });

  const isLoading = loadingInference || loadingDash;
  const isError = errorInference || errorDash;

  // Compute totals from inference data
  const totalRequests =
    inferenceUsage?.reduce((sum, d) => sum + d.request_count, 0) ?? 0;
  const totalPromptTokens =
    inferenceUsage?.reduce((sum, d) => sum + d.prompt_tokens, 0) ?? 0;
  const totalCompletionTokens =
    inferenceUsage?.reduce((sum, d) => sum + d.completion_tokens, 0) ?? 0;
  const totalInferenceCost =
    inferenceUsage?.reduce((sum, d) => sum + d.cost_usd, 0) ?? 0;

  return (
    <div>
      <h1 className="text-xl font-bold text-zinc-900 dark:text-white mb-6">Usage</h1>

      {isError ? (
        <ErrorState
          title="Couldn't load usage data"
          message="We couldn't reach the usage service. Your data is safe — please try again."
          onRetry={() => {
            refetchInference();
            refetchDash();
          }}
          isRetrying={loadingInference || fetchingDash}
        />
      ) : isLoading ? (
        <p className="text-zinc-500">Loading usage data...</p>
      ) : (
        <>
          {/* Summary cards */}
          <div className="grid grid-cols-1 sm:grid-cols-2 md:grid-cols-4 gap-3 md:gap-4 mb-8">
            <div className="rounded-lg border border-zinc-200 dark:border-zinc-800 p-4">
              <p className="text-xs text-zinc-500 uppercase tracking-wider">
                Total Requests
              </p>
              <p className="text-2xl font-bold text-zinc-900 dark:text-white mt-1">
                {formatNumber(totalRequests)}
              </p>
              <p className="text-xs text-zinc-400 dark:text-zinc-600 mt-1">Last 30 days</p>
            </div>
            <div className="rounded-lg border border-zinc-200 dark:border-zinc-800 p-4">
              <p className="text-xs text-zinc-500 uppercase tracking-wider">
                Prompt Tokens
              </p>
              <p className="text-2xl font-bold text-zinc-900 dark:text-white mt-1">
                {formatNumber(totalPromptTokens)}
              </p>
              <p className="text-xs text-zinc-400 dark:text-zinc-600 mt-1">Last 30 days</p>
            </div>
            <div className="rounded-lg border border-zinc-200 dark:border-zinc-800 p-4">
              <p className="text-xs text-zinc-500 uppercase tracking-wider">
                Completion Tokens
              </p>
              <p className="text-2xl font-bold text-zinc-900 dark:text-white mt-1">
                {formatNumber(totalCompletionTokens)}
              </p>
              <p className="text-xs text-zinc-400 dark:text-zinc-600 mt-1">Last 30 days</p>
            </div>
            <div className="rounded-lg border border-zinc-200 dark:border-zinc-800 p-4">
              <p className="text-xs text-zinc-500 uppercase tracking-wider">
                Inference Cost
              </p>
              <p className="text-2xl font-bold text-zinc-900 dark:text-white mt-1">
                {formatCost(totalInferenceCost)}
              </p>
              <p className="text-xs text-zinc-400 dark:text-zinc-600 mt-1">Last 30 days</p>
            </div>
          </div>

          {/* Overall billing totals */}
          {dashUsage && (
            <div className="rounded-lg border border-zinc-200 dark:border-zinc-800 p-6 mb-8">
              <h2 className="text-sm font-medium text-zinc-600 dark:text-zinc-400 mb-4">
                Billing Period Totals
              </h2>
              <div className="grid grid-cols-1 sm:grid-cols-3 gap-4 sm:gap-6">
                <div>
                  <p className="text-xs text-zinc-500">Total Cost</p>
                  <p className="text-lg font-semibold text-zinc-900 dark:text-white">
                    ${dashUsage.total_cost_usd.toFixed(2)}
                  </p>
                </div>
                <div>
                  <p className="text-xs text-zinc-500">Total Tokens In</p>
                  <p className="text-lg font-semibold text-zinc-900 dark:text-white">
                    {formatNumber(dashUsage.total_tokens_in)}
                  </p>
                </div>
                <div>
                  <p className="text-xs text-zinc-500">Total Tokens Out</p>
                  <p className="text-lg font-semibold text-zinc-900 dark:text-white">
                    {formatNumber(dashUsage.total_tokens_out)}
                  </p>
                </div>
              </div>
            </div>
          )}

          {/* Cost attribution per operation. Optional-chained: an API from
              before this field existed returns summaries without it. */}
          {dashUsage && (dashUsage.cost_by_operation?.length ?? 0) > 0 && (
            <div className="rounded-lg border border-zinc-200 dark:border-zinc-800 p-6 mb-8">
              <h2 className="text-sm font-medium text-zinc-600 dark:text-zinc-400 mb-4">
                Where the Money Went
              </h2>
              <div className="space-y-4">
                {dashUsage.cost_by_operation.map((op) => {
                  const share =
                    dashUsage.total_cost_usd > 0
                      ? op.cost_usd / dashUsage.total_cost_usd
                      : 0;
                  return (
                    <div key={op.operation}>
                      <div className="mb-1 flex items-baseline justify-between gap-2 text-sm">
                        <span className="text-zinc-900 dark:text-white">
                          {OPERATION_LABELS[op.operation] ?? op.operation}
                          <span className="ml-1.5 text-xs text-zinc-400 dark:text-zinc-600">
                            {op.events.toLocaleString()} event
                            {op.events === 1 ? "" : "s"}
                          </span>
                        </span>
                        <span className="font-mono text-zinc-600 dark:text-zinc-400">
                          {formatCost(op.cost_usd)}
                          <span className="ml-1.5 text-xs text-zinc-400 dark:text-zinc-600">
                            {(share * 100).toFixed(0)}%
                          </span>
                        </span>
                      </div>
                      <div
                        className="h-2 overflow-hidden rounded-full bg-zinc-100 dark:bg-zinc-800"
                        title={`${formatCost(op.cost_usd)} · ${(share * 100).toFixed(1)}% of total spend`}
                      >
                        <div
                          className="h-full rounded-full bg-violet-500"
                          style={{
                            width: `${Math.max(share * 100, op.cost_usd > 0 ? 1 : 0)}%`,
                          }}
                        />
                      </div>
                    </div>
                  );
                })}
              </div>
            </div>
          )}

          {/* Charts */}
          {inferenceUsage && inferenceUsage.length > 0 ? (
            <div className="grid grid-cols-1 md:grid-cols-2 gap-6 mb-8">
              <div className="rounded-lg border border-zinc-200 dark:border-zinc-800 p-6">
                <BarChart
                  data={inferenceUsage.map((d) => ({
                    key: d.date,
                    value: d.request_count,
                  }))}
                  maxValue={Math.max(
                    ...inferenceUsage.map((d) => d.request_count),
                  )}
                  label="Requests / Day"
                />
                <div className="flex justify-between mt-2">
                  <span className="text-xs text-zinc-400 dark:text-zinc-600">
                    {inferenceUsage[0]?.date}
                  </span>
                  <span className="text-xs text-zinc-400 dark:text-zinc-600">
                    {inferenceUsage[inferenceUsage.length - 1]?.date}
                  </span>
                </div>
              </div>

              <div className="rounded-lg border border-zinc-200 dark:border-zinc-800 p-6">
                <BarChart
                  data={inferenceUsage.map((d) => ({
                    key: d.date,
                    value: d.prompt_tokens + d.completion_tokens,
                  }))}
                  maxValue={Math.max(
                    ...inferenceUsage.map(
                      (d) => d.prompt_tokens + d.completion_tokens,
                    ),
                  )}
                  label="Tokens / Day"
                />
                <div className="flex justify-between mt-2">
                  <span className="text-xs text-zinc-400 dark:text-zinc-600">
                    {inferenceUsage[0]?.date}
                  </span>
                  <span className="text-xs text-zinc-400 dark:text-zinc-600">
                    {inferenceUsage[inferenceUsage.length - 1]?.date}
                  </span>
                </div>
              </div>
            </div>
          ) : (
            <div className="rounded-lg border border-zinc-200 dark:border-zinc-800 p-8 text-center mb-8">
              <p className="text-zinc-500">No inference usage data yet.</p>
              <p className="text-xs text-zinc-400 dark:text-zinc-600 mt-1">
                Deploy a model and send inference requests to see usage data
                here.
              </p>
            </div>
          )}

          {/* Daily breakdown table */}
          {inferenceUsage && inferenceUsage.length > 0 && (
            <div>
              <h2 className="text-sm font-medium text-zinc-600 dark:text-zinc-400 mb-3">
                Daily Breakdown
              </h2>
              <div className="rounded-lg border border-zinc-200 dark:border-zinc-800 overflow-hidden overflow-x-auto">
                <table className="w-full text-sm">
                  <thead>
                    <tr className="border-b border-zinc-200 dark:border-zinc-800">
                      <th className="px-4 py-2 text-left text-xs font-medium text-zinc-500">
                        Date
                      </th>
                      <th className="px-4 py-2 text-right text-xs font-medium text-zinc-500">
                        Requests
                      </th>
                      <th className="px-4 py-2 text-right text-xs font-medium text-zinc-500">
                        Prompt Tokens
                      </th>
                      <th className="px-4 py-2 text-right text-xs font-medium text-zinc-500">
                        Completion Tokens
                      </th>
                      <th className="px-4 py-2 text-right text-xs font-medium text-zinc-500">
                        Cost
                      </th>
                    </tr>
                  </thead>
                  <tbody>
                    {[...inferenceUsage].reverse().map((day) => (
                      <tr
                        key={day.date}
                        className="border-b border-zinc-100 dark:border-zinc-800/50 last:border-b-0"
                      >
                        <td className="px-4 py-2 text-zinc-700 dark:text-zinc-300">{day.date}</td>
                        <td className="px-4 py-2 text-right text-zinc-600 dark:text-zinc-400">
                          {day.request_count.toLocaleString()}
                        </td>
                        <td className="px-4 py-2 text-right text-zinc-600 dark:text-zinc-400">
                          {day.prompt_tokens.toLocaleString()}
                        </td>
                        <td className="px-4 py-2 text-right text-zinc-600 dark:text-zinc-400">
                          {day.completion_tokens.toLocaleString()}
                        </td>
                        <td className="px-4 py-2 text-right text-zinc-600 dark:text-zinc-400">
                          {formatCost(day.cost_usd)}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </div>
          )}
        </>
      )}
    </div>
  );
}
