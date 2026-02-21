"use client";

import { useQueryClient } from "@tanstack/react-query";
import { api, type Project, type CreateProjectInput, type PaginatedResponse } from "@/lib/api-client";
import { useAuthedQuery, useAuthedMutation } from "@/hooks/use-authed-query";

export function useProjects(offset = 0, limit = 20) {
  return useAuthedQuery<PaginatedResponse<Project>>({
    queryKey: ["projects", offset, limit],
    queryFn: (token) => api.projects.list(token, offset, limit),
  });
}

export function useProject(id: string) {
  return useAuthedQuery<Project>({
    queryKey: ["project", id],
    queryFn: (token) => api.projects.get(token, id),
    enabled: !!id,
  });
}

export function useCreateProject() {
  const queryClient = useQueryClient();

  return useAuthedMutation<Project, Error, CreateProjectInput>({
    mutationFn: (token, data) => api.projects.create(token, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["projects"] });
    },
  });
}

export function useDeleteProject() {
  const queryClient = useQueryClient();

  return useAuthedMutation<void, Error, string>({
    mutationFn: (token, id) => api.projects.delete(token, id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["projects"] });
    },
  });
}
