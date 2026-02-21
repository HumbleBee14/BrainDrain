"use client";

import { useAuth } from "@clerk/nextjs";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  api,
  type TrainingJob,
  type PaginatedResponse,
  type CreateTrainingJobInput,
} from "@/lib/api-client";

export function useTrainingJobs(projectId: string, offset = 0, limit = 20) {
  const { getToken } = useAuth();

  return useQuery<PaginatedResponse<TrainingJob>>({
    queryKey: ["training-jobs", projectId, offset, limit],
    queryFn: async () => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.trainingJobs.list(token, projectId, offset, limit);
    },
    enabled: !!projectId,
  });
}

export function useTrainingJob(id: string, enabled = true) {
  const { getToken } = useAuth();

  return useQuery<TrainingJob>({
    queryKey: ["training-job", id],
    queryFn: async () => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.trainingJobs.get(token, id);
    },
    enabled: !!id && enabled,
    refetchInterval: (query) => {
      const data = query.state.data;
      if (!data) return false;
      // Poll every 5s while job is actively training
      const isActive = ["pending", "provisioning", "training"].includes(
        data.status
      );
      return isActive ? 5000 : false;
    },
  });
}

export function useCreateTrainingJob(projectId: string) {
  const { getToken } = useAuth();
  const queryClient = useQueryClient();

  return useMutation<TrainingJob, Error, CreateTrainingJobInput>({
    mutationFn: async (data) => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.trainingJobs.create(token, projectId, data);
    },
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
  const { getToken } = useAuth();
  const queryClient = useQueryClient();

  return useMutation<TrainingJob, Error, string>({
    mutationFn: async (jobId) => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.trainingJobs.cancel(token, jobId);
    },
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
