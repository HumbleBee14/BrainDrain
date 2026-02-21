import type { NextConfig } from "next";
import { withSentryConfig } from "@sentry/nextjs";

const nextConfig: NextConfig = {
  output: "standalone",
};

export default process.env.NEXT_PUBLIC_SENTRY_DSN
  ? withSentryConfig(nextConfig, {
      // Suppress source map upload warnings in dev
      silent: true,
      // Don't widen source map upload scope
      widenClientFileUpload: false,
    })
  : nextConfig;
