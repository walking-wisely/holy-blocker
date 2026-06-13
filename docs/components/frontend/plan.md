# Frontend — Plan

The frontend is the public-facing React SPA hosted at `https://holyblocker.app`. It is
completely separate from the backend Go service and communicates with it only through
the JSON API (`/api/v1/...`).

Read alongside [docs/backend/PLAN.md](../backend/plan.md).

---

## 1. Responsibilities

The frontend owns two distinct concerns that happen to live on the same domain:

| Concern | Pages |
|---|---|
| **Marketing / product** | Landing page, download, about, privacy policy |
| **Partner flow** | Invite landing, email confirmation, unsubscribe, encouragement |

Keeping them in the same SPA avoids a second deployment target and lets them share
design tokens, fonts, and layout primitives. They are otherwise independent — separate
routes, separate components, no shared state.

---

## 2. Tech stack

- **React 18** with TypeScript
- **Vite** for bundling (same tooling as `apps/desktop`)
- **React Router v6** for client-side routing
- **TanStack Query** for API data fetching and cache management
- Styling: TBD (Tailwind CSS is the natural fit given the project's existing direction)
- Hosting: static file deployment (Cloudflare Pages or Vercel — the build output is a
  plain `dist/` directory with no SSR requirement)

No SSR framework (Next.js, Remix) in v1. The partner pages contain no content that
needs to be indexed by search engines, and the marketing pages are simple enough to
render fast as static HTML shells with client-side hydration.

---

## 3. Route map

```
/                          → Landing (marketing)
/download                  → Download page (links to GitHub Releases)
/privacy                   → Privacy policy
/invite/:token             → Partner invite landing page
/invite/verify/:confirmToken  → Email confirmation handler (auto-calls API on mount)
/unsubscribe               → Unsubscribe handler (?t={inviteToken} in query string)
/encouragement/:token      → Partner encouragement send page
```

---

## 4. Partner flow pages

### `/invite/:token`

Fetches `GET /api/v1/public/invites/{token}` on mount.

States:
- **Loading** — skeleton
- **Pending** — shows user's display name, plain-English explanation of what the partner
  will and won't receive, name + email form, "Confirm" button
- **Expired** — "This invite has expired. Ask [user] to send a new one."
- **Already active** — "You're already set up as [user]'s accountability partner."
- **Not found** — "This link isn't valid."

On form submit: calls `POST /api/v1/public/invites/{token}/confirm`, then renders
"Check your email" inline (no route change needed).

### `/invite/verify/:confirmToken`

On mount: calls `POST /api/v1/public/invites/verify` with the token from the URL.

States:
- **Loading** — "Confirming your email…"
- **Success** — "You're confirmed, [name]. You'll receive a notification if [user]
  attempts to disable their protection, and a weekly summary each Monday."
- **Expired / already used** — "This confirmation link has expired or was already used."

### `/unsubscribe`

Reads `?t={inviteToken}` from the query string on mount. Calls
`POST /api/v1/public/invites/{token}/unsubscribe`.

States:
- **Loading** — "Unsubscribing…"
- **Success** — "You've been removed. You won't receive any further notifications."
- **Already inactive** — "You're already unsubscribed."

### `/encouragement/:token`

Fetches invite status to verify the partnership is still active. Shows a short form:
preset options ("Proud of you", "Praying for you", "Keep going") and a free-text field
(max 200 chars). Calls `POST /api/v1/invites/{token}/encouragement`.

Rate-limited on the backend (one per 24 hours); frontend shows a friendly message if
the limit is hit.

---

## 5. Marketing pages (v1 scope)

The v1 marketing section is intentionally minimal — the product has no paying customers
yet and the priority is the partner flow.

**`/`** — one-page landing:
- Brief product description (what Holy Blocker does and why)
- Download button (links to latest GitHub Release installer)
- Two-line accountability section (linking to privacy policy)
- No analytics, no cookie banners, no tracking

**`/download`** — fetches the latest release from GitHub's public API and renders a
direct download link. Fallback to a hardcoded URL if the API is unavailable.

**`/privacy`** — static text. Key points: no browsing data ever leaves the device, the
backend holds only partnership metadata (partner email, invite token, streak count),
partner email is used only for the notifications they opted into.

---

## 6. Repository location

```
apps/
  web/                     ← this app
    src/
      pages/
        Landing.tsx
        Download.tsx
        Privacy.tsx
        invite/
          InviteLanding.tsx
          InviteVerify.tsx
          Unsubscribe.tsx
          Encouragement.tsx
      components/
        Layout.tsx
        InviteForm.tsx
        ConfirmationStates.tsx
      api/
        client.ts          ← typed fetch wrapper pointing at VITE_API_URL
        invites.ts         ← TanStack Query hooks for invite endpoints
      main.tsx
      router.tsx
    index.html
    vite.config.ts
    tsconfig.json
    package.json
```

Sits alongside `apps/desktop` in the monorepo. Shares `pnpm` workspace tooling.
Does **not** share code with the desktop app — different runtime, different target.

---

## 7. Environment configuration

```
VITE_API_URL=https://api.holyblocker.app    # production
VITE_API_URL=http://localhost:8080           # local dev
```

The backend and frontend are deployed to separate origins in production
(`holyblocker.app` vs `api.holyblocker.app`), which is why the backend requires CORS
configuration (see [backend/PLAN.md](../backend/plan.md)).

---

## 8. Implementation order

Partner flow comes first — it is the only frontend dependency blocking the
accountability feature.

1. Project scaffold: `pnpm create vite apps/web --template react-ts`, React Router,
   TanStack Query, base layout
2. `api/client.ts` + `api/invites.ts` — typed wrappers for all partner endpoints
3. `InviteLanding.tsx` — the page partners actually land on; most important to get right
4. `InviteVerify.tsx` and `Unsubscribe.tsx` — short, mostly state-machine components
5. `Encouragement.tsx`
6. Landing page (`/`) and `Download.tsx` — can be static placeholders initially
7. `Privacy.tsx`

---

## Related

- [docs/backend/PLAN.md](../backend/plan.md) — API the frontend calls
- [docs/flows/partner-setup.md](../../product/flows/partner-setup.md) — full partner flow
- [docs/decisions/accountability.md](../../decisions/accountability.md) — what partners
  see and why
