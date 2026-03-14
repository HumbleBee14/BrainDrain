"use client";

import { useQueryClient } from "@tanstack/react-query";
import { api, type TeamMember, type TeamInvitation } from "@/lib/api-client";
import { useAuthedQuery, useAuthedMutation } from "@/hooks/use-authed-query";

export function useTeamMembers() {
  return useAuthedQuery<TeamMember[]>({
    queryKey: ["team", "members"],
    queryFn: (token) => api.team.listMembers(token),
  });
}

export function useTeamInvitations() {
  return useAuthedQuery<TeamInvitation[]>({
    queryKey: ["team", "invitations"],
    queryFn: (token) => api.team.listInvitations(token),
  });
}

export function useInviteMember() {
  const queryClient = useQueryClient();

  return useAuthedMutation<
    TeamInvitation,
    Error,
    { email: string; role?: string }
  >({
    mutationFn: (token, data) => api.team.invite(token, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["team"] });
    },
  });
}

export function useUpdateRole() {
  const queryClient = useQueryClient();

  return useAuthedMutation<TeamMember, Error, { userId: string; role: string }>(
    {
      mutationFn: (token, { userId, role }) =>
        api.team.updateRole(token, userId, role),
      onSuccess: () => {
        queryClient.invalidateQueries({ queryKey: ["team"] });
      },
    },
  );
}

export function useRemoveMember() {
  const queryClient = useQueryClient();

  return useAuthedMutation<void, Error, string>({
    mutationFn: (token, userId) => api.team.removeMember(token, userId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["team"] });
    },
  });
}

export function useRevokeInvitation() {
  const queryClient = useQueryClient();

  return useAuthedMutation<TeamInvitation, Error, string>({
    mutationFn: (token, id) => api.team.revokeInvitation(token, id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["team"] });
    },
  });
}
