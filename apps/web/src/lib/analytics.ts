/**
 * Provider-agnostic analytics abstraction.
 *
 * Swap the provider by setting NEXT_PUBLIC_ANALYTICS_PROVIDER.
 * Currently supports: "console" (dev logging), or none (no-op).
 * Add PostHog/Mixpanel/Amplitude by implementing AnalyticsProvider.
 */

export interface AnalyticsProvider {
  /** Track a named event with optional properties */
  track(event: string, properties?: Record<string, unknown>): void;
  /** Identify a user for event association */
  identify(userId: string, traits?: Record<string, unknown>): void;
  /** Track a page view */
  page(name?: string, properties?: Record<string, unknown>): void;
}

/** No-op provider — used when analytics is disabled */
const noopProvider: AnalyticsProvider = {
  track: () => {},
  identify: () => {},
  page: () => {},
};

/** Console provider — logs events in development */
const consoleProvider: AnalyticsProvider = {
  track: (event, properties) => {
    console.log("[Analytics] track:", event, properties);
  },
  identify: (userId, traits) => {
    console.log("[Analytics] identify:", userId, traits);
  },
  page: (name, properties) => {
    console.log("[Analytics] page:", name, properties);
  },
};

function createProvider(): AnalyticsProvider {
  const provider = process.env.NEXT_PUBLIC_ANALYTICS_PROVIDER;

  if (provider === "console") {
    return consoleProvider;
  }

  // Default: no-op
  return noopProvider;
}

/** Global analytics instance */
export const analytics: AnalyticsProvider = createProvider();

// ── Predefined event names for type safety ──

export const AnalyticsEvents = {
  // Projects
  PROJECT_CREATED: "project_created",
  PROJECT_DELETED: "project_deleted",

  // Documents
  DOCUMENTS_UPLOADED: "documents_uploaded",
  PARSE_TRIGGERED: "parse_triggered",

  // Datasets
  REFINE_TRIGGERED: "refine_triggered",
  DATASET_PREVIEWED: "dataset_previewed",

  // Training
  TRAINING_STARTED: "training_started",
  TRAINING_COMPLETED: "training_completed",

  // Evaluation
  EVALUATION_STARTED: "evaluation_started",

  // Deployment
  MODEL_DEPLOYED: "model_deployed",
  MODEL_UNDEPLOYED: "model_undeployed",
  API_KEY_CREATED: "api_key_created",

  // Playground
  PLAYGROUND_MESSAGE_SENT: "playground_message_sent",
} as const;
