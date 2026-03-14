"use client";

import * as Sentry from "@sentry/nextjs";
import { useParams } from "next/navigation";
import Link from "next/link";
import { useEffect } from "react";

export default function DatasetError({
  error,
  reset,
}: {
  error: Error & { digest?: string };
  reset: () => void;
}) {
  const params = useParams<{ id: string }>();

  useEffect(() => {
    Sentry.captureException(error);
  }, [error]);

  return (
    <div className="flex flex-col items-center justify-center min-h-[60vh] gap-4">
      <div className="rounded-lg border border-red-200 dark:border-red-500/20 bg-red-50 dark:bg-red-500/5 p-8 max-w-md text-center">
        <h2 className="text-lg font-semibold text-zinc-900 dark:text-white mb-2">
          Failed to load dataset
        </h2>
        <p className="text-sm text-zinc-600 dark:text-zinc-400 mb-4">
          {error.message || "An unexpected error occurred while loading dataset details."}
        </p>
        {error.digest && (
          <p className="text-xs text-zinc-500 mb-4 font-mono">
            Error ID: {error.digest}
          </p>
        )}
        <div className="flex gap-3 justify-center">
          <button
            onClick={reset}
            className="px-4 py-2 text-sm font-medium rounded-md bg-zinc-100 dark:bg-zinc-800 text-zinc-900 dark:text-white hover:bg-zinc-200 dark:hover:bg-zinc-700 transition"
          >
            Try again
          </button>
          {params.id && (
            <Link
              href={`/projects/${params.id}`}
              className="px-4 py-2 text-sm font-medium rounded-md border border-zinc-300 dark:border-zinc-700 text-zinc-600 dark:text-zinc-400 hover:text-zinc-900 dark:hover:text-white transition"
            >
              Back to project
            </Link>
          )}
        </div>
      </div>
    </div>
  );
}
