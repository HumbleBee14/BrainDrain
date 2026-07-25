"use client";

import { forwardRef } from "react";

type Variant = "primary" | "secondary" | "ghost" | "danger";
type Size = "sm" | "md";

const BASE =
  "inline-flex items-center justify-center gap-2 rounded-lg font-medium transition-colors " +
  "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-offset-2 " +
  "focus-visible:ring-offset-white dark:focus-visible:ring-offset-zinc-950 " +
  "disabled:cursor-not-allowed disabled:opacity-50";

// Violet is the single primary across the app; blue stays reserved for links
// and informational accents so one action reads as primary on any given screen.
const VARIANTS: Record<Variant, string> = {
  primary:
    "bg-violet-600 text-white hover:bg-violet-500 active:bg-violet-700 focus-visible:ring-violet-500 disabled:hover:bg-violet-600",
  secondary:
    "border border-zinc-200 bg-white text-zinc-700 hover:bg-zinc-50 focus-visible:ring-zinc-400 " +
    "dark:border-zinc-800 dark:bg-zinc-900 dark:text-zinc-200 dark:hover:bg-zinc-800",
  ghost:
    "text-zinc-600 hover:bg-zinc-100 hover:text-zinc-900 focus-visible:ring-zinc-400 " +
    "dark:text-zinc-400 dark:hover:bg-zinc-800 dark:hover:text-white",
  danger:
    "border border-red-300 text-red-600 hover:bg-red-50 focus-visible:ring-red-500 " +
    "dark:border-red-900 dark:text-red-400 dark:hover:bg-red-950/40",
};

const SIZES: Record<Size, string> = {
  sm: "px-3 py-1.5 text-xs",
  md: "px-4 py-2 text-sm",
};

function Spinner() {
  return (
    <svg
      className="h-4 w-4 animate-spin"
      viewBox="0 0 24 24"
      fill="none"
      aria-hidden="true"
    >
      <circle
        className="opacity-25"
        cx="12"
        cy="12"
        r="10"
        stroke="currentColor"
        strokeWidth="4"
      />
      <path
        className="opacity-75"
        fill="currentColor"
        d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"
      />
    </svg>
  );
}

export interface ButtonProps
  extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: Variant;
  size?: Size;
  /// Shows a spinner and blocks interaction without collapsing the button width.
  loading?: boolean;
}

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(
  function Button(
    {
      variant = "primary",
      size = "md",
      loading = false,
      disabled,
      className = "",
      children,
      ...props
    },
    ref,
  ) {
    return (
      <button
        ref={ref}
        disabled={disabled || loading}
        aria-busy={loading || undefined}
        className={`${BASE} ${VARIANTS[variant]} ${SIZES[size]} ${className}`}
        {...props}
      >
        {loading && <Spinner />}
        {children}
      </button>
    );
  },
);
