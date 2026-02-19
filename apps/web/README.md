# apps/web

**The Next.js frontend — dashboard for managing projects, uploads, training, and models.**

| | |
|---|---|
| Language | TypeScript |
| Framework | Next.js 15 (App Router) |
| Auth | Clerk (`@clerk/nextjs`) |
| Styling | Tailwind CSS |
| Data fetching | React Query (`@tanstack/react-query`) |
| Deploys as | Standalone Node.js server in Docker (~50MB image) |
| Port | 3000 (default) |

## Pages

| Route | Page | Description |
|---|---|---|
| `/` | Landing | Marketing page |
| `/sign-in` | Auth | Clerk sign-in |
| `/sign-up` | Auth | Clerk sign-up |
| `/dashboard` | Dashboard | Stats overview (projects, models, documents) |
| `/projects` | Project list | All projects with loading/empty/data states |
| `/projects/new` | Create project | Form: name, description, task type |
| `/projects/[id]` | Project detail | Info, document upload, pipeline stages, delete |

## Key Files

| File | Purpose |
|---|---|
| `src/middleware.ts` | Clerk auth — protects `/dashboard` and `/projects` routes |
| `src/app/providers.tsx` | React Query client setup |
| `src/lib/api-client.ts` | Typed fetch wrapper — auto-injects Clerk bearer token |
| `src/hooks/use-projects.ts` | React Query hooks: `useProjects`, `useProject`, `useCreateProject`, `useDeleteProject` |
| `src/lib/utils.ts` | `cn()` helper (clsx + tailwind-merge) |

## Running Locally

```bash
# 1. Install dependencies
pnpm install

# 2. Set up environment
cp .env.local.example .env.local
# Fill in Clerk keys and API URL

# 3. Start dev server
pnpm dev          # http://localhost:3000
```

### Required Environment Variables

| Variable | Example | Description |
|---|---|---|
| `NEXT_PUBLIC_CLERK_PUBLISHABLE_KEY` | `pk_test_...` | Clerk public key |
| `CLERK_SECRET_KEY` | `sk_test_...` | Clerk secret key |
| `NEXT_PUBLIC_API_URL` | `http://localhost:8000` | Rust API backend URL |

### Other Commands

```bash
pnpm build        # Production build (standalone output)
pnpm lint         # ESLint
pnpm type-check   # TypeScript check (if script exists, else: npx tsc --noEmit)
```

## Docker Build & Deploy

```bash
# Build image (from repo root — needs workspace context)
docker build -f apps/web/Dockerfile -t platform-web .

# Run container
docker run -p 3000:3000 \
  -e NEXT_PUBLIC_CLERK_PUBLISHABLE_KEY=pk_... \
  -e CLERK_SECRET_KEY=sk_... \
  -e NEXT_PUBLIC_API_URL=https://api.platform.com \
  platform-web
```

Final image is **~50MB** (Alpine + Next.js standalone).
