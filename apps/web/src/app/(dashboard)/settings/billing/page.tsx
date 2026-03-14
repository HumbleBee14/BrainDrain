"use client";

import {
  useSubscription,
  usePlanLimits,
  useCreateCheckout,
  useCreatePortal,
} from "@/hooks/use-billing";

const PLANS = [
  {
    id: "starter",
    name: "Starter",
    price: "Free",
    features: [
      "2 projects",
      "2 models",
      "1 team member",
      "1K training pairs",
      "5 GB storage",
    ],
  },
  {
    id: "growth",
    name: "Growth",
    price: "$49/mo",
    features: [
      "10 projects",
      "10 models",
      "10 team members",
      "50K training pairs",
      "50 GB storage",
    ],
  },
  {
    id: "pro",
    name: "Pro",
    price: "$199/mo",
    features: [
      "100 projects",
      "50 models",
      "50 team members",
      "500K training pairs",
      "500 GB storage",
    ],
  },
];

export default function BillingSettingsPage() {
  const { data: subscription, isLoading: subLoading } = useSubscription();
  const { data: limits } = usePlanLimits();
  const createCheckout = useCreateCheckout();
  const createPortal = useCreatePortal();

  const currentPlan = subscription?.plan || "starter";

  const handleUpgrade = (plan: string) => {
    createCheckout.mutate(
      {
        plan,
        success_url: window.location.href,
        cancel_url: window.location.href,
      },
      {
        onSuccess: (data) => {
          if (data.url) window.location.href = data.url;
        },
      },
    );
  };

  const handleManageBilling = () => {
    createPortal.mutate(
      { return_url: window.location.href },
      {
        onSuccess: (data) => {
          if (data.url) window.location.href = data.url;
        },
      },
    );
  };

  if (subLoading) {
    return (
      <div className="max-w-4xl">
        <h1 className="text-xl md:text-2xl font-bold text-zinc-900 dark:text-white mb-2">Billing</h1>
        <p className="text-zinc-500">Loading...</p>
      </div>
    );
  }

  return (
    <div className="max-w-4xl">
      <h1 className="text-xl md:text-2xl font-bold text-zinc-900 dark:text-white mb-2">Billing</h1>
      <p className="text-zinc-600 dark:text-zinc-400 mb-8">
        Manage your subscription and plan limits.
      </p>

      {/* Current Subscription Card */}
      <div className="border border-zinc-200 dark:border-zinc-800 rounded-lg p-6 mb-8">
        <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3">
          <div>
            <h2 className="text-lg font-semibold text-zinc-900 dark:text-white">Current Plan</h2>
            <p className="text-zinc-600 dark:text-zinc-400 text-sm mt-1">
              You are on the{" "}
              <span className="text-emerald-400 font-medium capitalize">
                {currentPlan}
              </span>{" "}
              plan.
              {subscription?.status && (
                <span className="ml-2 text-xs text-zinc-500">
                  Status: {subscription.status}
                </span>
              )}
            </p>
            {subscription?.current_period_end && (
              <p className="text-zinc-500 text-xs mt-1">
                Current period ends{" "}
                {new Date(subscription.current_period_end).toLocaleDateString()}
              </p>
            )}
          </div>
          {currentPlan !== "starter" && (
            <button
              onClick={handleManageBilling}
              disabled={createPortal.isPending}
              className="bg-zinc-100 dark:bg-zinc-800 hover:bg-zinc-200 dark:hover:bg-zinc-700 disabled:opacity-50 text-zinc-900 dark:text-white px-4 py-2 rounded-md text-sm font-medium transition"
            >
              {createPortal.isPending ? "Opening..." : "Manage Billing"}
            </button>
          )}
        </div>
        {createPortal.isError && (
          <p className="text-red-400 text-sm mt-2">
            {createPortal.error.message}
          </p>
        )}
      </div>

      {/* Plan Cards Grid */}
      <div className="grid grid-cols-1 sm:grid-cols-2 md:grid-cols-3 gap-3 md:gap-4 mb-8">
        {PLANS.map((plan) => {
          const isCurrent = plan.id === currentPlan;
          return (
            <div
              key={plan.id}
              className={`border rounded-lg p-6 ${
                isCurrent
                  ? "border-emerald-500 bg-emerald-50 dark:bg-emerald-500/5"
                  : "border-zinc-200 dark:border-zinc-800"
              }`}
            >
              <h3 className="text-lg font-semibold text-zinc-900 dark:text-white">{plan.name}</h3>
              <p className="text-2xl font-bold text-zinc-900 dark:text-white mt-2">{plan.price}</p>
              <ul className="mt-4 space-y-2">
                {plan.features.map((feature) => (
                  <li
                    key={feature}
                    className="text-sm text-zinc-600 dark:text-zinc-400 flex items-start gap-2"
                  >
                    <span className="text-emerald-400 mt-0.5">&#10003;</span>
                    {feature}
                  </li>
                ))}
              </ul>
              <div className="mt-6">
                {isCurrent ? (
                  <span className="block text-center text-sm text-emerald-400 font-medium py-2">
                    Current Plan
                  </span>
                ) : (
                  <button
                    onClick={() => handleUpgrade(plan.id)}
                    disabled={createCheckout.isPending}
                    className="w-full bg-emerald-600 hover:bg-emerald-500 disabled:opacity-50 disabled:cursor-not-allowed text-white px-4 py-2 rounded-md text-sm font-medium transition"
                  >
                    {createCheckout.isPending
                      ? "Processing..."
                      : plan.id === "starter"
                        ? "Downgrade"
                        : "Upgrade"}
                  </button>
                )}
              </div>
            </div>
          );
        })}
      </div>
      {createCheckout.isError && (
        <p className="text-red-400 text-sm mb-4">
          {createCheckout.error.message}
        </p>
      )}

      {/* Plan Usage */}
      {limits && (
        <div className="border border-zinc-200 dark:border-zinc-800 rounded-lg p-6">
          <h2 className="text-lg font-semibold text-zinc-900 dark:text-white mb-4">Plan Limits</h2>
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
      <p className="text-lg font-semibold text-zinc-900 dark:text-white mt-1">Up to {display}</p>
    </div>
  );
}
