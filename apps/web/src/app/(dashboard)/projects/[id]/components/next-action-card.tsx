"use client";

import type { ReactNode } from "react";

/**
 * The one card that answers "what happens next?". `progress` renders an
 * animated in-flight state; otherwise children carry the action buttons.
 */
export function NextActionCard({
  title,
  detail,
  progress = false,
  children,
}: {
  title: string;
  detail: string;
  progress?: boolean;
  children?: ReactNode;
}) {
  return (
    <div className="rounded-lg border border-violet-200 bg-violet-50/40 p-4 dark:border-violet-900/60 dark:bg-violet-950/20 md:p-5">
      <div className="flex items-start gap-3">
        {progress && (
          <span className="mt-1.5 h-2 w-2 shrink-0 animate-pulse rounded-full bg-violet-500" />
        )}
        <div className="min-w-0 flex-1">
          <h3 className="font-semibold text-zinc-900 dark:text-white">
            {title}
          </h3>
          <p className="mt-0.5 text-sm text-zinc-600 dark:text-zinc-400">
            {detail}
          </p>
          {children && (
            <div className="mt-3 flex flex-wrap items-center gap-2">
              {children}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
