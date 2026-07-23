"use client";

import { useAuth } from "@clerk/nextjs";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  api,
  type InferenceSample,
  type FeedbackRating,
  type PaginatedResponse,
  type PromoteSampleItem,
  type PromoteSamplesResponse,
} from "@/lib/api-client";

export function useSamples(
  modelId: string,
  offset = 0,
  limit = 20,
  rating?: FeedbackRating | "unrated",
) {
  const { getToken } = useAuth();

  return useQuery<PaginatedResponse<InferenceSample>>({
    queryKey: ["samples", modelId, "list", offset, limit, rating ?? "all"],
    queryFn: async () => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.feedback.listSamples(token, modelId, offset, limit, rating);
    },
    enabled: !!modelId,
  });
}

export function useRateSample(modelId: string) {
  const { getToken } = useAuth();
  const queryClient = useQueryClient();

  return useMutation<
    void,
    Error,
    { sampleId: string; rating: FeedbackRating; comment?: string }
  >({
    mutationFn: async ({ sampleId, rating, comment }) => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.feedback.rateSample(token, sampleId, rating, comment);
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["samples", modelId] });
    },
  });
}

export function usePromoteSamples(modelId: string) {
  const { getToken } = useAuth();
  const queryClient = useQueryClient();

  return useMutation<
    PromoteSamplesResponse,
    Error,
    { samples: PromoteSampleItem[]; name?: string }
  >({
    mutationFn: async ({ samples, name }) => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.feedback.promoteSamples(token, modelId, samples, name);
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["samples", modelId] });
      queryClient.invalidateQueries({ queryKey: ["datasets"] });
    },
  });
}

export function useSetCapture(modelId: string) {
  const { getToken } = useAuth();
  const queryClient = useQueryClient();

  return useMutation<void, Error, boolean>({
    mutationFn: async (enabled) => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.feedback.setCapture(token, modelId, enabled);
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["models"] });
    },
  });
}
