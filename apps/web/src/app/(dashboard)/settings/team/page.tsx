"use client";

import { useEffect, useState } from "react";
import { toast } from "sonner";
import { z } from "zod";
import {
  useTeamMembers,
  useTeamInvitations,
  useInviteMember,
  useUpdateRole,
  useRemoveMember,
  useRevokeInvitation,
} from "@/hooks/use-team";

const emailSchema = z
  .string()
  .trim()
  .min(1, "Email is required")
  .email("Enter a valid email address");

export default function TeamSettingsPage() {
  const { data: members, isLoading: membersLoading } = useTeamMembers();
  const { data: invitations, isLoading: invitationsLoading } =
    useTeamInvitations();
  const inviteMember = useInviteMember();
  const updateRole = useUpdateRole();
  const removeMember = useRemoveMember();
  const revokeInvitation = useRevokeInvitation();

  useEffect(() => {
    if (inviteMember.isSuccess) toast.success("Invitation sent");
  }, [inviteMember.isSuccess]);
  useEffect(() => {
    if (inviteMember.isError) toast.error(inviteMember.error.message);
  }, [inviteMember.isError, inviteMember.error]);
  useEffect(() => {
    if (updateRole.isSuccess) toast.success("Role updated");
  }, [updateRole.isSuccess]);
  useEffect(() => {
    if (updateRole.isError) toast.error(updateRole.error.message);
  }, [updateRole.isError, updateRole.error]);
  useEffect(() => {
    if (removeMember.isSuccess) toast.success("Member removed");
  }, [removeMember.isSuccess]);
  useEffect(() => {
    if (removeMember.isError) toast.error(removeMember.error.message);
  }, [removeMember.isError, removeMember.error]);
  useEffect(() => {
    if (revokeInvitation.isSuccess) toast.success("Invitation revoked");
  }, [revokeInvitation.isSuccess]);
  useEffect(() => {
    if (revokeInvitation.isError) toast.error(revokeInvitation.error.message);
  }, [revokeInvitation.isError, revokeInvitation.error]);

  const [email, setEmail] = useState("");
  const [role, setRole] = useState("member");
  const [emailError, setEmailError] = useState<string | null>(null);

  const handleEmailChange = (value: string) => {
    setEmail(value);
    if (emailError) setEmailError(null);
  };

  const handleInvite = () => {
    const parsed = emailSchema.safeParse(email);
    if (!parsed.success) {
      setEmailError(parsed.error.issues[0]?.message ?? "Enter a valid email address");
      return;
    }
    const normalized = parsed.data.toLowerCase();

    const alreadyMember = members?.some(
      (m) => m.email.toLowerCase() === normalized,
    );
    if (alreadyMember) {
      setEmailError("This person is already a team member");
      return;
    }

    const alreadyInvited = invitations?.some(
      (i) => i.status === "pending" && i.email.toLowerCase() === normalized,
    );
    if (alreadyInvited) {
      setEmailError("An invitation is already pending for this email");
      return;
    }

    setEmailError(null);
    inviteMember.mutate(
      { email: normalized, role },
      {
        onSuccess: () => {
          setEmail("");
          setRole("member");
        },
      },
    );
  };

  return (
    <div className="max-w-4xl">
      <h1 className="text-xl md:text-2xl font-bold text-zinc-900 dark:text-white mb-2">Team Settings</h1>
      <p className="text-zinc-600 dark:text-zinc-400 mb-8">
        Manage your team members and invitations.
      </p>

      {/* Invite Form */}
      <div className="border border-zinc-200 dark:border-zinc-800 rounded-lg p-6 mb-8">
        <h2 className="text-lg font-semibold text-zinc-900 dark:text-white mb-4">
          Invite Team Member
        </h2>
        <div className="flex flex-col sm:flex-row gap-3">
          <div className="flex-1">
            <input
              type="email"
              placeholder="Email address"
              value={email}
              onChange={(e) => handleEmailChange(e.target.value)}
              className={`w-full bg-zinc-50 dark:bg-zinc-900 border rounded-md px-3 py-2 text-zinc-900 dark:text-white placeholder:text-zinc-400 dark:placeholder:text-zinc-500 focus:outline-none focus:ring-1 focus:ring-emerald-500 ${
                emailError
                  ? "border-red-400 dark:border-red-500"
                  : "border-zinc-300 dark:border-zinc-700"
              }`}
              onKeyDown={(e) => e.key === "Enter" && handleInvite()}
              aria-invalid={emailError ? true : undefined}
            />
          </div>
          <select
            value={role}
            onChange={(e) => setRole(e.target.value)}
            className="bg-zinc-50 dark:bg-zinc-900 border border-zinc-300 dark:border-zinc-700 rounded-md px-3 py-2 text-zinc-900 dark:text-white focus:outline-none focus:ring-1 focus:ring-emerald-500"
          >
            <option value="viewer">Viewer</option>
            <option value="member">Member</option>
            <option value="admin">Admin</option>
          </select>
          <button
            onClick={handleInvite}
            disabled={inviteMember.isPending || !email.trim()}
            className="bg-emerald-600 hover:bg-emerald-500 disabled:opacity-50 disabled:cursor-not-allowed text-white px-4 py-2 rounded-md text-sm font-medium transition"
          >
            {inviteMember.isPending ? "Sending..." : "Send Invite"}
          </button>
        </div>
        {emailError ? (
          <p className="text-red-500 text-sm mt-2">{emailError}</p>
        ) : (
          inviteMember.isError && (
            <p className="text-red-400 text-sm mt-2">
              {inviteMember.error.message}
            </p>
          )
        )}
      </div>

      {/* Team Members */}
      <div className="border border-zinc-200 dark:border-zinc-800 rounded-lg mb-8">
        <div className="p-4 border-b border-zinc-200 dark:border-zinc-800">
          <h2 className="text-lg font-semibold text-zinc-900 dark:text-white">
            Team Members{" "}
            {members && (
              <span className="text-zinc-500 font-normal">
                ({members.length})
              </span>
            )}
          </h2>
        </div>
        {membersLoading ? (
          <div className="p-8 text-center text-zinc-500">
            Loading members...
          </div>
        ) : members?.length === 0 ? (
          <div className="p-8 text-center text-zinc-500">
            No team members yet.
          </div>
        ) : (
          <div className="divide-y divide-zinc-200 dark:divide-zinc-800">
            {members?.map((member) => {
              const isUpdatingRole =
                updateRole.isPending &&
                updateRole.variables?.userId === member.user_id;
              const isRemoving =
                removeMember.isPending &&
                removeMember.variables === member.user_id;

              return (
                <div
                  key={member.id}
                  className="flex flex-col sm:flex-row sm:items-center justify-between gap-2 p-4"
                >
                  <div className="min-w-0">
                    <p className="text-zinc-900 dark:text-white text-sm truncate">{member.email}</p>
                    <p className="text-zinc-500 text-xs">
                      Joined {new Date(member.joined_at).toLocaleDateString()}
                    </p>
                  </div>
                  <div className="flex items-center gap-3">
                    {member.role === "owner" ? (
                      <span className="text-xs bg-amber-50 text-amber-600 dark:bg-amber-500/10 dark:text-amber-400 px-2 py-1 rounded">
                        Owner
                      </span>
                    ) : (
                      <select
                        value={member.role}
                        disabled={isUpdatingRole}
                        onChange={(e) =>
                          updateRole.mutate({
                            userId: member.user_id,
                            role: e.target.value,
                          })
                        }
                        className="bg-zinc-50 dark:bg-zinc-900 border border-zinc-300 dark:border-zinc-700 rounded px-2 py-1 text-xs text-zinc-700 dark:text-zinc-300 disabled:opacity-50 disabled:cursor-not-allowed"
                      >
                        <option value="viewer">Viewer</option>
                        <option value="member">Member</option>
                        <option value="admin">Admin</option>
                      </select>
                    )}
                    {member.role !== "owner" && (
                      <button
                        onClick={() => {
                          if (confirm(`Remove ${member.email} from the team?`)) {
                            removeMember.mutate(member.user_id);
                          }
                        }}
                        disabled={isRemoving}
                        className="text-red-400 hover:text-red-300 text-xs transition disabled:opacity-50 disabled:cursor-not-allowed"
                      >
                        {isRemoving ? "Removing..." : "Remove"}
                      </button>
                    )}
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </div>

      {/* Pending Invitations */}
      <div className="border border-zinc-200 dark:border-zinc-800 rounded-lg">
        <div className="p-4 border-b border-zinc-200 dark:border-zinc-800">
          <h2 className="text-lg font-semibold text-zinc-900 dark:text-white">
            Pending Invitations{" "}
            {invitations && (
              <span className="text-zinc-500 font-normal">
                ({invitations.filter((i) => i.status === "pending").length})
              </span>
            )}
          </h2>
        </div>
        {invitationsLoading ? (
          <div className="p-8 text-center text-zinc-500">
            Loading invitations...
          </div>
        ) : !invitations?.some((i) => i.status === "pending") ? (
          <div className="p-8 text-center text-zinc-500">
            No pending invitations.
          </div>
        ) : (
          <div className="divide-y divide-zinc-200 dark:divide-zinc-800">
            {invitations
              ?.filter((i) => i.status === "pending")
              .map((invitation) => {
                const isRevoking =
                  revokeInvitation.isPending &&
                  revokeInvitation.variables === invitation.id;

                return (
                  <div
                    key={invitation.id}
                    className="flex flex-col sm:flex-row sm:items-center justify-between gap-2 p-4"
                  >
                    <div className="min-w-0">
                      <p className="text-zinc-900 dark:text-white text-sm truncate">{invitation.email}</p>
                      <p className="text-zinc-500 text-xs">
                        Role: {invitation.role} &middot; Expires{" "}
                        {new Date(invitation.expires_at).toLocaleDateString()}
                      </p>
                    </div>
                    <button
                      onClick={() => revokeInvitation.mutate(invitation.id)}
                      disabled={isRevoking}
                      className="text-red-400 hover:text-red-300 text-xs transition disabled:opacity-50 disabled:cursor-not-allowed"
                    >
                      {isRevoking ? "Revoking..." : "Revoke"}
                    </button>
                  </div>
                );
              })}
          </div>
        )}
      </div>
    </div>
  );
}
