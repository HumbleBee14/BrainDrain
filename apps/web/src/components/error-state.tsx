"use client";

export function ErrorState({
  title = "Something went wrong",
  message,
  onRetry,
  isRetrying = false,
  compact = false,
}: {
  title?: string;
  message?: string;
  onRetry?: () => void;
  isRetrying?: boolean;
  compact?: boolean;
}) {
  return (
    <div
      className={`flex flex-col items-center justify-center text-center rounded-lg border border-red-200 dark:border-red-900/50 bg-red-50/50 dark:bg-red-950/20 ${
        compact ? "p-6" : "p-10"
      }`}
    >
      <div className="flex h-10 w-10 items-center justify-center rounded-full bg-red-100 dark:bg-red-900/40 mb-3">
        <svg
          className="h-5 w-5 text-red-600 dark:text-red-400"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth={2}
          strokeLinecap="round"
          strokeLinejoin="round"
          aria-hidden="true"
        >
          <path d="M12 9v4" />
          <path d="M12 17h.01" />
          <path d="M10.29 3.86 1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0Z" />
        </svg>
      </div>
      <p className="text-sm font-medium text-zinc-900 dark:text-white">{title}</p>
      {message && (
        <p className="mt-1 max-w-md text-sm text-zinc-500 dark:text-zinc-400 break-words">
          {message}
        </p>
      )}
      {onRetry && (
        <button
          onClick={onRetry}
          disabled={isRetrying}
          className="mt-4 rounded-lg border border-zinc-300 dark:border-zinc-700 px-4 py-2 text-sm font-medium text-zinc-700 dark:text-zinc-300 hover:border-zinc-400 dark:hover:border-zinc-600 transition disabled:opacity-50"
        >
          {isRetrying ? "Retrying…" : "Try again"}
        </button>
      )}
    </div>
  );
}
