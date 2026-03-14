"use client";

import { useAuth } from "@clerk/nextjs";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  api,
  type TriggerParseResponse,
  type TriggerRefineResponse,
  type TriggerFullPipelineRequest,
  type TriggerFullPipelineResponse,
  type ProjectPipelineStatus,
} from "@/lib/api-client";
import { useStatusStream } from "@/hooks/use-status-stream";

export function usePipelineStatus(projectId: string, enabled = true) {
  const { getToken } = useAuth();
  const queryClient = useQueryClient();

  const query = useQuery<ProjectPipelineStatus>({
    queryKey: ["pipeline-status", projectId],
    queryFn: async () => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.pipeline.getStatus(token, projectId);
    },
    enabled: !!projectId && enabled,
  });

  const isActive =
    (query.data?.documents.parsing ?? 0) > 0 ||
    (query.data?.datasets.generating ?? 0) > 0 ||
    (query.data?.training_jobs.training ?? 0) > 0;

  useStatusStream<ProjectPipelineStatus>(
    projectId ? `/api/v1/projects/${projectId}/status/stream` : null,
    !!projectId && enabled && isActive,
    (data) => {
      queryClient.setQueryData(["pipeline-status", projectId], data);
    },
  );

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
      queryClient.invalidateQueries({
        queryKey: ["pipeline-status", projectId],
      });
      queryClient.invalidateQueries({ queryKey: ["documents", projectId] });
    },
  });
}

export function useTriggerFullPipeline(projectId: string) {
  const { getToken } = useAuth();
  const queryClient = useQueryClient();

  return useMutation<
    TriggerFullPipelineResponse,
    Error,
    TriggerFullPipelineRequest
  >({
    mutationFn: async (data) => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.pipeline.triggerFullPipeline(token, projectId, data);
    },
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: ["pipeline-status", projectId],
      });
      queryClient.invalidateQueries({ queryKey: ["documents", projectId] });
      queryClient.invalidateQueries({ queryKey: ["datasets", projectId] });
      queryClient.invalidateQueries({
        queryKey: ["training-jobs", projectId],
      });
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
      queryClient.invalidateQueries({
        queryKey: ["pipeline-status", projectId],
      });
      queryClient.invalidateQueries({ queryKey: ["datasets", projectId] });
    },
  });
}
