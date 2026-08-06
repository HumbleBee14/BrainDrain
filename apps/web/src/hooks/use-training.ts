"use client";

import { useQueryClient } from "@tanstack/react-query";
import {
  api,
  type TrainingJob,
  type CostEstimateResponse,
  type PaginatedResponse,
  type CreateTrainingJobInput,
} from "@/lib/api-client";
import { useAuthedQuery, useAuthedMutation } from "@/hooks/use-authed-query";
import { useStatusStream } from "@/hooks/use-status-stream";

export function useTrainingJobs(projectId: string, offset = 0, limit = 20) {
  return useAuthedQuery<PaginatedResponse<TrainingJob>>({
    queryKey: ["training-jobs", projectId, "list", offset, limit],
    queryFn: (token) => api.trainingJobs.list(token, projectId, offset, limit),
    enabled: !!projectId,
  });
}

export function useTrainingJob(id: string, enabled = true) {
  const queryClient = useQueryClient();

  const query = useAuthedQuery<TrainingJob>({
    queryKey: ["training-jobs", "detail", id],
    queryFn: (token) => api.trainingJobs.get(token, id),
    enabled: !!id && enabled,
  });

  const isActive =
    query.data?.status === "pending" ||
    query.data?.status === "provisioning" ||
    query.data?.status === "training";

  useStatusStream<TrainingJob>(
    id ? `/api/v1/training-jobs/${id}/status/stream` : null,
    !!id && enabled && isActive,
    (data) => {
      queryClient.setQueryData(["training-jobs", "detail", id], data);
    },
  );

  return query;
}

export function useCreateTrainingJob(projectId: string) {
  const queryClient = useQueryClient();

  return useAuthedMutation<TrainingJob, Error, CreateTrainingJobInput>({
    mutationFn: (token, data) =>
      api.trainingJobs.create(token, projectId, data),
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

export function useApproveCost(projectId: string) {
  const queryClient = useQueryClient();

  return useAuthedMutation<TrainingJob, Error, string>({
    mutationFn: (token, jobId) => api.trainingJobs.approveCost(token, jobId),
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

export function useTrainingMetricsSnapshot(jobId: string, enabled = true) {
  return useAuthedQuery<Record<string, unknown>>({
    queryKey: ["training-jobs", "metrics-snapshot", jobId],
    queryFn: (token) => api.trainingJobs.getMetrics(token, jobId),
    enabled: !!jobId && enabled,
  });
}

export function useEstimateTrainingCost(
  projectId: string,
  data: CreateTrainingJobInput,
) {
  return useAuthedQuery<CostEstimateResponse>({
    queryKey: [
      "training-cost-estimate",
      projectId,
      data.dataset_id,
      data.base_model,
      data.mode,
      data.gpu_class,
    ],
    queryFn: (token) => api.trainingJobs.estimate(token, projectId, data),
    enabled: !!projectId && !!data.dataset_id,
  });
}
