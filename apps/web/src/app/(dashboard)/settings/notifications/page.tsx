"use client";

import { useState, useEffect, useCallback } from "react";
import { toast } from "sonner";
import {
  useNotificationPreferences,
  useUpdatePreferences,
  useDeliveryHistory,
  useTestWebhook,
  useRetryDelivery,
} from "@/hooks/use-notifications";

const EVENT_TYPES = [
  { id: "training_complete", label: "Training Complete" },
  { id: "evaluation_complete", label: "Evaluation Complete" },
  { id: "deployment_status", label: "Deployment Status" },
  { id: "invitation", label: "Invitation" },
];

const CHANNELS = [
  { id: "email", label: "Email" },
  { id: "webhook", label: "Webhook" },
];

interface PreferenceState {
  [key: string]: { enabled: boolean; config: Record<string, unknown> };
}

function buildKey(channel: string, eventType: string) {
  return `${channel}:${eventType}`;
}

export default function NotificationsSettingsPage() {
  const { data: preferences, isLoading: prefsLoading } =
    useNotificationPreferences();
  const updatePreferences = useUpdatePreferences();
  const { data: deliveries, isLoading: deliveriesLoading } =
    useDeliveryHistory();
  const testWebhook = useTestWebhook();
  const retryDelivery = useRetryDelivery();

  useEffect(() => {
    if (testWebhook.isSuccess) {
      const status = testWebhook.data?.status;
      if (status === "sent")
        toast.success("Webhook test delivered successfully");
      else
        toast.error(
          `Webhook test failed: ${testWebhook.data?.last_error ?? "unknown error"}`,
        );
    }
  }, [testWebhook.isSuccess, testWebhook.data]);
  useEffect(() => {
    if (testWebhook.isError) toast.error(testWebhook.error.message);
  }, [testWebhook.isError, testWebhook.error]);
  useEffect(() => {
    if (retryDelivery.isSuccess) {
      const status = retryDelivery.data?.status;
      if (status === "sent") toast.success("Delivery retry succeeded");
      else
        toast.error(
          `Retry failed: ${retryDelivery.data?.last_error ?? "unknown error"}`,
        );
    }
  }, [retryDelivery.isSuccess, retryDelivery.data]);
  useEffect(() => {
    if (retryDelivery.isError) toast.error(retryDelivery.error.message);
  }, [retryDelivery.isError, retryDelivery.error]);

  useEffect(() => {
    if (updatePreferences.isSuccess)
      toast.success("Notification preferences saved");
  }, [updatePreferences.isSuccess]);
  useEffect(() => {
    if (updatePreferences.isError) toast.error(updatePreferences.error.message);
  }, [updatePreferences.isError, updatePreferences.error]);

  const [localPrefs, setLocalPrefs] = useState<PreferenceState>({});
  const [webhookUrl, setWebhookUrl] = useState("");
  const [hasChanges, setHasChanges] = useState(false);

  // Sync server preferences into local state
  useEffect(() => {
    if (!preferences) return;
    const map: PreferenceState = {};
    for (const pref of preferences) {
      map[buildKey(pref.channel, pref.event_type)] = {
        enabled: pref.enabled,
        config: pref.config,
      };
    }
    setLocalPrefs(map);

    // Extract webhook URL from any webhook preference config
    const webhookPref = preferences.find(
      (p) => p.channel === "webhook" && p.config?.url,
    );
    if (webhookPref?.config?.url) {
      setWebhookUrl(webhookPref.config.url as string);
    }
  }, [preferences]);

  const isEnabled = useCallback(
    (channel: string, eventType: string) => {
      return localPrefs[buildKey(channel, eventType)]?.enabled ?? false;
    },
    [localPrefs],
  );

  const togglePref = (channel: string, eventType: string) => {
    const key = buildKey(channel, eventType);
    setLocalPrefs((prev) => ({
      ...prev,
      [key]: {
        enabled: !(prev[key]?.enabled ?? false),
        config: prev[key]?.config ?? {},
      },
    }));
    setHasChanges(true);
  };

  const hasWebhookEnabled = EVENT_TYPES.some((et) =>
    isEnabled("webhook", et.id),
  );

  const handleSave = () => {
    const prefs: Array<{
      channel: string;
      event_type: string;
      enabled: boolean;
      config?: Record<string, unknown>;
    }> = [];

    for (const channel of CHANNELS) {
      for (const eventType of EVENT_TYPES) {
        const key = buildKey(channel.id, eventType.id);
        const entry = localPrefs[key];
        const config: Record<string, unknown> =
          channel.id === "webhook" && webhookUrl ? { url: webhookUrl } : {};

        prefs.push({
          channel: channel.id,
          event_type: eventType.id,
          enabled: entry?.enabled ?? false,
          config,
        });
      }
    }

    updatePreferences.mutate(
      { preferences: prefs },
      { onSuccess: () => setHasChanges(false) },
    );
  };

  return (
    <div className="max-w-4xl">
      <h1 className="text-xl md:text-2xl font-bold text-zinc-900 dark:text-white mb-2">Notifications</h1>
      <p className="text-zinc-600 dark:text-zinc-400 mb-8">
        Choose which events trigger notifications and how they are delivered.
      </p>

      {/* Preference Toggles */}
      <div className="border border-zinc-200 dark:border-zinc-800 rounded-lg mb-8">
        <div className="p-4 border-b border-zinc-200 dark:border-zinc-800">
          <h2 className="text-lg font-semibold text-zinc-900 dark:text-white">Preferences</h2>
        </div>

        {prefsLoading ? (
          <div className="p-8 text-center text-zinc-500">Loading...</div>
        ) : (
          <>
            {/* Table Header */}
            <div className="grid grid-cols-[1fr,repeat(2,80px)] sm:grid-cols-[1fr,repeat(2,100px)] gap-2 px-3 sm:px-4 py-3 border-b border-zinc-200 dark:border-zinc-800">
              <span className="text-xs text-zinc-500 uppercase tracking-wide">
                Event
              </span>
              {CHANNELS.map((ch) => (
                <span
                  key={ch.id}
                  className="text-xs text-zinc-500 uppercase tracking-wide text-center"
                >
                  {ch.label}
                </span>
              ))}
            </div>

            {/* Rows */}
            {EVENT_TYPES.map((et) => (
              <div
                key={et.id}
                className="grid grid-cols-[1fr,repeat(2,80px)] sm:grid-cols-[1fr,repeat(2,100px)] gap-2 px-3 sm:px-4 py-3 border-b border-zinc-200 dark:border-zinc-800 last:border-b-0"
              >
                <span className="text-sm text-zinc-900 dark:text-white">{et.label}</span>
                {CHANNELS.map((ch) => (
                  <div key={ch.id} className="flex justify-center">
                    <button
                      type="button"
                      role="switch"
                      aria-checked={isEnabled(ch.id, et.id)}
                      onClick={() => togglePref(ch.id, et.id)}
                      className={`relative inline-flex h-5 w-9 items-center rounded-full transition-colors ${
                        isEnabled(ch.id, et.id)
                          ? "bg-emerald-600"
                          : "bg-zinc-700"
                      }`}
                    >
                      <span
                        className={`inline-block h-3.5 w-3.5 transform rounded-full bg-white transition-transform ${
                          isEnabled(ch.id, et.id)
                            ? "translate-x-4"
                            : "translate-x-0.5"
                        }`}
                      />
                    </button>
                  </div>
                ))}
              </div>
            ))}

            {/* Webhook URL */}
            {hasWebhookEnabled && (
              <div className="px-4 py-4 border-t border-zinc-200 dark:border-zinc-800">
                <label className="block text-sm text-zinc-600 dark:text-zinc-400 mb-2">
                  Webhook URL
                </label>
                <div className="flex gap-2">
                  <input
                    type="url"
                    placeholder="https://example.com/webhook"
                    value={webhookUrl}
                    onChange={(e) => {
                      setWebhookUrl(e.target.value);
                      setHasChanges(true);
                    }}
                    className="flex-1 bg-zinc-50 dark:bg-zinc-900 border border-zinc-300 dark:border-zinc-700 rounded-md px-3 py-2 text-zinc-900 dark:text-white placeholder:text-zinc-400 dark:placeholder:text-zinc-500 focus:outline-none focus:ring-1 focus:ring-emerald-500 text-sm"
                  />
                  {(() => {
                    const webhookPref = preferences?.find(
                      (p) => p.channel === "webhook" && p.config?.url,
                    );
                    if (!webhookPref || hasChanges) return null;
                    return (
                      <button
                        onClick={() => testWebhook.mutate(webhookPref.id)}
                        disabled={testWebhook.isPending}
                        className="rounded-md bg-zinc-200 dark:bg-zinc-700 px-3 py-2 text-sm font-medium text-zinc-700 dark:text-zinc-300 hover:bg-zinc-300 dark:hover:bg-zinc-600 transition disabled:opacity-50 whitespace-nowrap"
                      >
                        {testWebhook.isPending ? "Testing..." : "Test Webhook"}
                      </button>
                    );
                  })()}
                </div>
              </div>
            )}

            {/* Save Button */}
            <div className="px-4 py-4 border-t border-zinc-200 dark:border-zinc-800 flex items-center gap-3">
              <button
                onClick={handleSave}
                disabled={!hasChanges || updatePreferences.isPending}
                className="bg-emerald-600 hover:bg-emerald-500 disabled:opacity-50 disabled:cursor-not-allowed text-white px-4 py-2 rounded-md text-sm font-medium transition"
              >
                {updatePreferences.isPending ? "Saving..." : "Save Preferences"}
              </button>
              {updatePreferences.isError && (
                <p className="text-red-400 text-sm">
                  {updatePreferences.error.message}
                </p>
              )}
              {updatePreferences.isSuccess && !hasChanges && (
                <p className="text-emerald-400 text-sm">Saved.</p>
              )}
            </div>
          </>
        )}
      </div>

      {/* Delivery History */}
      <div className="border border-zinc-200 dark:border-zinc-800 rounded-lg">
        <div className="p-4 border-b border-zinc-200 dark:border-zinc-800">
          <h2 className="text-lg font-semibold text-zinc-900 dark:text-white">Delivery History</h2>
        </div>

        {deliveriesLoading ? (
          <div className="p-8 text-center text-zinc-500">Loading...</div>
        ) : !deliveries?.data?.length ? (
          <div className="p-8 text-center text-zinc-500">
            No deliveries yet.
          </div>
        ) : (
          <div className="overflow-x-auto">
            <table className="w-full text-sm">
              <thead>
                <tr className="border-b border-zinc-200 dark:border-zinc-800">
                  <th className="px-4 py-3 text-left text-xs text-zinc-500 uppercase tracking-wide font-medium">
                    Event
                  </th>
                  <th className="px-4 py-3 text-left text-xs text-zinc-500 uppercase tracking-wide font-medium">
                    Channel
                  </th>
                  <th className="px-4 py-3 text-left text-xs text-zinc-500 uppercase tracking-wide font-medium">
                    Status
                  </th>
                  <th className="px-4 py-3 text-left text-xs text-zinc-500 uppercase tracking-wide font-medium">
                    Attempts
                  </th>
                  <th className="px-4 py-3 text-left text-xs text-zinc-500 uppercase tracking-wide font-medium">
                    Time
                  </th>
                  <th className="px-4 py-3 text-left text-xs text-zinc-500 uppercase tracking-wide font-medium">
                    Actions
                  </th>
                </tr>
              </thead>
              <tbody className="divide-y divide-zinc-200 dark:divide-zinc-800">
                {deliveries.data.map(
                  (d: {
                    id: string;
                    event_type: string;
                    channel: string;
                    status: string;
                    attempts: number;
                    last_error: string | null;
                    created_at: string;
                    sent_at: string | null;
                  }) => (
                    <tr key={d.id}>
                      <td className="px-4 py-3 text-zinc-900 dark:text-white whitespace-nowrap">
                        {formatEventType(d.event_type)}
                      </td>
                      <td className="px-4 py-3 text-zinc-600 dark:text-zinc-400 capitalize whitespace-nowrap">
                        {d.channel}
                      </td>
                      <td className="px-4 py-3 whitespace-nowrap">
                        <span className="flex items-center gap-1.5">
                          <StatusBadge status={d.status} />
                          {d.status === "failed" && d.last_error && (
                            <span
                              className="text-xs text-red-400/70 truncate max-w-[200px]"
                              title={d.last_error}
                            >
                              {d.last_error}
                            </span>
                          )}
                        </span>
                      </td>
                      <td className="px-4 py-3 text-zinc-600 dark:text-zinc-400 whitespace-nowrap">
                        {d.attempts}
                      </td>
                      <td className="px-4 py-3 text-zinc-500 whitespace-nowrap text-xs">
                        {d.sent_at
                          ? new Date(d.sent_at).toLocaleString()
                          : new Date(d.created_at).toLocaleString()}
                      </td>
                      <td className="px-4 py-3 whitespace-nowrap">
                        {d.status === "failed" && (
                          <button
                            onClick={() => retryDelivery.mutate(d.id)}
                            disabled={retryDelivery.isPending}
                            className="text-xs text-violet-400 hover:text-violet-300 transition disabled:opacity-50"
                          >
                            {retryDelivery.isPending ? "Retrying..." : "Retry"}
                          </button>
                        )}
                      </td>
                    </tr>
                  ),
                )}
              </tbody>
            </table>
          </div>
        )}
      </div>
    </div>
  );
}

function formatEventType(eventType: string): string {
  return eventType
    .split("_")
    .map((w) => w.charAt(0).toUpperCase() + w.slice(1))
    .join(" ");
}

function StatusBadge({ status }: { status: string }) {
  let classes = "text-xs px-2 py-0.5 rounded ";
  switch (status) {
    case "sent":
      classes += "bg-emerald-50 text-emerald-600 dark:bg-emerald-500/10 dark:text-emerald-400";
      break;
    case "failed":
      classes += "bg-red-50 text-red-600 dark:bg-red-500/10 dark:text-red-400";
      break;
    case "pending":
      classes += "bg-amber-50 text-amber-600 dark:bg-amber-500/10 dark:text-amber-400";
      break;
    default:
      classes += "bg-zinc-200 dark:bg-zinc-700 text-zinc-600 dark:text-zinc-400";
  }
  return <span className={classes}>{status}</span>;
}
