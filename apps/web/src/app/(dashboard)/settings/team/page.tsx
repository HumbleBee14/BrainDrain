"use client";

import { useState } from "react";
import {
  useTeamMembers,
  useTeamInvitations,
  useInviteMember,
  useUpdateRole,
  useRemoveMember,
  useRevokeInvitation,
} from "@/hooks/use-team";

export default function TeamSettingsPage() {
  const { data: members, isLoading: membersLoading } = useTeamMembers();
  const { data: invitations, isLoading: invitationsLoading } = useTeamInvitations();
  const inviteMember = useInviteMember();
  const updateRole = useUpdateRole();
  const removeMember = useRemoveMember();
  const revokeInvitation = useRevokeInvitation();

  const [email, setEmail] = useState("");
  const [role, setRole] = useState("member");

  const handleInvite = () => {
    if (!email.trim()) return;
    inviteMember.mutate({ email: email.trim(), role }, {
      onSuccess: () => {
        setEmail("");
        setRole("member");
      },
    });
  };

  return (
    <div className="max-w-4xl">
      <h1 className="text-2xl font-bold text-white mb-2">Team Settings</h1>
      <p className="text-zinc-400 mb-8">Manage your team members and invitations.</p>

      {/* Invite Form */}
      <div className="border border-zinc-800 rounded-lg p-6 mb-8">
        <h2 className="text-lg font-semibold text-white mb-4">Invite Team Member</h2>
        <div className="flex gap-3">
          <input
            type="email"
            placeholder="Email address"
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            className="flex-1 bg-zinc-900 border border-zinc-700 rounded-md px-3 py-2 text-white placeholder:text-zinc-500 focus:outline-none focus:ring-1 focus:ring-emerald-500"
            onKeyDown={(e) => e.key === "Enter" && handleInvite()}
          />
          <select
            value={role}
            onChange={(e) => setRole(e.target.value)}
            className="bg-zinc-900 border border-zinc-700 rounded-md px-3 py-2 text-white focus:outline-none focus:ring-1 focus:ring-emerald-500"
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
        {inviteMember.isError && (
          <p className="text-red-400 text-sm mt-2">{inviteMember.error.message}</p>
        )}
      </div>

      {/* Team Members */}
      <div className="border border-zinc-800 rounded-lg mb-8">
        <div className="p-4 border-b border-zinc-800">
          <h2 className="text-lg font-semibold text-white">
            Team Members {members && <span className="text-zinc-500 font-normal">({members.length})</span>}
          </h2>
        </div>
        {membersLoading ? (
          <div className="p-8 text-center text-zinc-500">Loading members...</div>
        ) : members?.length === 0 ? (
          <div className="p-8 text-center text-zinc-500">No team members yet.</div>
        ) : (
          <div className="divide-y divide-zinc-800">
            {members?.map((member) => (
              <div key={member.id} className="flex items-center justify-between p-4">
                <div>
                  <p className="text-white text-sm">{member.email}</p>
                  <p className="text-zinc-500 text-xs">
                    Joined {new Date(member.joined_at).toLocaleDateString()}
                  </p>
                </div>
                <div className="flex items-center gap-3">
                  {member.role === "owner" ? (
                    <span className="text-xs bg-amber-500/10 text-amber-400 px-2 py-1 rounded">
                      Owner
                    </span>
                  ) : (
                    <select
                      value={member.role}
                      onChange={(e) =>
                        updateRole.mutate({ userId: member.user_id, role: e.target.value })
                      }
                      className="bg-zinc-900 border border-zinc-700 rounded px-2 py-1 text-xs text-zinc-300"
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
                      className="text-red-400 hover:text-red-300 text-xs transition"
                    >
                      Remove
                    </button>
                  )}
                </div>
              </div>
            ))}
          </div>
        )}
      </div>

      {/* Pending Invitations */}
      <div className="border border-zinc-800 rounded-lg">
        <div className="p-4 border-b border-zinc-800">
          <h2 className="text-lg font-semibold text-white">
            Pending Invitations{" "}
            {invitations && (
              <span className="text-zinc-500 font-normal">
                ({invitations.filter((i) => i.status === "pending").length})
              </span>
            )}
          </h2>
        </div>
        {invitationsLoading ? (
          <div className="p-8 text-center text-zinc-500">Loading invitations...</div>
        ) : !invitations?.some((i) => i.status === "pending") ? (
          <div className="p-8 text-center text-zinc-500">No pending invitations.</div>
        ) : (
          <div className="divide-y divide-zinc-800">
            {invitations
              ?.filter((i) => i.status === "pending")
              .map((invitation) => (
                <div key={invitation.id} className="flex items-center justify-between p-4">
                  <div>
                    <p className="text-white text-sm">{invitation.email}</p>
                    <p className="text-zinc-500 text-xs">
                      Role: {invitation.role} &middot; Expires{" "}
                      {new Date(invitation.expires_at).toLocaleDateString()}
                    </p>
                  </div>
                  <button
                    onClick={() => revokeInvitation.mutate(invitation.id)}
                    className="text-red-400 hover:text-red-300 text-xs transition"
                  >
                    Revoke
                  </button>
                </div>
              ))}
          </div>
        )}
      </div>
    </div>
  );
}
