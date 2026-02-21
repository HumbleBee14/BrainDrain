"use client";

import { useQueryClient } from "@tanstack/react-query";
import {
  api,
  type TrainingJob,
  type PaginatedResponse,
  type CreateTrainingJobInput,
} from "@/lib/api-client";
import { useAuthedQuery, useAuthedMutation } from "@/hooks/use-authed-query";

export function useTrainingJobs(projectId: string, offset = 0, limit = 20) {
  return useAuthedQuery<PaginatedResponse<TrainingJob>>({
    queryKey: ["training-jobs", projectId, "list", offset, limit],
    queryFn: (token) => api.trainingJobs.list(token, projectId, offset, limit),
    enabled: !!projectId,
  });
}

export function useTrainingJob(id: string, enabled = true) {
  return useAuthedQuery<TrainingJob>({
    queryKey: ["training-jobs", "detail", id],
    queryFn: (token) => api.trainingJobs.get(token, id),
    enabled: !!id && enabled,
    refetchInterval: (query) => {
      const data = query.state.data;
      if (!data) return false;
      // Poll every 5s while job is actively training
      const isActive: boolean = data.status === "pending" || data.status === "provisioning" || data.status === "training";
      return isActive ? 5000 : false;
    },
  });
}

export function useCreateTrainingJob(projectId: string) {
  const queryClient = useQueryClient();

  return useAuthedMutation<TrainingJob, Error, CreateTrainingJobInput>({
    mutationFn: (token, data) => api.trainingJobs.create(token, projectId, data),
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: ["training-jobs", projectId],
      });
      queryClient.invalidateQueries({
        queryKey: ["pipeline-status", projectId],
      });
    },
  });
}

export function useCancelTrainingJob(projectId: string) {
  const queryClient = useQueryClient();

  return useAuthedMutation<TrainingJob, Error, string>({
    mutationFn: (token, jobId) => api.trainingJobs.cancel(token, jobId),
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: ["training-jobs", projectId],
      });
      queryClient.invalidateQueries({
        queryKey: ["pipeline-status", projectId],
      });
    },
  });
}
