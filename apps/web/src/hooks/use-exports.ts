"use client";

import { useAuth } from "@clerk/nextjs";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  api,
  type ExportResponse,
  type ExportDownloadResponse,
  type OllamaExportResponse,
} from "@/lib/api-client";
import { useStatusStream } from "@/hooks/use-status-stream";

export function useModelExports(modelId: string) {
  const { getToken } = useAuth();
  const queryClient = useQueryClient();

  const query = useQuery<ExportResponse[]>({
    queryKey: ["exports", modelId],
    queryFn: async () => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.exports.list(token, modelId);
    },
    enabled: !!modelId,
    // The SSE stream below is best-effort; polling guarantees a terminal
    // status (including failures) reaches the UI without a manual reload.
    refetchInterval: (query) =>
      query.state.data?.some(
        (e) => e.status === "pending" || e.status === "processing",
      )
        ? 5000
        : false,
  });

  const hasActive = query.data?.some(
    (e) => e.status === "pending" || e.status === "processing",
  );

  useStatusStream<ExportResponse[]>(
    modelId ? `/api/v1/models/${modelId}/exports/stream` : null,
    !!modelId && !!hasActive,
    (data) => {
      queryClient.setQueryData(["exports", modelId], data);
    },
  );

  return query;
}

export function useCreateExport(modelId: string) {
  const { getToken } = useAuth();
  const queryClient = useQueryClient();

  return useMutation<ExportResponse, Error, { quant_type?: string }>({
    mutationFn: async (data) => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.exports.create(token, modelId, data);
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["exports", modelId] });
    },
  });
}

export function useExportDownload() {
  const { getToken } = useAuth();

  return useMutation<ExportDownloadResponse, Error, string>({
    mutationFn: async (exportId) => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.exports.download(token, exportId);
    },
  });
}

export function useOllamaRecipe() {
  const { getToken } = useAuth();

  return useMutation<OllamaExportResponse, Error, string>({
    mutationFn: async (exportId) => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.exports.ollama(token, exportId);
    },
  });
}
