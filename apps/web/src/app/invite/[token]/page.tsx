"use client";

import { useParams, useRouter } from "next/navigation";
import { useAuth, useUser } from "@clerk/nextjs";
import { useState } from "react";
import { api } from "@/lib/api-client";

export default function AcceptInvitePage() {
  const { token } = useParams<{ token: string }>();
  const { getToken } = useAuth();
  const { user } = useUser();
  const router = useRouter();
  const [status, setStatus] = useState<"idle" | "loading" | "success" | "error">("idle");
  const [error, setError] = useState("");

  const handleAccept = async () => {
    if (!user) return;
    setStatus("loading");
    try {
      const authToken = await getToken();
      if (!authToken) throw new Error("Not authenticated");
      await api.team.acceptInvitation(authToken, token);
      setStatus("success");
      setTimeout(() => router.push("/dashboard"), 2000);
    } catch (e) {
      setStatus("error");
      setError(e instanceof Error ? e.message : "Failed to accept invitation");
    }
  };

  return (
    <div className="min-h-screen bg-zinc-950 flex items-center justify-center">
      <div className="border border-zinc-800 rounded-lg p-8 max-w-md w-full text-center">
        <h1 className="text-2xl font-bold text-white mb-4">Team Invitation</h1>
        {status === "success" ? (
          <div>
            <p className="text-emerald-400 mb-2">Invitation accepted!</p>
            <p className="text-zinc-400 text-sm">Redirecting to dashboard...</p>
          </div>
        ) : status === "error" ? (
          <div>
            <p className="text-red-400 mb-4">{error}</p>
            <button
              onClick={() => setStatus("idle")}
              className="text-zinc-400 hover:text-white text-sm transition"
            >
              Try again
            </button>
          </div>
        ) : (
          <div>
            <p className="text-zinc-400 mb-6">
              You've been invited to join a team. Click below to accept.
            </p>
            <button
              onClick={handleAccept}
              disabled={status === "loading" || !user}
              className="bg-emerald-600 hover:bg-emerald-500 disabled:opacity-50 text-white px-6 py-2 rounded-md font-medium transition"
            >
              {status === "loading" ? "Accepting..." : "Accept Invitation"}
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
