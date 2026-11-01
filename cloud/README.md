# Quod Cloud

The dashboard for app.quodci.com: which CI steps Quod replayed, why, and how
much time that saved. Next.js 16 (App Router, Cache Components), WorkOS
AuthKit for sign-in. The design it follows is the 2026-11-01 entry in
`../DECISIONS.md`.

```sh
npm install
npm run dev        # http://localhost:3000, demo session and sample data
npm test           # unit tests (auth fail-closed rule, formatters, fixtures)
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

- `app/`: routes. `/[team]`, `/[team]/[repo]`, `/[team]/[repo]/runs/[run]`,
  `/[team]/[repo]/steps/[key]`, `/[team]/connect`, `/[team]/settings`, and
  team pages for repositories, runs, trust, runners and usage.
- `lib/api/`: the `CloudApi` interface, the sample-data adapter and the
  quod-server HTTP stub. See `docs/api.md`.
- `lib/auth/`: the sign-in mode rule (`config.ts`), the session, WorkOS
  members, invitations and the Admin Portal.
- `lib/dal.ts`: every page resolves its team here (membership check).
- `proxy.ts`: the Next.js 16 proxy (formerly middleware) that runs AuthKit.
- `next.config.ts`: the boot-time config check and security headers.
