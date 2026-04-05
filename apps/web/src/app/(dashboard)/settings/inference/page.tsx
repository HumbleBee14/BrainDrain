"use client";

import { useEffect, useState } from "react";
import { toast } from "sonner";
import {
  useInferenceInstances,
  useRegisterInstance,
  useUpdateInstanceLifecycle,
  useDeleteInstance,
} from "@/hooks/use-inference-instances";
import type { InferenceInstanceResponse } from "@/lib/api-client";

export default function InferenceInstancesPage() {
  const { data: instances, isLoading } = useInferenceInstances();
  const registerMutation = useRegisterInstance();
  const lifecycleMutation = useUpdateInstanceLifecycle();
  const deleteMutation = useDeleteInstance();

  const [showForm, setShowForm] = useState(false);
  const [form, setForm] = useState({
    name: "",
    base_url: "",
    backend_type: "vllm",
    gpu_class: "",
    base_model: "",
    max_adapters: 4,
  });

  useEffect(() => {
    if (registerMutation.isSuccess) {
      toast.success("Instance registered");
      setShowForm(false);
      setForm({
        name: "",
        base_url: "",
        backend_type: "vllm",
        gpu_class: "",
        base_model: "",
        max_adapters: 4,
      });
    }
  }, [registerMutation.isSuccess]);

  useEffect(() => {
    if (registerMutation.isError)
      toast.error(registerMutation.error?.message ?? "Failed to register");
  }, [registerMutation.isError, registerMutation.error]);

  useEffect(() => {
    if (lifecycleMutation.isSuccess) toast.success("Lifecycle updated");
  }, [lifecycleMutation.isSuccess]);

  useEffect(() => {
    if (lifecycleMutation.isError)
      toast.error(lifecycleMutation.error?.message ?? "Failed to update");
  }, [lifecycleMutation.isError, lifecycleMutation.error]);

  useEffect(() => {
    if (deleteMutation.isSuccess) toast.success("Instance deleted");
  }, [deleteMutation.isSuccess]);

  useEffect(() => {
    if (deleteMutation.isError)
      toast.error(deleteMutation.error?.message ?? "Failed to delete");
  }, [deleteMutation.isError, deleteMutation.error]);

  function handleRegister() {
    registerMutation.mutate({
      name: form.name,
      base_url: form.base_url,
      backend_type: form.backend_type,
      gpu_class: form.gpu_class || null,
      base_model: form.base_model,
      max_adapters: form.max_adapters,
      metadata: {},
    });
  }

  function handleLifecycle(
    id: string,
    lifecycle_state: "ready" | "draining" | "retired",
  ) {
    lifecycleMutation.mutate({ id, lifecycle_state });
  }

  function handleDelete(id: string, name: string) {
    if (!confirm(`Delete instance "${name}"? This cannot be undone.`)) return;
    deleteMutation.mutate({ id });
  }

  return (
    <div>
      <div className="flex items-center justify-between mb-6">
        <div>
          <h2 className="text-xl font-bold text-zinc-900 dark:text-white">
            Inference Instances
          </h2>
          <p className="text-sm text-zinc-500 mt-1">
            Manage GPU serving instances for model deployment and inference.
          </p>
        </div>
        <button
          onClick={() => setShowForm(!showForm)}
          className="px-4 py-2 text-sm font-medium rounded-md bg-emerald-600 text-white hover:bg-emerald-500 transition"
        >
          {showForm ? "Cancel" : "Register Instance"}
        </button>
      </div>

      {showForm && (
        <RegisterForm
          form={form}
          setForm={setForm}
          onSubmit={handleRegister}
          isPending={registerMutation.isPending}
        />
      )}

      {isLoading && (
        <p className="text-sm text-zinc-500">Loading instances...</p>
      )}

      {instances && instances.length === 0 && !isLoading && (
        <div className="text-center py-12 text-zinc-500">
          <p className="text-lg font-medium">No inference instances registered</p>
          <p className="text-sm mt-1">
            Register a GPU server to enable multi-instance deployments.
          </p>
        </div>
      )}

      {instances && instances.length > 0 && (
        <div className="space-y-4">
          {instances.map((instance) => (
            <InstanceCard
              key={instance.id}
              instance={instance}
              onLifecycle={handleLifecycle}
              onDelete={handleDelete}
              isPending={
                lifecycleMutation.isPending || deleteMutation.isPending
              }
            />
          ))}
        </div>
      )}
    </div>
  );
}

function RegisterForm({
  form,
  setForm,
  onSubmit,
  isPending,
}: {
  form: {
    name: string;
    base_url: string;
    backend_type: string;
    gpu_class: string;
    base_model: string;
    max_adapters: number;
  };
  setForm: (f: typeof form) => void;
  onSubmit: () => void;
  isPending: boolean;
}) {
  const valid =
    form.name.trim() && form.base_url.trim() && form.base_model.trim();

  return (
    <div className="mb-6 p-4 rounded-lg border border-zinc-200 dark:border-zinc-800 bg-zinc-50 dark:bg-zinc-900">
      <h3 className="text-sm font-semibold mb-3 text-zinc-900 dark:text-white">
        Register New Instance
      </h3>
      <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
        <Field
          label="Name"
          value={form.name}
          onChange={(v) => setForm({ ...form, name: v })}
          placeholder="gpu-a10g-1"
        />
        <Field
          label="Base URL"
          value={form.base_url}
          onChange={(v) => setForm({ ...form, base_url: v })}
          placeholder="http://vllm-1:8080"
        />
        <div>
          <label className="block text-xs font-medium text-zinc-700 dark:text-zinc-300 mb-1">
            Backend Type
          </label>
          <select
            value={form.backend_type}
            onChange={(e) =>
              setForm({ ...form, backend_type: e.target.value })
            }
            className="w-full px-3 py-2 text-sm rounded-md border border-zinc-300 dark:border-zinc-700 bg-white dark:bg-zinc-800 text-zinc-900 dark:text-white"
          >
            <option value="vllm">vLLM</option>
            <option value="tgi">TGI</option>
            <option value="sglang">SGLang</option>
          </select>
        </div>
        <Field
          label="GPU Class (optional)"
          value={form.gpu_class}
          onChange={(v) => setForm({ ...form, gpu_class: v })}
          placeholder="a10g"
        />
        <Field
          label="Base Model"
          value={form.base_model}
          onChange={(v) => setForm({ ...form, base_model: v })}
          placeholder="meta-llama/Llama-3.1-8B"
        />
        <div>
          <label className="block text-xs font-medium text-zinc-700 dark:text-zinc-300 mb-1">
            Max Adapters
          </label>
          <input
            type="number"
            min={1}
            value={form.max_adapters}
            onChange={(e) =>
              setForm({ ...form, max_adapters: parseInt(e.target.value) || 1 })
            }
            className="w-full px-3 py-2 text-sm rounded-md border border-zinc-300 dark:border-zinc-700 bg-white dark:bg-zinc-800 text-zinc-900 dark:text-white"
          />
        </div>
      </div>
      <div className="mt-4 flex justify-end">
        <button
          onClick={onSubmit}
          disabled={!valid || isPending}
          className="px-4 py-2 text-sm font-medium rounded-md bg-emerald-600 text-white hover:bg-emerald-500 disabled:opacity-50 disabled:cursor-not-allowed transition"
        >
          {isPending ? "Registering..." : "Register"}
        </button>
      </div>
    </div>
  );
}

function Field({
  label,
  value,
  onChange,
  placeholder,
}: {
  label: string;
  value: string;
  onChange: (v: string) => void;
  placeholder?: string;
}) {
  return (
    <div>
      <label className="block text-xs font-medium text-zinc-700 dark:text-zinc-300 mb-1">
        {label}
      </label>
      <input
        type="text"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={placeholder}
        className="w-full px-3 py-2 text-sm rounded-md border border-zinc-300 dark:border-zinc-700 bg-white dark:bg-zinc-800 text-zinc-900 dark:text-white placeholder:text-zinc-400"
      />
    </div>
  );
}

function InstanceCard({
  instance,
  onLifecycle,
  onDelete,
  isPending,
}: {
  instance: InferenceInstanceResponse;
  onLifecycle: (
    id: string,
    state: "ready" | "draining" | "retired",
  ) => void;
  onDelete: (id: string, name: string) => void;
  isPending: boolean;
}) {
  const healthColor =
    instance.health_status === "healthy"
      ? "bg-emerald-100 text-emerald-700 dark:bg-emerald-900/30 dark:text-emerald-400"
      : instance.health_status === "unhealthy"
        ? "bg-red-100 text-red-700 dark:bg-red-900/30 dark:text-red-400"
        : "bg-zinc-100 text-zinc-600 dark:bg-zinc-800 dark:text-zinc-400";

  const lifecycleColor =
    instance.lifecycle_state === "ready"
      ? "bg-emerald-100 text-emerald-700 dark:bg-emerald-900/30 dark:text-emerald-400"
      : instance.lifecycle_state === "draining"
        ? "bg-amber-100 text-amber-700 dark:bg-amber-900/30 dark:text-amber-400"
        : "bg-zinc-100 text-zinc-600 dark:bg-zinc-800 dark:text-zinc-400";

  return (
    <div className="p-4 rounded-lg border border-zinc-200 dark:border-zinc-800">
      <div className="flex items-start justify-between">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2 flex-wrap">
            <h3 className="text-sm font-semibold text-zinc-900 dark:text-white">
              {instance.name}
            </h3>
            <span
              className={`inline-block px-2 py-0.5 text-xs font-medium rounded-full ${healthColor}`}
            >
              {instance.health_status}
            </span>
            <span
              className={`inline-block px-2 py-0.5 text-xs font-medium rounded-full ${lifecycleColor}`}
            >
              {instance.lifecycle_state}
            </span>
          </div>
          <p className="text-xs text-zinc-500 mt-1 truncate">
            {instance.base_url}
          </p>
          <div className="flex gap-4 mt-2 text-xs text-zinc-500">
            <span>
              {instance.backend_type.toUpperCase()} &middot;{" "}
              {instance.base_model}
            </span>
            {instance.gpu_class && <span>GPU: {instance.gpu_class}</span>}
            <span>
              Adapters: {instance.active_adapter_count}/{instance.max_adapters}
            </span>
          </div>
          {instance.last_health_check_at && (
            <p className="text-xs text-zinc-400 mt-1">
              Last checked:{" "}
              {new Date(instance.last_health_check_at).toLocaleString()}
            </p>
          )}
        </div>

        <div className="flex gap-2 ml-4 shrink-0">
          {instance.lifecycle_state === "ready" && (
            <button
              onClick={() => onLifecycle(instance.id, "draining")}
              disabled={isPending}
              className="px-3 py-1.5 text-xs font-medium rounded-md border border-amber-300 dark:border-amber-700 text-amber-700 dark:text-amber-400 hover:bg-amber-50 dark:hover:bg-amber-900/20 disabled:opacity-50 transition"
            >
              Drain
            </button>
          )}
          {instance.lifecycle_state === "draining" && (
            <>
              <button
                onClick={() => onLifecycle(instance.id, "ready")}
                disabled={isPending}
                className="px-3 py-1.5 text-xs font-medium rounded-md border border-emerald-300 dark:border-emerald-700 text-emerald-700 dark:text-emerald-400 hover:bg-emerald-50 dark:hover:bg-emerald-900/20 disabled:opacity-50 transition"
              >
                Resume
              </button>
              <button
                onClick={() => onLifecycle(instance.id, "retired")}
                disabled={isPending || instance.active_adapter_count > 0}
                className="px-3 py-1.5 text-xs font-medium rounded-md border border-zinc-300 dark:border-zinc-700 text-zinc-600 dark:text-zinc-400 hover:bg-zinc-50 dark:hover:bg-zinc-800 disabled:opacity-50 transition"
              >
                Retire
              </button>
            </>
          )}
          {instance.lifecycle_state === "retired" && (
            <button
              onClick={() => onDelete(instance.id, instance.name)}
              disabled={isPending || instance.active_adapter_count > 0}
              className="px-3 py-1.5 text-xs font-medium rounded-md border border-red-300 dark:border-red-700 text-red-600 dark:text-red-400 hover:bg-red-50 dark:hover:bg-red-900/20 disabled:opacity-50 transition"
            >
              Delete
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
