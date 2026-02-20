"use client";

import { useAuth } from "@clerk/nextjs";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  api,
  type TriggerParseResponse,
  type TriggerRefineResponse,
  type ProjectPipelineStatus,
} from "@/lib/api-client";

export function usePipelineStatus(projectId: string, enabled = true) {
  const { getToken } = useAuth();

  const query = useQuery<ProjectPipelineStatus>({
    queryKey: ["pipeline-status", projectId],
    queryFn: async () => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.pipeline.getStatus(token, projectId);
    },
    enabled: !!projectId && enabled,
    refetchInterval: (query) => {
      const data = query.state.data;
      if (!data) return false;
      // Poll every 3s while pipeline is actively processing
      const isActive =
        data.documents.parsing > 0 || data.datasets.generating > 0;
      return isActive ? 3000 : false;
    },
  });

  return query;
}

export function useTriggerParse(projectId: string) {
  const { getToken } = useAuth();
  const queryClient = useQueryClient();

  return useMutation<TriggerParseResponse, Error>({
    mutationFn: async () => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.pipeline.triggerParse(token, projectId);
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["pipeline-status", projectId] });
      queryClient.invalidateQueries({ queryKey: ["documents", projectId] });
    },
  });
}

export function useTriggerRefine(projectId: string) {
  const { getToken } = useAuth();
  const queryClient = useQueryClient();

  return useMutation<
    TriggerRefineResponse,
    Error,
    { taskType?: string; config?: Record<string, unknown> }
  >({
    mutationFn: async ({ taskType, config }) => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.pipeline.triggerRefine(token, projectId, taskType, config);
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["pipeline-status", projectId] });
      queryClient.invalidateQueries({ queryKey: ["datasets", projectId] });
    },
  });
}
