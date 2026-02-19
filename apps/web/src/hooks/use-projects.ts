"use client";

import { useAuth } from "@clerk/nextjs";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { api, type Project, type CreateProjectInput, type PaginatedResponse } from "@/lib/api-client";

export function useProjects(offset = 0, limit = 20) {
  const { getToken } = useAuth();

  return useQuery<PaginatedResponse<Project>>({
    queryKey: ["projects", offset, limit],
    queryFn: async () => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.projects.list(token, offset, limit);
    },
  });
}

export function useProject(id: string) {
  const { getToken } = useAuth();

  return useQuery<Project>({
    queryKey: ["project", id],
    queryFn: async () => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.projects.get(token, id);
    },
    enabled: !!id,
  });
}

export function useCreateProject() {
  const { getToken } = useAuth();
  const queryClient = useQueryClient();

  return useMutation<Project, Error, CreateProjectInput>({
    mutationFn: async (data) => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.projects.create(token, data);
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["projects"] });
    },
  });
}

export function useDeleteProject() {
  const { getToken } = useAuth();
  const queryClient = useQueryClient();

  return useMutation<void, Error, string>({
    mutationFn: async (id) => {
      const token = await getToken();
      if (!token) throw new Error("Not authenticated");
      return api.projects.delete(token, id);
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["projects"] });
    },
  });
}
