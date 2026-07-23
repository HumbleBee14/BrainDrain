"use client";

import { useState, useMemo, useCallback } from "react";
import { useAuditLogs } from "@/hooks/use-audit-logs";
import { ErrorState } from "@/components/error-state";

const PAGE_SIZE = 25;

const RESOURCE_TYPES = [
  { id: "", label: "All Resources" },
  { id: "project", label: "Project" },
  { id: "training_job", label: "Training Job" },
  { id: "model", label: "Model" },
  { id: "dataset", label: "Dataset" },
  { id: "evaluation", label: "Evaluation" },
  { id: "api_key", label: "API Key" },
  { id: "team_member", label: "Team Member" },
  { id: "invitation", label: "Invitation" },
  { id: "notification_preference", label: "Notification" },
  { id: "notification_delivery", label: "Delivery" },
];

function formatAction(action: string): string {
  return action.replace(/[._]/g, " ").replace(/\b\w/g, (c) => c.toUpperCase());
}

function formatRelativeTime(dateStr: string): string {
  const date = new Date(dateStr);
  const now = new Date();
  const diffMs = now.getTime() - date.getTime();
  const diffMins = Math.floor(diffMs / 60000);
  const diffHrs = Math.floor(diffMs / 3600000);
  const diffDays = Math.floor(diffMs / 86400000);

  if (diffMins < 1) return "just now";
  if (diffMins < 60) return `${diffMins}m ago`;
  if (diffHrs < 24) return `${diffHrs}h ago`;
  if (diffDays < 7) return `${diffDays}d ago`;
  return date.toLocaleDateString();
}

export default function AuditLogPage() {
  const [page, setPage] = useState(0);
  const [actionFilter, setActionFilter] = useState("");
  const [resourceTypeFilter, setResourceTypeFilter] = useState("");
  const [searchQuery, setSearchQuery] = useState("");

  const { data, isLoading, isError, isFetching, refetch } = useAuditLogs({
    offset: page * PAGE_SIZE,
    limit: PAGE_SIZE,
    action: actionFilter || undefined,
    resource_type: resourceTypeFilter || undefined,
  });

  const totalPages = data ? Math.ceil(data.total / PAGE_SIZE) : 0;

  // Client-side search across action, resource_type, actor_id
  const filteredData = useMemo(() => {
    if (!data?.data || !searchQuery.trim()) return data?.data ?? [];
    const q = searchQuery.toLowerCase();
    return data.data.filter(
      (log) =>
        log.action.toLowerCase().includes(q) ||
        log.resource_type.toLowerCase().includes(q) ||
        log.actor_id.toLowerCase().includes(q) ||
        (log.resource_id && log.resource_id.toLowerCase().includes(q)),
    );
  }, [data?.data, searchQuery]);

  const handleExportCsv = useCallback(() => {
    if (!data?.data?.length) return;

    // Sanitize CSV cell values to prevent formula injection.
    // Spreadsheet apps execute formulas starting with =, +, -, @, tab, or CR.
    const sanitizeCsvCell = (val: string): string => {
      const escaped = val.replace(/"/g, '""');
      if (/^[=+\-@\t\r]/.test(escaped)) {
        return `"'${escaped}"`;
      }
      return `"${escaped}"`;
    };

    const rows = data.data.map((log) => ({
      timestamp: new Date(log.created_at).toISOString(),
      action: log.action,
      resource_type: log.resource_type,
      resource_id: log.resource_id ?? "",
      actor_id: log.actor_id,
      metadata: JSON.stringify(log.metadata),
    }));

    const headers = Object.keys(rows[0]);
    const csv = [
      headers.join(","),
      ...rows.map((row) =>
        headers
          .map((h) => {
            const val = String(row[h as keyof typeof row]);
            return sanitizeCsvCell(val);
          })
          .join(","),
      ),
    ].join("\n");

    const blob = new Blob([csv], { type: "text/csv" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `audit-log-${new Date().toISOString().slice(0, 10)}.csv`;
    a.click();
    URL.revokeObjectURL(url);
  }, [data?.data]);

  const resetFilters = () => {
    setActionFilter("");
    setResourceTypeFilter("");
    setSearchQuery("");
    setPage(0);
  };

  const hasFilters = actionFilter || resourceTypeFilter || searchQuery;

  // Collect unique actions from current data for the action filter dropdown
  const uniqueActions = useMemo(() => {
    if (!data?.data) return [];
    const set = new Set(data.data.map((d) => d.action));
    return Array.from(set).sort();
  }, [data?.data]);

  return (
    <div>
      <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3 mb-6">
        <div>
          <h1 className="text-xl md:text-2xl font-bold text-zinc-900 dark:text-white">Audit Log</h1>
          <p className="text-sm text-zinc-500 mt-1">
            {data
              ? `${data.total.toLocaleString()} total events`
              : "Loading..."}
          </p>
        </div>
        <button
          onClick={handleExportCsv}
          disabled={!data?.data?.length}
          className="rounded-lg bg-zinc-100 dark:bg-zinc-800 px-4 py-2 text-sm font-medium text-zinc-700 dark:text-zinc-300 hover:bg-zinc-200 dark:hover:bg-zinc-700 transition disabled:opacity-50 disabled:cursor-not-allowed"
        >
          Export CSV
        </button>
      </div>

      {/* Filters */}
      <div className="flex flex-wrap gap-3 mb-4">
        <input
          type="text"
          placeholder="Search actions, resources, actors..."
          value={searchQuery}
          onChange={(e) => setSearchQuery(e.target.value)}
          className="flex-1 min-w-[200px] bg-zinc-50 dark:bg-zinc-900 border border-zinc-300 dark:border-zinc-700 rounded-md px-3 py-2 text-sm text-zinc-900 dark:text-white placeholder:text-zinc-400 dark:placeholder:text-zinc-500 focus:outline-none focus:ring-1 focus:ring-emerald-500"
        />
        <select
          value={resourceTypeFilter}
          onChange={(e) => {
            setResourceTypeFilter(e.target.value);
            setPage(0);
          }}
          className="bg-zinc-50 dark:bg-zinc-900 border border-zinc-300 dark:border-zinc-700 rounded-md px-3 py-2 text-sm text-zinc-900 dark:text-white focus:outline-none focus:ring-1 focus:ring-emerald-500"
        >
          {RESOURCE_TYPES.map((rt) => (
            <option key={rt.id} value={rt.id}>
              {rt.label}
            </option>
          ))}
        </select>
        {uniqueActions.length > 0 && (
          <select
            value={actionFilter}
            onChange={(e) => {
              setActionFilter(e.target.value);
              setPage(0);
            }}
            className="bg-zinc-50 dark:bg-zinc-900 border border-zinc-300 dark:border-zinc-700 rounded-md px-3 py-2 text-sm text-zinc-900 dark:text-white focus:outline-none focus:ring-1 focus:ring-emerald-500"
          >
            <option value="">All Actions</option>
            {uniqueActions.map((a) => (
              <option key={a} value={a}>
                {formatAction(a)}
              </option>
            ))}
          </select>
        )}
        {hasFilters && (
          <button
            onClick={resetFilters}
            className="text-sm text-zinc-600 dark:text-zinc-400 hover:text-zinc-900 dark:hover:text-white transition px-2"
          >
            Clear filters
          </button>
        )}
      </div>

      {/* Table */}
      {isError ? (
        <ErrorState
          title="Couldn't load the audit log"
          message="The audit service didn't respond. Check your connection and try again."
          onRetry={() => refetch()}
          isRetrying={isFetching}
        />
      ) : (
      <div className="border border-zinc-200 dark:border-zinc-800 rounded-lg overflow-hidden">
        {isLoading ? (
          <div className="p-12 text-center text-zinc-500">
            Loading audit logs...
          </div>
        ) : !filteredData.length ? (
          <div className="p-12 text-center text-zinc-500">
            {hasFilters
              ? "No audit logs match the current filters."
              : "No audit logs yet."}
          </div>
        ) : (
          <div className="overflow-x-auto">
            <table className="w-full text-sm">
              <thead>
                <tr className="border-b border-zinc-200 dark:border-zinc-800 bg-zinc-50/50 dark:bg-zinc-900/50">
                  <th className="px-4 py-3 text-left text-xs text-zinc-500 uppercase tracking-wide font-medium">
                    Time
                  </th>
                  <th className="px-4 py-3 text-left text-xs text-zinc-500 uppercase tracking-wide font-medium">
                    Action
                  </th>
                  <th className="px-4 py-3 text-left text-xs text-zinc-500 uppercase tracking-wide font-medium">
                    Resource
                  </th>
                  <th className="px-4 py-3 text-left text-xs text-zinc-500 uppercase tracking-wide font-medium">
                    Actor
                  </th>
                  <th className="px-4 py-3 text-left text-xs text-zinc-500 uppercase tracking-wide font-medium">
                    Details
                  </th>
                </tr>
              </thead>
              <tbody className="divide-y divide-zinc-100 dark:divide-zinc-800/50">
                {filteredData.map((log) => (
                  <tr
                    key={log.id}
                    className="hover:bg-zinc-50 dark:hover:bg-zinc-900/30 transition-colors"
                  >
                    <td
                      className="px-4 py-3 text-zinc-500 whitespace-nowrap"
                      title={new Date(log.created_at).toLocaleString()}
                    >
                      {formatRelativeTime(log.created_at)}
                    </td>
                    <td className="px-4 py-3 whitespace-nowrap">
                      <ActionBadge action={log.action} />
                    </td>
                    <td className="px-4 py-3 whitespace-nowrap">
                      <span className="text-zinc-600 dark:text-zinc-400">
                        {log.resource_type.replace(/_/g, " ")}
                      </span>
                      {log.resource_id && (
                        <span className="text-zinc-400 dark:text-zinc-600 text-xs ml-1.5 font-mono">
                          {log.resource_id.slice(0, 8)}
                        </span>
                      )}
                    </td>
                    <td className="px-4 py-3 text-zinc-600 dark:text-zinc-400 whitespace-nowrap font-mono text-xs">
                      {log.actor_id.length > 20
                        ? `${log.actor_id.slice(0, 20)}...`
                        : log.actor_id}
                    </td>
                    <td className="px-4 py-3">
                      {Object.keys(log.metadata).length > 0 && (
                        <MetadataPreview metadata={log.metadata} />
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}

        {/* Pagination */}
        {totalPages > 1 && (
          <div className="flex items-center justify-between px-4 py-3 border-t border-zinc-200 dark:border-zinc-800">
            <span className="text-sm text-zinc-500">
              Page {page + 1} of {totalPages}
            </span>
            <div className="flex gap-2">
              <button
                onClick={() => setPage((p) => Math.max(0, p - 1))}
                disabled={page === 0}
                className="px-3 py-1 text-sm rounded-md bg-zinc-100 dark:bg-zinc-800 text-zinc-700 dark:text-zinc-300 hover:bg-zinc-200 dark:hover:bg-zinc-700 transition disabled:opacity-50 disabled:cursor-not-allowed"
              >
                Previous
              </button>
              <button
                onClick={() => setPage((p) => Math.min(totalPages - 1, p + 1))}
                disabled={page >= totalPages - 1}
                className="px-3 py-1 text-sm rounded-md bg-zinc-100 dark:bg-zinc-800 text-zinc-700 dark:text-zinc-300 hover:bg-zinc-200 dark:hover:bg-zinc-700 transition disabled:opacity-50 disabled:cursor-not-allowed"
              >
                Next
              </button>
            </div>
          </div>
        )}
      </div>
      )}
    </div>
  );
}

function ActionBadge({ action }: { action: string }) {
  let color = "bg-zinc-100 dark:bg-zinc-800 text-zinc-600 dark:text-zinc-400";

  if (action.startsWith("create") || action.startsWith("approve")) {
    color = "bg-emerald-50 text-emerald-600 dark:bg-emerald-500/10 dark:text-emerald-400";
  } else if (action.startsWith("delete") || action.startsWith("revoke")) {
    color = "bg-red-50 text-red-600 dark:bg-red-500/10 dark:text-red-400";
  } else if (action.startsWith("update") || action.startsWith("deploy")) {
    color = "bg-blue-50 text-blue-600 dark:bg-blue-500/10 dark:text-blue-400";
  } else if (action.includes("reject") || action.includes("cancel")) {
    color = "bg-amber-50 text-amber-600 dark:bg-amber-500/10 dark:text-amber-400";
  } else if (action.includes("notification") || action.includes("webhook")) {
    color = "bg-violet-50 text-violet-600 dark:bg-violet-500/10 dark:text-violet-400";
  }

  return (
    <span
      className={`inline-flex px-2 py-0.5 rounded text-xs font-medium ${color}`}
    >
      {formatAction(action)}
    </span>
  );
}

function MetadataPreview({ metadata }: { metadata: Record<string, unknown> }) {
  const entries = Object.entries(metadata).slice(0, 3);
  if (!entries.length) return null;

  return (
    <span
      className="text-xs text-zinc-400 dark:text-zinc-600"
      title={JSON.stringify(metadata, null, 2)}
    >
      {entries
        .map(
          ([k, v]) => `${k}: ${typeof v === "string" ? v : JSON.stringify(v)}`,
        )
        .join(", ")}
    </span>
  );
}
