"use client";

import * as Sentry from "@sentry/nextjs";
import Link from "next/link";
import { useEffect } from "react";

export default function ProjectError({
  error,
  reset,
}: {
  error: Error & { digest?: string };
  reset: () => void;
}) {
  useEffect(() => {
    Sentry.captureException(error);
  }, [error]);

  const isNotFound = error.message?.toLowerCase().includes("not found");

  return (
    <div className="flex flex-col items-center justify-center min-h-[60vh] gap-4">
      <div className="rounded-lg border border-red-500/20 bg-red-500/5 p-8 max-w-md text-center">
        <h2 className="text-lg font-semibold text-white mb-2">
          {isNotFound ? "Project not found" : "Failed to load project"}
        </h2>
        <p className="text-sm text-zinc-400 mb-4">
          {isNotFound
            ? "This project may have been deleted or you don't have access."
            : error.message || "An unexpected error occurred."}
        </p>
        {error.digest && (
          <p className="text-xs text-zinc-500 mb-4 font-mono">
            Error ID: {error.digest}
          </p>
        )}
        <div className="flex gap-3 justify-center">
          <button
            onClick={reset}
            className="px-4 py-2 text-sm font-medium rounded-md bg-zinc-800 text-white hover:bg-zinc-700 transition"
          >
            Try again
          </button>
          <Link
            href="/projects"
            className="px-4 py-2 text-sm font-medium rounded-md border border-zinc-700 text-zinc-400 hover:text-white transition"
          >
            Back to projects
          </Link>
        </div>
      </div>
    </div>
  );
}
