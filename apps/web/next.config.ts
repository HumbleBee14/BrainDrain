import type { NextConfig } from "next";
import { withSentryConfig } from "@sentry/nextjs";

const nextConfig: NextConfig = {
  // The Docker deploy uses a standalone server. The Cloudflare (OpenNext) build
  // sets CF_WORKER_BUILD=1 and must use the default output — the adapter bundles
  // its own Worker.
  output: process.env.CF_WORKER_BUILD ? undefined : "standalone",
};

export default process.env.NEXT_PUBLIC_SENTRY_DSN
  ? withSentryConfig(nextConfig, {
      // Suppress source map upload warnings in dev
      silent: true,
      // Don't widen source map upload scope
      widenClientFileUpload: false,
    })
  : nextConfig;
