# Quod Cloud

The dashboard for app.quodci.com: which CI steps Quod replayed, why, and how
much time that saved. Next.js 16 (App Router, Cache Components). Sign-in is
our own pages on the WorkOS User Management APIs; the session cookie is
AuthKit's (`@workos-inc/authkit-nextjs`). The design it follows is the
2026-11-01 entry in `../DECISIONS.md`.

```sh
npm install
npm run dev        # http://localhost:3000, demo session and sample data
npm test           # unit tests (auth rules, returnTo and input checks, formatters, fixtures)
npm run lint && npx tsc --noEmit && npm run build
```

`npm run dev` loads the committed `.env.development`, which opts in to a
local demo user (`QUOD_AUTH=demo`) and sample data (`QUOD_API=fixture`).
To sign in for real, copy `.env.example` to `.env.local` and fill in the
four WorkOS values; they take precedence over the demo flag. `next start`
refuses to boot without a complete WorkOS config and a data source, and
never runs the demo, whatever `NODE_ENV` says.

`NEXT_PUBLIC_WORKOS_REDIRECT_URI` is inlined into the build: leave it unset
for `next build` and set it at runtime, or build with exactly the runtime
value.

## Sign-in pages

| Page | What it does |
| --- | --- |
| `/sign-in` | Email first. A domain with an SSO connection goes straight to that SSO; otherwise password, "Email me a code" (Magic Auth), GitHub or Google. |
| `/sign-up` | Name, email, password, then the email code. |
| `/verify-email` | The 6-digit email code (WorkOS `email_verification_required`). |
| `/sign-in/organization` | Pick a team (`organization_selection_required`). |
| `/sign-in/mfa` | Authenticator app: set up (`mfa_enrollment`) or code (`mfa_challenge`). |
| `/reset-password` | Ask for a link; with `?token=` choose a new password. |
| `/invite?invitation_token=` | Accept an invitation: create an account, or sign in to accept. |
| `/callback` | Where GitHub, Google and SSO come back (PKCE). |

In the demo session each page shows its states as previews (links under the
card, `?preview=`); nothing is sent. Previews are off whenever WorkOS is
configured.

## WorkOS dashboard settings

For each environment (staging keys: localhost and app.quodci.com today;
production keys later), under **Redirects** and **Authentication**:

| WorkOS setting | localhost | app.quodci.com |
| --- | --- | --- |
| Redirect URI (default) | `http://localhost:3000/callback` | `https://app.quodci.com/callback` |
| Sign-in endpoint (Initiate login URI) | `http://localhost:3000/sign-in` | `https://app.quodci.com/sign-in` |
| Sign-up URL | `http://localhost:3000/sign-up` | `https://app.quodci.com/sign-up` |
| User invitation URL | `http://localhost:3000/invite` | `https://app.quodci.com/invite` |
| Password reset URL | `http://localhost:3000/reset-password` | `https://app.quodci.com/reset-password` |
| Sign-out redirect | `http://localhost:3000/signed-out` | `https://app.quodci.com/signed-out` |

WorkOS appends `?invitation_token=` and `?token=` to the invitation and reset
URLs; the pages read those names.

## Code

- `app/`: routes. `/[team]`, `/[team]/[repo]`, `/[team]/[repo]/runs/[run]`,
  `/[team]/[repo]/steps/[key]`, `/[team]/connect`, `/[team]/settings`, and
  team pages for repositories, runs, trust, runners and usage. Sign-in pages
  are in `app/(auth)/`.
- `lib/api/`: the `CloudApi` interface, the sample-data adapter and the
  quod-server HTTP stub. See `docs/api.md`.
- `lib/auth/`: the sign-in mode rule (`config.ts`), input checks
  (`validate.ts`), the sign-in actions (`auth-actions.ts`) and their pending
  steps (`flow.ts`), the session, WorkOS members, invitations and the Admin
  Portal.
- `lib/dal.ts`: every page resolves its team here (membership check).
- `proxy.ts`: the Next.js 16 proxy (formerly middleware). It refreshes the
  AuthKit session and sends signed-out visitors to `/sign-in?returnTo=...`.
- `next.config.ts`: the boot-time config check and security headers.
