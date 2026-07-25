"use client";

import { useAuth } from "@clerk/nextjs";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  api,
  type Model,
  type DeploymentStatusResponse,
} from "@/lib/api-client";
import { useStatusStream } from "@/hooks/use-status-stream";

export function useDeploymentStatus(modelId: string, enabled = true) {
  const { getToken } = useAuth();
  const queryClient = useQueryClient();

  const query = useQuery<DeploymentStatusResponse>({
    queryKey: ["deployment-status", modelId],
    queryFn: async () => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.deployments.status(token, modelId);
    },
    enabled: !!modelId && enabled,
    // Deploy now completes server-side, so the UI must converge on its own.
    refetchInterval: (query) =>
      query.state.data?.deployment_status === "deploying" ? 5000 : false,
  });

  const isDeploying = query.data?.deployment_status === "deploying";

  useStatusStream<DeploymentStatusResponse>(
    modelId ? `/api/v1/models/${modelId}/deployment/stream` : null,
    !!modelId && enabled && isDeploying,
    (data) => {
      queryClient.setQueryData(["deployment-status", modelId], data);
    },
  );

  return query;
}

export function useDeployModel(modelId: string) {
  const { getToken } = useAuth();
  const queryClient = useQueryClient();

  return useMutation<Model, Error, void>({
    mutationFn: async () => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.deployments.deploy(token, modelId);
    },
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: ["deployment-status", modelId],
      });
      queryClient.invalidateQueries({
        queryKey: ["models", "detail", modelId],
      });
    },
  });
}

export function useUndeployModel(modelId: string) {
  const { getToken } = useAuth();
  const queryClient = useQueryClient();

  return useMutation<Model, Error, void>({
    mutationFn: async () => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.deployments.undeploy(token, modelId);
    },
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: ["deployment-status", modelId],
      });
      queryClient.invalidateQueries({
        queryKey: ["models", "detail", modelId],
      });
    },
  });
}
