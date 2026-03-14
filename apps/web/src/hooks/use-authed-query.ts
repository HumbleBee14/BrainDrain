"use client";

import { useAuth } from "@clerk/nextjs";
import {
  useMutation,
  useQuery,
  type UseMutationOptions,
  type UseQueryOptions,
} from "@tanstack/react-query";

/**
 * Wrapper around useQuery that automatically injects the Clerk auth token.
 * Eliminates repeated getToken() boilerplate across all hooks.
 *
 * Usage:
 *   useAuthedQuery({
 *     queryKey: ["projects", offset, limit],
 *     queryFn: (token) => api.projects.list(token, offset, limit),
 *     enabled: !!projectId,
 *   });
 */
export function useAuthedQuery<TData = unknown, TError = Error>(
  options: Omit<UseQueryOptions<TData, TError>, "queryFn"> & {
    queryFn: (token: string) => Promise<TData>;
  },
) {
  const { getToken } = useAuth();

  return useQuery({
    ...options,
    queryFn: async () => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return options.queryFn(token);
    },
  });
}

/**
 * Wrapper around useMutation that automatically injects the Clerk auth token.
 * Eliminates repeated getToken() boilerplate across all mutation hooks.
 *
 * Usage:
 *   useAuthedMutation({
 *     mutationFn: (token, data) => api.projects.create(token, data),
 *     onSuccess: () => queryClient.invalidateQueries({ queryKey: ["projects"] }),
 *   });
 */
export function useAuthedMutation<
  TData = unknown,
  TError = Error,
  TVariables = void,
>(
  options: Omit<UseMutationOptions<TData, TError, TVariables>, "mutationFn"> & {
    mutationFn: (token: string, variables: TVariables) => Promise<TData>;
  },
) {
  const { getToken } = useAuth();

  return useMutation({
    ...options,
    mutationFn: async (variables: TVariables) => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return options.mutationFn(token, variables);
    },
  });
}
