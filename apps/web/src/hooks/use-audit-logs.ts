"use client";

import { api } from "@/lib/api-client";
import { useAuthedQuery } from "@/hooks/use-authed-query";

export function useAuditLogs(
  params: {
    offset?: number;
    limit?: number;
    action?: string;
    resource_type?: string;
  } = {},
) {
  return useAuthedQuery({
    queryKey: [
      "audit-logs",
      params.offset,
      params.limit,
      params.action,
      params.resource_type,
    ],
    queryFn: (token) => api.auditLogs.list(token, params),
  });
}
