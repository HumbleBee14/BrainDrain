# Deploying the web tier (free, no backend required)

Two independent sites, both free, both on your Cloudflare domain:

| Site | Repo | Host | Domain |
|---|---|---|---|
| Marketing + blog | `ekcron-org/ekcron-web` (Astro, static) | Cloudflare Pages | `ekcron.com` |
| App / dashboard | this repo, `apps/web` (Next.js, OpenNext) | Cloudflare Workers | `app.ekcron.com` |

The app needs **no backend** to run: login/logout is Clerk (hosted), so auth
works standalone. Calls to the Rust API just fail into error states — pages stay
navigable. Point `NEXT_PUBLIC_API_URL` at a placeholder until the backend exists.

---

## 1. Marketing site → Cloudflare Pages

1. Cloudflare dashboard → **Workers & Pages → Create → Pages → Connect to Git** → `ekcron-org/ekcron-web`.
2. Build settings: preset **Astro**, build command `pnpm build`, output directory `dist`.
3. (Optional) env vars: `PUBLIC_APP_URL=https://app.ekcron.com` (defaults are fine).
4. Deploy, then **Custom domains → add `ekcron.com` and `www`** (auto-configured — DNS is already on Cloudflare).

Done. No backend, no secrets.

---

## 2. App → Cloudflare Workers (OpenNext)

### The critical detail: build-time vs runtime env

`NEXT_PUBLIC_*` variables are **baked into the client bundle during the build**.
They must be present as **build variables**, not only as Worker runtime vars.
Server-only values (`CLERK_SECRET_KEY`) are runtime secrets.

| Variable | Type | Notes |
|---|---|---|
| `NEXT_PUBLIC_CLERK_PUBLISHABLE_KEY` | build | `pk_test_…` to start; `pk_live_…` for production Clerk |
| `NEXT_PUBLIC_API_URL` | build | Placeholder ok (e.g. `https://api.ekcron.com`); calls fail gracefully |
| `NEXT_PUBLIC_APP_NAME` | build | `Ekcron` |
| `NEXT_PUBLIC_MARKETING_URL` | build | `https://ekcron.com` (sign-out target) |
| `NEXT_PUBLIC_CLERK_SIGN_IN_URL` / `_SIGN_UP_URL` | build | `/sign-in`, `/sign-up` |
| `NEXT_PUBLIC_CLERK_AFTER_SIGN_IN_URL` / `_AFTER_SIGN_UP_URL` | build | `/dashboard` |
| `CLERK_SECRET_KEY` | runtime secret | `sk_test_…` / `sk_live_…` |

### Option A — Workers Builds (Git integration, recommended)

1. Cloudflare → **Workers & Pages → Create → Workers → Import a repository** → this repo.
2. **Root directory:** `apps/web`
3. **Build command:** `pnpm cf:build`  ·  **Deploy command:** `pnpm exec opennextjs-cloudflare deploy`
4. Add the **build variables** from the table above (all the `NEXT_PUBLIC_*` ones).
5. Add `CLERK_SECRET_KEY` as an encrypted **runtime** variable/secret.
6. Deploy. You get a `…​.workers.dev` URL — test login there first.
7. **Custom domain:** Worker → **Settings → Domains & Routes → Add → `app.ekcron.com`** (auto-routes, DNS on Cloudflare).

### Option B — deploy from your laptop (fastest first deploy)

```bash
cd apps/web
npx wrangler login                 # authorize in browser
# put the NEXT_PUBLIC_* + CLERK vars in apps/web/.env.local (gitignored)
pnpm cf:deploy                     # builds (CF_WORKER_BUILD=1) + deploys
npx wrangler secret put CLERK_SECRET_KEY   # runtime secret
```

`.env.local` is read at build time, so the `NEXT_PUBLIC_*` values get inlined
correctly. Add the custom domain as in step 7 above.

---

## 3. Clerk for a custom domain

- **Testing:** the dev instance (`pk_test_…`) works on the `*.workers.dev` URL — verify login/logout there.
- **Production (`app.ekcron.com`):** create a Clerk **production instance**. It gives you a set of CNAME records (`clerk`, `accounts`, `clkmail`, …) to add in Cloudflare DNS. Then swap in the `pk_live_…` / `sk_live_…` keys.
- In Clerk, set the allowed/redirect origins to include `https://app.ekcron.com`, and the sign-out/after-auth URLs to match the env above.

---

## 4. DNS summary (all in Cloudflare)

| Record | Points to | Set by |
|---|---|---|
| `ekcron.com`, `www` | Pages project | Pages "Custom domains" (auto) |
| `app.ekcron.com` | Worker | Worker "Domains & Routes" (auto) |
| `clerk`, `accounts`, `clkmail`, … | Clerk | Clerk production-instance setup |

---

## Notes

- The Docker deploy path is unaffected: `output: standalone` still applies unless `CF_WORKER_BUILD=1` is set (only the `cf:*` scripts set it).
- OpenNext build artifacts (`.open-next/`, `.wrangler/`) are gitignored.
- Free-tier limits (Workers requests/day, Pages builds) are generous for a personal project; revisit if traffic grows.
