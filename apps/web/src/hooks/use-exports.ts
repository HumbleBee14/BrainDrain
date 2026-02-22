"use client";

import { useAuth } from "@clerk/nextjs";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { api, type ExportResponse, type ExportDownloadResponse } from "@/lib/api-client";

export function useModelExports(modelId: string) {
  const { getToken } = useAuth();

  return useQuery<ExportResponse[]>({
    queryKey: ["exports", modelId],
    queryFn: async () => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.exports.list(token, modelId);
    },
    enabled: !!modelId,
    refetchInterval: (query) => {
      const data = query.state.data;
      if (!data) return false;
      // Poll while any export is in progress
      const hasActive = data.some(
        (e) => e.status === "pending" || e.status === "processing"
      );
      return hasActive ? 5000 : false;
    },
  });
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
