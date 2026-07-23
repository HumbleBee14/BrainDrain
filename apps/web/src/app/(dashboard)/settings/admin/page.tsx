"use client";

import { useEffect, useState } from "react";
import { toast } from "sonner";
import { z } from "zod";
import {
  useAdminConfig,
  useUpdateAdminConfig,
  useResetAdminConfig,
} from "@/hooks/use-settings";
import { useCurrentRole } from "@/hooks/use-team";
import { ErrorState } from "@/components/error-state";
import type { AdminConfigResponse } from "@/lib/api-client";

const positiveInt = (min: number) =>
  z.number().int("Must be a whole number").gte(min, `Must be at least ${min}`);
const nonNegative = z.number().gte(0, "Cannot be negative");

const adminConfigSchema = z.object({
  gpu_rates: z.record(z.string(), nonNegative),
  cost_approval_threshold: nonNegative,
  inference_input_cost_per_million: nonNegative,
  inference_output_cost_per_million: nonNegative,
  default_max_tokens: positiveInt(1),
  default_rate_limit_rpm: positiveInt(1),
  max_batch_size: positiveInt(1),
  chunk_size_tokens: positiveInt(100),
});

const GPU_LABELS: Record<string, string> = {
  t4: "T4",
  a10g: "A10G",
  l40s: "L40S",
  a10040gb: "A100 40GB",
  a10080gb: "A100 80GB",
  h100: "H100",
};

function NumberField({
  label,
  description,
  value,
  onChange,
  step,
  min,
  prefix,
  suffix,
  error,
}: {
  label: string;
  description: string;
  value: number;
  onChange: (v: number) => void;
  step?: number;
  min?: number;
  prefix?: string;
  suffix?: string;
  error?: string;
}) {
  return (
    <div className="rounded-lg border border-zinc-200 dark:border-zinc-800 p-4">
      <label className="block text-sm text-zinc-900 dark:text-white font-medium mb-1">
        {label}
      </label>
      <p className="text-xs text-zinc-500 mb-2">{description}</p>
      <div className="flex items-center gap-1">
        {prefix && <span className="text-sm text-zinc-500">{prefix}</span>}
        <input
          type="number"
          value={Number.isNaN(value) ? "" : value}
          onChange={(e) => {
            const raw = e.target.value;
            onChange(raw === "" ? NaN : parseFloat(raw));
          }}
          step={step ?? 1}
          min={min ?? 0}
          className={`w-full rounded-lg border bg-zinc-50 dark:bg-zinc-900 px-3 py-2 text-sm text-zinc-900 dark:text-white font-mono ${
            error
              ? "border-red-400 dark:border-red-500"
              : "border-zinc-300 dark:border-zinc-700"
          }`}
        />
        {suffix && <span className="text-sm text-zinc-500">{suffix}</span>}
      </div>
      {error && <p className="mt-1 text-xs text-red-500">{error}</p>}
    </div>
  );
}

export default function AdminConfigPage() {
  const { data: config, isLoading, isError, refetch, isFetching } =
    useAdminConfig();
  const { isAdmin, isLoading: roleLoading } = useCurrentRole();
  const updateConfig = useUpdateAdminConfig();
  const resetConfig = useResetAdminConfig();

  const [form, setForm] = useState<AdminConfigResponse | null>(null);
  const [dirty, setDirty] = useState(false);
  const [errors, setErrors] = useState<Record<string, string>>({});

  useEffect(() => {
    if (config && !form) {
      setForm(config);
    }
  }, [config, form]);

  useEffect(() => {
    if (updateConfig.isSuccess) toast.success("Admin configuration saved");
  }, [updateConfig.isSuccess]);
  useEffect(() => {
    if (updateConfig.isError) toast.error(updateConfig.error.message);
  }, [updateConfig.isError, updateConfig.error]);
  useEffect(() => {
    if (resetConfig.isSuccess) {
      toast.success("Configuration reset to platform defaults");
      setForm(null);
    }
  }, [resetConfig.isSuccess]);

  const updateField = <K extends keyof AdminConfigResponse>(
    key: K,
    value: AdminConfigResponse[K],
  ) => {
    if (!form) return;
    setForm({ ...form, [key]: value });
    setDirty(true);
    setErrors({});
  };

  const updateGpuRate = (gpuClass: string, rate: number) => {
    if (!form) return;
    setForm({
      ...form,
      gpu_rates: { ...form.gpu_rates, [gpuClass]: rate },
    });
    setDirty(true);
    setErrors({});
  };

  const handleSave = () => {
    if (!form) return;
    const parsed = adminConfigSchema.safeParse(form);
    if (!parsed.success) {
      const fieldErrors: Record<string, string> = {};
      for (const issue of parsed.error.issues) {
        fieldErrors[issue.path.join(".")] = issue.message;
      }
      setErrors(fieldErrors);
      toast.error("Please fix the highlighted fields before saving");
      return;
    }
    setErrors({});
    updateConfig.mutate(parsed.data);
    setDirty(false);
  };

  const handleReset = () => {
    resetConfig.mutate(undefined as unknown as void);
    setDirty(false);
  };

  if (roleLoading) {
    return (
      <div className="flex items-center justify-center py-20">
        <p className="text-zinc-500">Loading configuration...</p>
      </div>
    );
  }

  if (!isAdmin) {
    return (
      <div className="flex flex-col items-center justify-center text-center py-20">
        <div className="flex h-12 w-12 items-center justify-center rounded-full bg-zinc-100 dark:bg-zinc-800 mb-4">
          <svg
            className="h-6 w-6 text-zinc-500"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth={2}
            strokeLinecap="round"
            strokeLinejoin="round"
            aria-hidden="true"
          >
            <rect width="18" height="11" x="3" y="11" rx="2" ry="2" />
            <path d="M7 11V7a5 5 0 0 1 10 0v4" />
          </svg>
        </div>
        <h1 className="text-lg font-semibold text-zinc-900 dark:text-white">
          Admin access required
        </h1>
        <p className="mt-1 max-w-sm text-sm text-zinc-500">
          Only organization admins and owners can view or change platform
          configuration. Contact an admin if you need access.
        </p>
      </div>
    );
  }

  if (isError) {
    return (
      <ErrorState
        title="Couldn't load admin configuration"
        message="The configuration service didn't respond. Check your connection and try again."
        onRetry={() => refetch()}
        isRetrying={isFetching}
      />
    );
  }

  if (isLoading || !form) {
    return (
      <div className="flex items-center justify-center py-20">
        <p className="text-zinc-500">Loading configuration...</p>
      </div>
    );
  }

  return (
    <div>
      <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3 mb-6">
        <div>
          <h1 className="text-xl md:text-2xl font-bold text-zinc-900 dark:text-white">
            Admin Configuration
          </h1>
          <p className="text-sm text-zinc-500 mt-1">
            Customize platform settings for your organization. Changes override
            platform defaults.
          </p>
        </div>
        <div className="flex items-center gap-2">
          {config?.is_configured && (
            <button
              onClick={handleReset}
              disabled={resetConfig.isPending}
              className="rounded-lg border border-zinc-300 dark:border-zinc-700 px-4 py-2 text-sm text-zinc-600 dark:text-zinc-400 hover:border-zinc-400 dark:hover:border-zinc-600 transition disabled:opacity-50"
            >
              Reset to Defaults
            </button>
          )}
          <button
            onClick={handleSave}
            disabled={!dirty || updateConfig.isPending}
            className="rounded-lg bg-emerald-600 px-4 py-2 text-sm font-medium text-white hover:bg-emerald-500 transition disabled:opacity-50"
          >
            {updateConfig.isPending ? "Saving..." : "Save Changes"}
          </button>
        </div>
      </div>

      {/* GPU Rates */}
      <div className="mb-8">
        <h2 className="text-lg font-semibold text-zinc-900 dark:text-white mb-1">
          GPU Hourly Rates
        </h2>
        <p className="text-xs text-zinc-500 mb-4">
          Cost per hour for each GPU class, used in training cost estimation.
        </p>
        <div className="grid grid-cols-1 sm:grid-cols-2 md:grid-cols-3 gap-2 md:gap-3">
          {Object.entries(form.gpu_rates).map(([gpuClass, rate]) => (
            <div
              key={gpuClass}
              className="rounded-lg border border-zinc-200 dark:border-zinc-800 p-3"
            >
              <label className="block text-xs text-zinc-500 mb-1">
                {GPU_LABELS[gpuClass] || gpuClass.toUpperCase()}
              </label>
              <div className="flex items-center gap-1">
                <span className="text-sm text-zinc-500">$</span>
                <input
                  type="number"
                  value={Number.isNaN(rate) ? "" : rate}
                  onChange={(e) =>
                    updateGpuRate(
                      gpuClass,
                      e.target.value === ""
                        ? NaN
                        : parseFloat(e.target.value),
                    )
                  }
                  step={0.1}
                  min={0}
                  className={`w-full rounded border bg-zinc-50 dark:bg-zinc-900 px-2 py-1.5 text-sm text-zinc-900 dark:text-white font-mono ${
                    errors[`gpu_rates.${gpuClass}`]
                      ? "border-red-400 dark:border-red-500"
                      : "border-zinc-300 dark:border-zinc-700"
                  }`}
                />
                <span className="text-xs text-zinc-400 dark:text-zinc-600">/hr</span>
              </div>
              {errors[`gpu_rates.${gpuClass}`] && (
                <p className="mt-1 text-xs text-red-500">
                  {errors[`gpu_rates.${gpuClass}`]}
                </p>
              )}
            </div>
          ))}
        </div>
      </div>

      {/* Training */}
      <div className="mb-8">
        <h2 className="text-lg font-semibold text-zinc-900 dark:text-white mb-1">Training</h2>
        <p className="text-xs text-zinc-500 mb-4">
          Controls for training cost approval and document processing.
        </p>
        <div className="grid grid-cols-1 md:grid-cols-2 gap-2 md:gap-3">
          <NumberField
            label="Cost Approval Threshold"
            description="Training jobs above this cost (USD) require admin approval before starting."
            value={form.cost_approval_threshold}
            onChange={(v) => updateField("cost_approval_threshold", v)}
            step={0.5}
            prefix="$"
            error={errors.cost_approval_threshold}
          />
          <NumberField
            label="Chunk Size"
            description="Document chunk size in tokens for the parsing pipeline."
            value={form.chunk_size_tokens}
            onChange={(v) => updateField("chunk_size_tokens", v)}
            step={100}
            min={100}
            suffix="tokens"
            error={errors.chunk_size_tokens}
          />
        </div>
      </div>

      {/* Inference */}
      <div className="mb-8">
        <h2 className="text-lg font-semibold text-zinc-900 dark:text-white mb-1">Inference</h2>
        <p className="text-xs text-zinc-500 mb-4">
          Pricing and limits for the inference API.
        </p>
        <div className="grid grid-cols-1 md:grid-cols-2 gap-2 md:gap-3">
          <NumberField
            label="Input Token Cost"
            description="Cost per million input (prompt) tokens."
            value={form.inference_input_cost_per_million}
            onChange={(v) =>
              updateField("inference_input_cost_per_million", v)
            }
            step={0.01}
            prefix="$"
            suffix="/M tokens"
            error={errors.inference_input_cost_per_million}
          />
          <NumberField
            label="Output Token Cost"
            description="Cost per million output (completion) tokens."
            value={form.inference_output_cost_per_million}
            onChange={(v) =>
              updateField("inference_output_cost_per_million", v)
            }
            step={0.01}
            prefix="$"
            suffix="/M tokens"
            error={errors.inference_output_cost_per_million}
          />
          <NumberField
            label="Default Max Tokens"
            description="Default max_tokens for inference requests when not specified by client."
            value={form.default_max_tokens}
            onChange={(v) => updateField("default_max_tokens", v)}
            step={64}
            min={1}
            error={errors.default_max_tokens}
          />
          <NumberField
            label="Max Batch Size"
            description="Maximum number of items allowed in a single batch inference request."
            value={form.max_batch_size}
            onChange={(v) => updateField("max_batch_size", v)}
            min={1}
            error={errors.max_batch_size}
          />
        </div>
      </div>

      {/* Rate Limits */}
      <div className="mb-8">
        <h2 className="text-lg font-semibold text-zinc-900 dark:text-white mb-1">Rate Limits</h2>
        <p className="text-xs text-zinc-500 mb-4">
          Default rate limits applied to newly created API keys.
        </p>
        <div className="grid grid-cols-1 md:grid-cols-2 gap-2 md:gap-3">
          <NumberField
            label="Default API Key Rate Limit"
            description="Requests per minute for new API keys."
            value={form.default_rate_limit_rpm}
            onChange={(v) => updateField("default_rate_limit_rpm", v)}
            min={1}
            suffix="req/min"
            error={errors.default_rate_limit_rpm}
          />
        </div>
      </div>
    </div>
  );
}
