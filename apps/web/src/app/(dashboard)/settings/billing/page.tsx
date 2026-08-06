"use client";

import { usePlanLimits } from "@/hooks/use-billing";

export default function BillingSettingsPage() {
  const { data: limits, isLoading } = usePlanLimits();

  return (
    <div className="max-w-4xl">
      <h1 className="text-xl md:text-2xl font-bold text-zinc-900 dark:text-white mb-2">
        Billing
      </h1>
      <p className="text-zinc-600 dark:text-zinc-400 mb-8">
        Your workspace limits. Usage-based costs are tracked on the Usage page.
      </p>

      {isLoading && <p className="text-zinc-500">Loading...</p>}

      {limits && (
        <div className="border border-zinc-200 dark:border-zinc-800 rounded-lg p-6">
          <h2 className="text-lg font-semibold text-zinc-900 dark:text-white mb-4">
            Plan Limits
          </h2>
          <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-4">
            <LimitCard label="Projects" max={limits.max_projects} />
            <LimitCard label="Models" max={limits.max_models} />
            <LimitCard label="Team Members" max={limits.max_team_members} />
            <LimitCard label="Training Pairs" max={limits.max_training_pairs} />
            <LimitCard label="Storage" max={limits.max_storage_gb} unit="GB" />
          </div>
        </div>
      )}
    </div>
  );
}

function LimitCard({
  label,
  max,
  unit,
}: {
  label: string;
  max: number;
  unit?: string;
}) {
  const display = unit
    ? `${max.toLocaleString()} ${unit}`
    : max.toLocaleString();
  return (
    <div className="bg-zinc-50 dark:bg-zinc-900 rounded-md p-4">
      <p className="text-xs text-zinc-500 uppercase tracking-wide">{label}</p>
      <p className="text-lg font-semibold text-zinc-900 dark:text-white mt-1">
        Up to {display}
      </p>
    </div>
  );
}
