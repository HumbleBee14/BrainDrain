"use client";

const ROUTES = [
  {
    title: "Automatic",
    tagline: "Fastest path",
    reads: "your documents",
    produces: "a tuned model, no decisions needed",
    detail:
      "Auto-runs every step: parse, generate training data, train, evaluate.",
  },
  {
    title: "Guided",
    tagline: "Quality control",
    reads: "your documents",
    produces: "a dataset you review before training",
    detail:
      "Steer the data generation in Data Studio — rate samples and refine before any GPU time is spent.",
  },
  {
    title: "Distill",
    tagline: "Cost reduction",
    reads: "your documents + a teacher model",
    produces: "a small model that matches a big one",
    detail:
      "A large model teaches a small one you own, with a report proving how close it got.",
  },
] as const;

export function RouteExplainer({ onImport }: { onImport: () => void }) {
  return (
    <div>
      <h2 className="text-lg font-semibold text-zinc-900 dark:text-white">
        Three ways to build your model
      </h2>
      <p className="mt-1 text-sm text-zinc-500">
        Every route starts with your documents — upload below and pick a route
        after they&apos;re in.
      </p>
      <div className="mt-4 grid grid-cols-1 gap-3 md:grid-cols-3">
        {ROUTES.map((route) => (
          <div
            key={route.title}
            className="rounded-lg border border-zinc-200 p-4 dark:border-zinc-800"
          >
            <div className="flex items-baseline justify-between gap-2">
              <h3 className="font-semibold text-zinc-900 dark:text-white">
                {route.title}
              </h3>
              <span className="text-xs text-violet-600 dark:text-violet-400">
                {route.tagline}
              </span>
            </div>
            <p className="mt-2 text-sm text-zinc-600 dark:text-zinc-400">
              {route.detail}
            </p>
            <dl className="mt-3 space-y-1 text-xs">
              <div className="flex gap-1.5">
                <dt className="shrink-0 font-medium text-zinc-400 dark:text-zinc-600">
                  Reads
                </dt>
                <dd className="text-zinc-500">{route.reads}</dd>
              </div>
              <div className="flex gap-1.5">
                <dt className="shrink-0 font-medium text-zinc-400 dark:text-zinc-600">
                  You get
                </dt>
                <dd className="text-zinc-500">{route.produces}</dd>
              </div>
            </dl>
          </div>
        ))}
      </div>
      <p className="mt-3 text-sm text-zinc-500">
        Already have training data?{" "}
        <button
          onClick={onImport}
          className="font-medium text-violet-600 underline-offset-2 hover:underline dark:text-violet-400"
        >
          Import a JSONL dataset instead
        </button>
      </p>
    </div>
  );
}
