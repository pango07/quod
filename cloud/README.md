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

The database tests run on an in-memory PGlite by default. Production uses
postgres.js, which encodes parameters differently (a jsonb bug once passed
on PGlite and failed on Neon), so CI should also run them against a real
Postgres: give the test job a Postgres service and set `TEST_DATABASE_URL`
(e.g. `postgres://postgres:postgres@localhost:5432/postgres`). Each test
file then migrates and uses a schema of its own, dropped at the end
(`lib/db/testing.ts`). Never point it at a database that matters.

`npm run dev` loads the committed `.env.development`, which opts in to a
local demo user (`QUOD_AUTH=demo`) and sample data (`QUOD_API=fixture`).
To sign in for real, copy `.env.example` to `.env.local` and fill in the
four WorkOS values; they take precedence over the demo flag. `next start`
refuses to boot without a complete WorkOS config, or with a `QUOD_API` it
cannot use, and never runs the demo, whatever `NODE_ENV` says.

Quod's own data comes from quod-server when `QUOD_API=http` and
`QUOD_SERVER_URL` are set; unset, teams see their GitHub CI and Quod's
numbers say "Appears once Quod runs" (`docs/api.md`, "Go-live"). To run the
pages against quod-server's recorded answers without a server, start
`node scripts/quod-server-stub.mjs` (dev only) and `npm run dev` with the
variables `docs/api.md` gives.

## Trust wording: classic or plain

The Trust page, a repo's Steps tab, the run and step pages and the
Overview's trust card have two wordings (cloud/trust-plain): "classic" (the
pages as they were) and "plain" (one row per job and command with earlier
versions folded under it; states in words: "Learning: needs 1 good run",
"Ready to skip", "Skipping", "Paused: a re-check found different output";
"skipped" for "replayed", "re-check" for "spot check"). `lib/ui-version.ts`
picks one: `?ui=classic` or `?ui=plain` on any page (remembered in the
`quod_ui` cookie), else `NEXT_PUBLIC_TRUST_V2_UI=0` at build time for
classic, else plain (the default). Going back for everyone is that variable
(and a redeploy); for one viewer, `?ui=classic`. The classic components are
untouched.

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
- `lib/api/`: the `CloudApi` interface and its adapters: sample data
  (`fixture.ts`), quod-server over HTTP (`http.ts`), none (`none.ts`), and
  the composite that adds GitHub's data. `docs/api.md` is the contract with
  quod-server, tested against its recorded answers (`contract.test.ts`).
- `lib/auth/`: the sign-in mode rule (`config.ts`), input checks
  (`validate.ts`), the sign-in actions (`auth-actions.ts`) and their pending
  steps (`flow.ts`), the session, WorkOS members, invitations and the Admin
  Portal.
- `lib/dal.ts`: every page resolves its team here (membership check).
- `lib/github/`: the Quod CI GitHub App: manifest setup, the webhook
  receiver, installation linking, and `live.ts`, the queries and job events
  for "Now running". `lib/live-events.ts` is the event shape.
- `lib/db/`, `db/migrations/`: Postgres (Neon) and its migrations. See
  "GitHub App setup" below.
- `proxy.ts`: the Next.js 16 proxy (formerly middleware). It refreshes the
  AuthKit session and sends signed-out visitors to `/sign-in?returnTo=...`.
- `next.config.ts`: the boot-time config check and security headers.
- `lib/live/`: the live layer. `sim.ts` (the sample team's simulated CI, a
  pure function of the clock), `sse.ts` (SSE framing), `stream.ts` (the
  stream: snapshot, events, heartbeats, close before the function limit),
  `scope.ts`. The browser side is `components/live/` (one EventSource per
  tab, "Now running", the Live badge). The route is `app/[team]/events`.

## Live

Pages render on the server as before; the browser then opens
`/[team]/events` (Server-Sent Events) and applies what happens on top:
"Now running" (jobs in progress with their current step and a ticking time,
and queued jobs with their wait), totals and the chart, repo last runs, the
run page step by step, and step trust. The tab title shows "● 2 running".
The header's Live badge says when the connection is reconnecting or paused
(hidden tabs pause). On the sample team the runs are simulated and labelled
"Sample data, simulated live"; set `QUOD_SIMULATE=0` to turn the simulator
off.

## Live CI from GitHub

For a team with a linked installation, Overview's "Now running" and the
repo list come from the GitHub App's data in Postgres, live, even before
Quod runs in a repo (Quod's numbers say "Appears once Quod runs in this
repo"). A team without one sees "Connect GitHub to see your CI live" (admins
get the button; members are told to ask an admin). Without `DATABASE_URL`,
or when the database does not answer, every page still renders and says so.

Every page works for a linked team before Quod runs in it: Runs lists the
workflow runs (filter by repo, branch, status; paged), a run opens at
`/<team>/<repo>/actions/<GitHub run id>` with its jobs and steps, updating
live; Runners lists the runners "Seen in your CI"; Usage shows CI minutes
from job time. Quod-only numbers say "Appears once Quod runs". Empty states
tell apart "not linked" (Connect GitHub), "linked, nothing yet" (push), and
data (`components/github-ci.tsx`, `lib/github/pages.ts`).

GitHub's webhooks report a job when it is queued, starts and completes, not
per step. So while someone watches a team, its live poller also reads the
team's running jobs from GitHub's REST API about every 15 s (at most 10 jobs
per pass, each job at most once per 15 s across server instances, backing
off on 403/429 or a low `X-RateLimit-Remaining`), and "Step 3 of 8" moves
with the job (`lib/github/step-poll.ts`). Nothing runs when nobody watches.

To try it locally with PGlite and made-up jobs:

```sh
npm run db:migrate -- --pglite /tmp/quod-pglite
# link a fake installation to a team (SQL into installations / repos, team_id = the team's WorkOS org id),
# then run the dev server with the database and a webhook secret:
DATABASE_URL=pglite:/tmp/quod-pglite GITHUB_APP_WEBHOOK_SECRET=dev-secret npm run dev
# and POST signed workflow_run / workflow_job payloads to /api/github/webhook
# (lib/github/fixtures.ts shows their shape and how they are signed).
```

PGlite allows one process per folder: link with the dev server stopped.

## GitHub App setup

The Quod CI GitHub App sends CI job status to Quod Cloud by webhook. It is
created once, from the dashboard, with GitHub's App Manifest flow.

**1. Database (engineer, once, after the product owner approves the SQL).**
The tables are in `db/migrations/0001_github_app.sql`,
`0002_github_app_review.sql`, `0003_reconcile_state.sql`,
`0004_step_polling.sql`, `0005_branch_protection.sql` (the writer-trust
warning's cache; not applied yet) and `0006_team_settings.sql` (the team's
runner price for the Overview's savings; not applied yet: until it is,
Settings can't save a price and the Overview uses GitHub's Linux 2-core
price). With Neon's
direct URL in your shell as
`DATABASE_URL_UNPOOLED` (it is in Vercel):

```sh
npm run db:migrate -- --dry-run   # prints the target host and pending files; writes nothing
npm run db:migrate                # applies them, each in its own transaction
```

**2. Vercel env (engineer).** Set these, then redeploy:

- `QUOD_OPERATOR_EMAILS`: the product owner's Quod sign-in email
  (comma-separated for more). Not secret.
- `APP_URL`: `https://app.quodci.com`. Not secret.
- `CRON_SECRET`: a random string of at least 16 characters, e.g. from
  `openssl rand -hex 32`. Secret. Vercel Cron sends it to the daily
  reconcile job (see "Lost webhooks" below).

`DATABASE_URL` is already set by the Neon integration.

**3. Create the App (product owner, on the phone).**

1. In the phone's browser, sign in to github.com as **pango07**. The App
   belongs to whichever account is signed in there.
2. Open **app.quodci.com/setup/github-app** and sign in to Quod.
3. Keep the name "Quod CI" (or change it) and tap **Create GitHub App**.
4. GitHub opens. Tap **Create GitHub App for pango07**. If GitHub says the
   name is taken, go back, change the name, and tap Create again.
5. GitHub returns to Quod with six values, shown **once**. For each one,
   tap **Copy**, then in Vercel open quod-cloud, Settings, Environment
   Variables, and add it for Production:

   | Variable | Secret? | In Vercel |
   | --- | --- | --- |
   | `GITHUB_APP_ID` | no | plain |
   | `GITHUB_APP_SLUG` | no | plain |
   | `GITHUB_APP_CLIENT_ID` | no | plain |
   | `GITHUB_APP_CLIENT_SECRET` | **yes** | mark Sensitive |
   | `GITHUB_APP_WEBHOOK_SECRET` | **yes** | mark Sensitive |
   | `GITHUB_APP_PRIVATE_KEY` | **yes** | mark Sensitive; paste all of it, BEGIN and END lines included |

   Leaving the page blanks the values, and they are not kept anywhere. If
   that happens, generate a new private key and client secret on the App's
   GitHub settings page instead.
6. In Vercel, **Redeploy** production.

**4. Install it (any team admin).** In the dashboard, open the team's
**Settings**, then **GitHub**, and tap **Connect GitHub** (or open
**/github/link** and tap **Install on GitHub**). Always start from Quod:
the install carries a one-time state that ties GitHub's answer to your
browser. Pick the account or organization and repositories, and authorize.
GitHub comes back to `/github/installed`. Quod checks with GitHub that you
may manage that installation, then shows **GitHub user X will connect Y**
and **Connect to &lt;team&gt;**. Tap it. The App is linked to the team you
are viewing; switch teams first to link another.

Linking requires (tested in `lib/github/link.test.ts` and
`flow-state.test.ts`):
- GitHub's answer carries Quod's state for your browser. A code without it
  is dropped and Quod asks GitHub again itself.
- You are an admin of the Quod team.
- GitHub lists the installation in your `/user/installations`.
- No other Quod team has it.
- It is your own account, or you administer every repository it covers.
- The set of repositories has not changed between the check and your tap.

To unlink, a team admin taps **Disconnect** in Settings, then GitHub; the
App stays installed on GitHub. Uninstalling on GitHub also unlinks it.

**Lost webhooks.** GitHub does not resend a failed webhook by itself, so
Quod reconciles in two ways. Each pass:
- asks GitHub to redeliver App deliveries that failed and never reached
  Quod's database;
- reads back from GitHub any run or job of a linked repo still unfinished
  after 15 minutes without an update.

The two triggers (Vercel Hobby allows one cron a day):
- **Daily:** Vercel Cron calls `/api/github/reconcile` once a day
  (`vercel.json`, about 06:17 UTC; Hobby may run it any time in that hour).
  It needs `CRON_SECRET`. It covers every team and the last 25 hours of
  failures, and deletes runs and jobs not updated for 30 days.
- **While someone watches:** when a team's dashboard is open, its live
  poller reconciles that team in the background, at most every 15 minutes.
  The `reconcile_state` table holds the throttle, so several server
  instances never double-run it.

With nobody watching, nothing runs between daily passes, so Neon can stay
scaled to zero. A failure while nobody watches heals at the latest at the
next daily run, or as soon as someone opens the team.

Locally: `npm run db:migrate -- --pglite ./.pglite`, then run
`DATABASE_URL=pglite:./.pglite npm run dev`. The GitHub round trips need a
real App, so locally the webhook (sign test payloads with your own
`GITHUB_APP_WEBHOOK_SECRET`) and the pages' error states are what run.
