# ── Stage 1: Dependencies ──
FROM node:20-alpine AS deps

RUN corepack enable && corepack prepare pnpm@9 --activate

WORKDIR /app

COPY package.json pnpm-lock.yaml pnpm-workspace.yaml ./
COPY apps/web/package.json apps/web/package.json
COPY packages/shared-types/package.json packages/shared-types/package.json

RUN pnpm install --frozen-lockfile

# ── Stage 2: Build ──
FROM node:20-alpine AS builder

RUN corepack enable && corepack prepare pnpm@9 --activate

WORKDIR /app

COPY --from=deps /app/node_modules ./node_modules
COPY --from=deps /app/apps/web/node_modules ./apps/web/node_modules
COPY --from=deps /app/packages/shared-types/node_modules ./packages/shared-types/node_modules

COPY package.json pnpm-workspace.yaml turbo.json ./
COPY apps/web/ apps/web/
COPY packages/shared-types/ packages/shared-types/

# Clerk requires NEXT_PUBLIC_CLERK_PUBLISHABLE_KEY at build time for page prerendering.
# Pass the real key via --build-arg for production builds. The default must be a
# format-valid Clerk test key (decodes to a host ending in `$`) — ClerkProvider
# rejects a malformed key during prerender. This dummy points at no real instance
# and is only for keyless CI validation builds.
ARG NEXT_PUBLIC_CLERK_PUBLISHABLE_KEY=pk_test_Y2xlcmsuZXhhbXBsZS5jb20k
ENV NEXT_PUBLIC_CLERK_PUBLISHABLE_KEY=$NEXT_PUBLIC_CLERK_PUBLISHABLE_KEY

RUN pnpm --filter web build

# ── Stage 3: Runtime ──
FROM node:20-alpine AS runtime

WORKDIR /app

RUN addgroup -g 1001 -S appuser && adduser -S appuser -u 1001

COPY --from=builder /app/apps/web/.next/standalone ./
COPY --from=builder /app/apps/web/.next/static ./apps/web/.next/static
COPY --from=builder /app/apps/web/public ./apps/web/public

USER appuser

ENV NODE_ENV=production
ENV PORT=3000
EXPOSE 3000

HEALTHCHECK --interval=15s --timeout=3s --retries=3 \
  CMD wget -q --spider http://localhost:3000/ || exit 1

CMD ["node", "apps/web/server.js"]
