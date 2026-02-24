"use client";

import * as Sentry from "@sentry/nextjs";
import { useEffect } from "react";

export default function DashboardError({
  error,
  reset,
}: {
  error: Error & { digest?: string };
  reset: () => void;
}) {
  useEffect(() => {
    Sentry.captureException(error);
  }, [error]);

  return (
    <div className="flex flex-col items-center justify-center min-h-[60vh] gap-4">
      <div className="rounded-lg border border-red-500/20 bg-red-500/5 p-8 max-w-md text-center">
        <h2 className="text-lg font-semibold text-white mb-2">
          Something went wrong
        </h2>
        <p className="text-sm text-zinc-400 mb-4">
          {error.message || "An unexpected error occurred. Our team has been notified."}
        </p>
        {error.digest && (
          <p className="text-xs text-zinc-500 mb-4 font-mono">
            Error ID: {error.digest}
          </p>
        )}
        <button
          onClick={reset}
          className="px-4 py-2 text-sm font-medium rounded-md bg-zinc-800 text-white hover:bg-zinc-700 transition"
        >
          Try again
        </button>
      </div>
    </div>
  );
}
