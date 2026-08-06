"use client";

import Link from "next/link";
import { OnboardingBanner } from "@/components/onboarding-banner";
import { ErrorState } from "@/components/error-state";
import { CostChart } from "@/components/cost-chart";
import {
  useDashboardStats,
  useUsageSummary,
  useRecentActivity,
} from "@/hooks/use-dashboard";

export default function DashboardPage() {
  const {
    data: stats,
    isLoading: statsLoading,
    isError: statsError,
    isFetching: statsFetching,
    refetch: refetchStats,
  } = useDashboardStats();
  const {
    data: usage,
    isLoading: usageLoading,
    isError: usageError,
    isFetching: usageFetching,
    refetch: refetchUsage,
  } = useUsageSummary();
  const {
    data: activity,
    isLoading: activityLoading,
    isError: activityError,
    isFetching: activityFetching,
    refetch: refetchActivity,
  } = useRecentActivity();

  const isError = statsError || usageError || activityError;
  const isFetching = statsFetching || usageFetching || activityFetching;

  return (
    <div>
      <OnboardingBanner />

      <div className="flex items-center justify-between mb-6 md:mb-8">
        <h1 className="text-xl md:text-2xl font-bold text-zinc-900 dark:text-white">Dashboard</h1>
        <Link
          href="/projects/new"
          className="rounded-lg bg-zinc-900 text-white hover:bg-zinc-800 dark:bg-white dark:text-zinc-950 dark:hover:bg-zinc-200 px-4 py-2 text-sm font-semibold transition"
        >
          New Project
        </Link>
      </div>

      {isError ? (
        <ErrorState
          title="Couldn't load your dashboard"
          message="We couldn't reach the dashboard service. Your data is safe — please try again."
          onRetry={() => {
            refetchStats();
            refetchUsage();
            refetchActivity();
          }}
          isRetrying={isFetching}
        />
      ) : (
        <>
      {/* Stats Cards */}
      <div className="grid grid-cols-2 sm:grid-cols-2 lg:grid-cols-4 gap-2 sm:gap-4 mb-6 md:mb-8">
        <StatCard
          label="Projects"
          value={stats?.total_projects}
          loading={statsLoading}
          href="/projects"
        />
        <StatCard
          label="Models"
          value={stats?.total_models}
          loading={statsLoading}
        />
        <StatCard
          label="Active Training"
          value={stats?.active_training_jobs}
          loading={statsLoading}
        />
        <StatCard
          label="Deployed"
          value={stats?.deployed_models}
          loading={statsLoading}
        />
      </div>

      {/* Usage Summary + Cost Chart */}
      <div className="grid grid-cols-1 md:grid-cols-2 gap-3 md:gap-4 mb-6 md:mb-8">
        <div className="border border-zinc-200 dark:border-zinc-800 rounded-lg p-6">
          <div className="mb-3 flex items-baseline justify-between">
            <h2 className="text-sm text-zinc-500 uppercase tracking-wide">
              Usage
            </h2>
            <Link
              href="/settings/usage"
              className="text-xs font-medium text-violet-600 underline-offset-2 hover:underline dark:text-violet-400"
            >
              View details →
            </Link>
          </div>
          {usageLoading ? (
            <p className="text-zinc-500">Loading...</p>
          ) : (
            <div className="space-y-3">
              <div className="flex justify-between">
                <span className="text-zinc-600 dark:text-zinc-400 text-sm">Total Cost</span>
                <span className="text-zinc-900 dark:text-white font-semibold">
                  ${(usage?.total_cost_usd ?? 0).toFixed(2)}
                </span>
              </div>
              <div className="flex justify-between">
                <span className="text-zinc-600 dark:text-zinc-400 text-sm">Tokens In</span>
                <span className="text-zinc-900 dark:text-white font-semibold">
                  {(usage?.total_tokens_in ?? 0).toLocaleString()}
                </span>
              </div>
              <div className="flex justify-between">
                <span className="text-zinc-600 dark:text-zinc-400 text-sm">Tokens Out</span>
                <span className="text-zinc-900 dark:text-white font-semibold">
                  {(usage?.total_tokens_out ?? 0).toLocaleString()}
                </span>
              </div>
              <div className="flex justify-between">
                <span className="text-zinc-600 dark:text-zinc-400 text-sm">Total Events</span>
                <span className="text-zinc-900 dark:text-white font-semibold">
                  {(usage?.total_events ?? 0).toLocaleString()}
                </span>
              </div>
            </div>
          )}
        </div>

        {/* Cost Chart */}
        <div className="border border-zinc-200 dark:border-zinc-800 rounded-lg p-6">
          <div className="mb-3 flex items-baseline justify-between">
            <h2 className="text-sm text-zinc-500 uppercase tracking-wide">
              Daily Cost
            </h2>
            <Link
              href="/settings/billing"
              className="text-xs font-medium text-violet-600 underline-offset-2 hover:underline dark:text-violet-400"
            >
              Billing →
            </Link>
          </div>
          {usageLoading ? (
            <p className="text-zinc-500">Loading...</p>
          ) : (
            <CostChart costByDay={usage?.cost_by_day ?? []} />
          )}
        </div>
      </div>

      {/* Additional Stats Row */}
      <div className="grid grid-cols-3 gap-2 sm:gap-4 mb-6 md:mb-8">
        <StatCard
          label="Documents"
          value={stats?.total_documents}
          loading={statsLoading}
        />
        <StatCard
          label="Training Jobs"
          value={stats?.total_training_jobs}
          loading={statsLoading}
        />
        <StatCard
          label="Evaluations"
          value={stats?.total_evaluations}
          loading={statsLoading}
        />
      </div>

      {/* Recent Activity */}
      <div className="border border-zinc-200 dark:border-zinc-800 rounded-lg">
        <div className="p-4 border-b border-zinc-200 dark:border-zinc-800">
          <h2 className="text-lg font-semibold text-zinc-900 dark:text-white">Recent Activity</h2>
        </div>
        {activityLoading ? (
          <div className="p-8 text-center text-zinc-500">Loading...</div>
        ) : !activity?.length ? (
          <div className="p-8 text-center text-zinc-500">
            No activity yet. Create a project to get started.
          </div>
        ) : (
          <div className="divide-y divide-zinc-200 dark:divide-zinc-800">
            {activity.slice(0, 10).map((entry) => (
              <div
                key={entry.id}
                className="flex items-center justify-between px-4 py-3"
              >
                <div className="flex items-center gap-3 min-w-0">
                  <ActivityIcon action={entry.action} />
                  <div className="min-w-0">
                    <p className="text-sm text-zinc-900 dark:text-white truncate">
                      <span className="font-medium">
                        {formatAction(entry.action)}
                      </span>{" "}
                      <span className="text-zinc-600 dark:text-zinc-400">
                        {entry.resource_type}
                      </span>
                    </p>
                    {entry.resource_id && (
                      <p className="text-xs text-zinc-400 dark:text-zinc-600 truncate">
                        {entry.resource_id}
                      </p>
                    )}
                  </div>
                </div>
                <span className="text-xs text-zinc-500 whitespace-nowrap ml-4">
                  {formatTimeAgo(entry.created_at)}
                </span>
              </div>
            ))}
          </div>
        )}
      </div>
        </>
      )}
    </div>
  );
}

function StatCard({
  label,
  value,
  loading,
  href,
}: {
  label: string;
  value?: number;
  loading: boolean;
  href?: string;
}) {
  const body = (
    <>
      <p className="text-sm text-zinc-500">{label}</p>
      <p className="text-xl md:text-3xl font-bold text-zinc-900 dark:text-white mt-1">
        {loading ? (
          <span className="text-zinc-300 dark:text-zinc-700">--</span>
        ) : (
          (value ?? 0).toLocaleString()
        )}
      </p>
    </>
  );
  const cls = "rounded-lg border border-zinc-200 dark:border-zinc-800 p-3 md:p-6";
  if (href) {
    return (
      <Link
        href={href}
        className={`${cls} block transition hover:border-zinc-300 dark:hover:border-zinc-700`}
      >
        {body}
      </Link>
    );
  }
  return <div className={cls}>{body}</div>;
}

function ActivityIcon({ action }: { action: string }) {
  let color = "bg-zinc-200 dark:bg-zinc-700";
  if (action.startsWith("create")) color = "bg-emerald-500/20";
  else if (action.startsWith("delete")) color = "bg-red-500/20";
  else if (action.startsWith("update")) color = "bg-blue-500/20";
  else if (action.startsWith("deploy")) color = "bg-amber-500/20";

  return <div className={`w-2 h-2 rounded-full ${color} shrink-0`} />;
}

function formatAction(action: string): string {
  return action
    .split("_")
    .map((w) => w.charAt(0).toUpperCase() + w.slice(1))
    .join(" ");
}

function formatTimeAgo(isoDate: string): string {
  const now = Date.now();
  const then = new Date(isoDate).getTime();
  const diffMs = now - then;

  const minutes = Math.floor(diffMs / 60_000);
  if (minutes < 1) return "just now";
  if (minutes < 60) return `${minutes}m ago`;

  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;

  const days = Math.floor(hours / 24);
  if (days < 30) return `${days}d ago`;

  return new Date(isoDate).toLocaleDateString();
}
