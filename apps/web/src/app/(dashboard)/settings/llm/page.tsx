"use client";

import { useState, useEffect } from "react";
import { toast } from "sonner";
import {
  useLlmSettings,
  useUpdateLlmSettings,
  useDeleteLlmSettings,
} from "@/hooks/use-settings";

const PROVIDERS = [
  {
    id: "openai",
    label: "OpenAI",
    defaultUrl: "https://api.openai.com/v1",
    models: ["gpt-4o-mini", "gpt-4o", "gpt-4-turbo", "gpt-3.5-turbo"],
  },
  {
    id: "groq",
    label: "Groq",
    defaultUrl: "https://api.groq.com/openai/v1",
    models: [
      "llama-3.1-70b-versatile",
      "llama-3.1-8b-instant",
      "mixtral-8x7b-32768",
    ],
  },
  {
    id: "together",
    label: "Together AI",
    defaultUrl: "https://api.together.xyz/v1",
    models: [
      "meta-llama/Llama-3.1-70B-Instruct",
      "meta-llama/Llama-3.1-8B-Instruct",
    ],
  },
  {
    id: "ollama",
    label: "Ollama (Local)",
    defaultUrl: "http://localhost:11434/v1",
    models: ["llama3.1", "mistral", "phi3"],
  },
  {
    id: "custom",
    label: "Custom (OpenAI-compatible)",
    defaultUrl: "",
    models: [],
  },
];

interface FormState {
  provider: string;
  api_base_url: string;
  api_key: string;
  model: string;
  max_tokens: number;
}

const DEFAULT_FORM: FormState = {
  provider: "openai",
  api_base_url: "https://api.openai.com/v1",
  api_key: "",
  model: "gpt-4o-mini",
  max_tokens: 2000,
};

export default function LlmSettingsPage() {
  const { data: settings, isLoading } = useLlmSettings();
  const updateSettings = useUpdateLlmSettings();
  const deleteSettings = useDeleteLlmSettings();

  useEffect(() => {
    if (updateSettings.isSuccess) toast.success("LLM settings saved");
  }, [updateSettings.isSuccess]);
  useEffect(() => {
    if (updateSettings.isError) toast.error(updateSettings.error.message);
  }, [updateSettings.isError, updateSettings.error]);
  useEffect(() => {
    if (deleteSettings.isSuccess)
      toast.success("LLM settings reset to defaults");
  }, [deleteSettings.isSuccess]);

  const [form, setForm] = useState<FormState>(DEFAULT_FORM);
  const [hasChanges, setHasChanges] = useState(false);
  const [showApiKey, setShowApiKey] = useState(false);
  const [showDeleteConfirm, setShowDeleteConfirm] = useState(false);

  // Sync server state into local form
  useEffect(() => {
    if (!settings) return;
    if (settings.is_configured) {
      setForm({
        provider: settings.provider ?? "openai",
        api_base_url: settings.api_base_url ?? "",
        api_key: "", // Never pre-fill — masked key is display-only
        model: settings.model ?? "",
        max_tokens: settings.max_tokens ?? 2000,
      });
    }
  }, [settings]);

  const selectedProvider = PROVIDERS.find((p) => p.id === form.provider);

  const handleProviderChange = (providerId: string) => {
    const provider = PROVIDERS.find((p) => p.id === providerId);
    setForm((prev) => ({
      ...prev,
      provider: providerId,
      api_base_url: provider?.defaultUrl ?? prev.api_base_url,
      model: provider?.models[0] ?? prev.model,
    }));
    setHasChanges(true);
  };

  const updateField = (field: keyof FormState, value: string | number) => {
    setForm((prev) => ({ ...prev, [field]: value }));
    setHasChanges(true);
  };

  const handleSave = () => {
    const payload: Record<string, string | number> = {
      provider: form.provider,
      api_base_url: form.api_base_url,
      model: form.model,
      max_tokens: form.max_tokens,
    };
    // Only send api_key if user typed a new one
    if (form.api_key) {
      payload.api_key = form.api_key;
    }
    updateSettings.mutate(payload, {
      onSuccess: () => {
        setHasChanges(false);
        setForm((prev) => ({ ...prev, api_key: "" }));
      },
    });
  };

  const handleDelete = () => {
    deleteSettings.mutate(undefined, {
      onSuccess: () => {
        setForm(DEFAULT_FORM);
        setHasChanges(false);
        setShowDeleteConfirm(false);
      },
    });
  };

  return (
    <div className="max-w-4xl">
      <h1 className="text-2xl font-bold text-zinc-900 dark:text-white mb-2">LLM Provider</h1>
      <p className="text-zinc-600 dark:text-zinc-400 mb-8">
        Configure which LLM provider is used for synthetic data generation,
        evaluation judging, and training reward scoring. If not configured, the
        platform&apos;s default provider is used.
      </p>

      {isLoading ? (
        <div className="border border-zinc-200 dark:border-zinc-800 rounded-lg p-8 text-center text-zinc-500">
          Loading settings...
        </div>
      ) : (
        <>
          {/* Current Status */}
          <div className="border border-zinc-200 dark:border-zinc-800 rounded-lg p-4 mb-6">
            <div className="flex items-center gap-3">
              <div
                className={`w-2.5 h-2.5 rounded-full ${
                  settings?.is_configured ? "bg-emerald-500" : "bg-zinc-600"
                }`}
              />
              <span className="text-sm text-zinc-900 dark:text-white">
                {settings?.is_configured ? (
                  <>
                    Custom provider configured
                    {settings.provider && (
                      <span className="text-zinc-600 dark:text-zinc-400">
                        {" "}
                        &mdash;{" "}
                        {PROVIDERS.find((p) => p.id === settings.provider)
                          ?.label ?? settings.provider}
                      </span>
                    )}
                    {settings.api_key_masked && (
                      <span className="text-zinc-500 ml-2 font-mono text-xs">
                        ({settings.api_key_masked})
                      </span>
                    )}
                  </>
                ) : (
                  <span className="text-zinc-600 dark:text-zinc-400">
                    Using platform defaults &mdash; configure your own provider
                    below
                  </span>
                )}
              </span>
            </div>
          </div>

          {/* Configuration Form */}
          <div className="border border-zinc-200 dark:border-zinc-800 rounded-lg">
            <div className="p-4 border-b border-zinc-200 dark:border-zinc-800">
              <h2 className="text-lg font-semibold text-zinc-900 dark:text-white">
                Configuration
              </h2>
            </div>

            <div className="p-6 space-y-6">
              {/* Provider */}
              <div>
                <label className="block text-sm text-zinc-600 dark:text-zinc-400 mb-2">
                  Provider
                </label>
                <select
                  value={form.provider}
                  onChange={(e) => handleProviderChange(e.target.value)}
                  className="w-full bg-zinc-50 dark:bg-zinc-900 border border-zinc-300 dark:border-zinc-700 rounded-md px-3 py-2 text-zinc-900 dark:text-white focus:outline-none focus:ring-1 focus:ring-emerald-500"
                >
                  {PROVIDERS.map((p) => (
                    <option key={p.id} value={p.id}>
                      {p.label}
                    </option>
                  ))}
                </select>
              </div>

              {/* API Base URL */}
              <div>
                <label className="block text-sm text-zinc-600 dark:text-zinc-400 mb-2">
                  API Base URL
                </label>
                <input
                  type="url"
                  value={form.api_base_url}
                  onChange={(e) => updateField("api_base_url", e.target.value)}
                  placeholder="https://api.openai.com/v1"
                  className="w-full bg-zinc-50 dark:bg-zinc-900 border border-zinc-300 dark:border-zinc-700 rounded-md px-3 py-2 text-zinc-900 dark:text-white placeholder:text-zinc-400 dark:placeholder:text-zinc-500 focus:outline-none focus:ring-1 focus:ring-emerald-500 text-sm font-mono"
                />
                <p className="text-xs text-zinc-500 mt-1">
                  Must be an OpenAI-compatible API endpoint.
                </p>
              </div>

              {/* API Key */}
              <div>
                <label className="block text-sm text-zinc-600 dark:text-zinc-400 mb-2">
                  API Key
                </label>
                <div className="relative">
                  <input
                    type={showApiKey ? "text" : "password"}
                    value={form.api_key}
                    onChange={(e) => updateField("api_key", e.target.value)}
                    placeholder={
                      settings?.api_key_masked
                        ? `Current: ${settings.api_key_masked} (leave blank to keep)`
                        : "sk-..."
                    }
                    className="w-full bg-zinc-50 dark:bg-zinc-900 border border-zinc-300 dark:border-zinc-700 rounded-md px-3 py-2 pr-16 text-zinc-900 dark:text-white placeholder:text-zinc-400 dark:placeholder:text-zinc-500 focus:outline-none focus:ring-1 focus:ring-emerald-500 text-sm font-mono"
                  />
                  <button
                    type="button"
                    onClick={() => setShowApiKey(!showApiKey)}
                    className="absolute right-2 top-1/2 -translate-y-1/2 text-xs text-zinc-600 dark:text-zinc-400 hover:text-zinc-800 dark:hover:text-zinc-200 transition"
                  >
                    {showApiKey ? "Hide" : "Show"}
                  </button>
                </div>
                <p className="text-xs text-zinc-500 mt-1">
                  Your API key is stored securely and never returned in full.
                  Only a masked version is displayed.
                </p>
              </div>

              {/* Model */}
              <div>
                <label className="block text-sm text-zinc-600 dark:text-zinc-400 mb-2">
                  Model
                </label>
                {selectedProvider && selectedProvider.models.length > 0 ? (
                  <div className="flex gap-2">
                    <select
                      value={
                        selectedProvider.models.includes(form.model)
                          ? form.model
                          : "__custom__"
                      }
                      onChange={(e) => {
                        if (e.target.value !== "__custom__") {
                          updateField("model", e.target.value);
                        }
                      }}
                      className="flex-1 bg-zinc-50 dark:bg-zinc-900 border border-zinc-300 dark:border-zinc-700 rounded-md px-3 py-2 text-zinc-900 dark:text-white focus:outline-none focus:ring-1 focus:ring-emerald-500"
                    >
                      {selectedProvider.models.map((m) => (
                        <option key={m} value={m}>
                          {m}
                        </option>
                      ))}
                      {!selectedProvider.models.includes(form.model) && (
                        <option value="__custom__">Custom: {form.model}</option>
                      )}
                      <option value="__custom__">Other (type below)</option>
                    </select>
                    {(!selectedProvider.models.includes(form.model) ||
                      form.model === "") && (
                      <input
                        type="text"
                        value={form.model}
                        onChange={(e) => updateField("model", e.target.value)}
                        placeholder="model-name"
                        className="flex-1 bg-zinc-50 dark:bg-zinc-900 border border-zinc-300 dark:border-zinc-700 rounded-md px-3 py-2 text-zinc-900 dark:text-white placeholder:text-zinc-400 dark:placeholder:text-zinc-500 focus:outline-none focus:ring-1 focus:ring-emerald-500 text-sm font-mono"
                      />
                    )}
                  </div>
                ) : (
                  <input
                    type="text"
                    value={form.model}
                    onChange={(e) => updateField("model", e.target.value)}
                    placeholder="model-name"
                    className="w-full bg-zinc-50 dark:bg-zinc-900 border border-zinc-300 dark:border-zinc-700 rounded-md px-3 py-2 text-zinc-900 dark:text-white placeholder:text-zinc-400 dark:placeholder:text-zinc-500 focus:outline-none focus:ring-1 focus:ring-emerald-500 text-sm font-mono"
                  />
                )}
              </div>

              {/* Max Tokens */}
              <div>
                <label className="block text-sm text-zinc-600 dark:text-zinc-400 mb-2">
                  Max Tokens
                </label>
                <input
                  type="number"
                  value={form.max_tokens}
                  onChange={(e) =>
                    updateField("max_tokens", parseInt(e.target.value) || 0)
                  }
                  min={100}
                  max={128000}
                  className="w-48 bg-zinc-50 dark:bg-zinc-900 border border-zinc-300 dark:border-zinc-700 rounded-md px-3 py-2 text-zinc-900 dark:text-white focus:outline-none focus:ring-1 focus:ring-emerald-500 text-sm"
                />
                <p className="text-xs text-zinc-500 mt-1">
                  Maximum tokens per LLM call for data generation and evaluation
                  judging.
                </p>
              </div>
            </div>

            {/* Actions */}
            <div className="px-6 py-4 border-t border-zinc-200 dark:border-zinc-800 flex items-center gap-3">
              <button
                onClick={handleSave}
                disabled={!hasChanges || updateSettings.isPending}
                className="bg-emerald-600 hover:bg-emerald-500 disabled:opacity-50 disabled:cursor-not-allowed text-white px-4 py-2 rounded-md text-sm font-medium transition"
              >
                {updateSettings.isPending ? "Saving..." : "Save Configuration"}
              </button>

              {settings?.is_configured && (
                <>
                  {showDeleteConfirm ? (
                    <div className="flex items-center gap-2">
                      <span className="text-sm text-zinc-600 dark:text-zinc-400">
                        Reset to platform defaults?
                      </span>
                      <button
                        onClick={handleDelete}
                        disabled={deleteSettings.isPending}
                        className="text-red-400 hover:text-red-300 text-sm font-medium transition"
                      >
                        {deleteSettings.isPending ? "Resetting..." : "Confirm"}
                      </button>
                      <button
                        onClick={() => setShowDeleteConfirm(false)}
                        className="text-zinc-600 dark:text-zinc-400 hover:text-zinc-800 dark:hover:text-zinc-200 text-sm transition"
                      >
                        Cancel
                      </button>
                    </div>
                  ) : (
                    <button
                      onClick={() => setShowDeleteConfirm(true)}
                      className="text-red-400 hover:text-red-300 text-sm transition"
                    >
                      Reset to Defaults
                    </button>
                  )}
                </>
              )}

              {updateSettings.isError && (
                <p className="text-red-400 text-sm">
                  {updateSettings.error.message}
                </p>
              )}
              {deleteSettings.isError && (
                <p className="text-red-400 text-sm">
                  {deleteSettings.error.message}
                </p>
              )}
              {updateSettings.isSuccess && !hasChanges && (
                <p className="text-emerald-400 text-sm">Saved.</p>
              )}
            </div>
          </div>

          {/* How It Works */}
          <div className="border border-zinc-200 dark:border-zinc-800 rounded-lg mt-6 p-6">
            <h3 className="text-sm font-semibold text-zinc-900 dark:text-white mb-3">
              How it works
            </h3>
            <ul className="space-y-2 text-sm text-zinc-600 dark:text-zinc-400">
              <li className="flex items-start gap-2">
                <span className="text-zinc-400 dark:text-zinc-600 mt-0.5">1.</span>
                When configured, your LLM provider is used for synthetic data
                generation, evaluation judging, and training reward scoring.
              </li>
              <li className="flex items-start gap-2">
                <span className="text-zinc-400 dark:text-zinc-600 mt-0.5">2.</span>
                API calls are made directly from the workers to your provider
                &mdash; the platform never proxies or stores your LLM traffic.
              </li>
              <li className="flex items-start gap-2">
                <span className="text-zinc-400 dark:text-zinc-600 mt-0.5">3.</span>
                If no custom provider is configured, the platform&apos;s default
                LLM provider is used (usage may be subject to platform billing).
              </li>
              <li className="flex items-start gap-2">
                <span className="text-zinc-400 dark:text-zinc-600 mt-0.5">4.</span>
                Any OpenAI-compatible API works (OpenAI, Groq, Together AI,
                Ollama, vLLM, etc.).
              </li>
            </ul>
          </div>
        </>
      )}
    </div>
  );
}
