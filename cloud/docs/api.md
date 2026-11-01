# CloudApi: what the dashboard needs from quod-server

The dashboard reads everything through one TypeScript interface,
`CloudApi` in `lib/api/types.ts`. That file is the source of truth for
field names and types; this page is the short version for the Rust side.

Two adapters implement it:

- `lib/api/fixture.ts`: sample data from the approved prototypes. Used only
  with `QUOD_API=fixture` (set for `next dev` by `.env.development`).
- `lib/api/http.ts`: quod-server over HTTP. **A stub**: written against the
  endpoints below, never run against a server.

## Protocol

- Base: `QUOD_API_URL`, paths under `/v1`. Every request sends
  `quod-protocol: quod.v1` and `authorization: Bearer <WorkOS access token>`.
- Authorize by the token's subject (the WorkOS user) and that user's
  membership in the team's WorkOS Organization. Do not rely on the token's
  `org_id` alone: a viewer can open any of their teams by URL.
- JSON only. Times are ISO 8601 UTC strings, durations are seconds, saved
  time is whole minutes, shares are 0..1.
- 404 for a team, repo, run or step the viewer cannot see (never 403, so
  existence does not leak). 401/403 only for a bad token.
- Overview, repo, run list and step responses carry `asOf` (the server
  clock); the UI measures "4m ago" from it.
- Repo slugs are unique per team and must not be one of the reserved team
  page names: `repositories`, `runs`, `trust`, `runners`, `usage`,
  `settings`, `connect`.

## Who may do what

WorkOS roles: `admin` and `member`. The server enforces these; the UI only mirrors them.

- Read anything in the team: any member.
- Re-run without Quod: any member, as anyone with access can re-run a workflow in GitHub.
- Change proofs needed for a step: admins.
- Save the team's Connect choice: admins. Members can still view both setups.
- Path ids (team, repo, step key) match `^[A-Za-z0-9._-]{1,100}$` and are never `.` or `..`;
  run numbers are integers 1..999,999,999. The adapter refuses anything else before a request.

## Endpoints

| Method and path | Body / query | Returns | Screen |
| --- | --- | --- | --- |
| `POST /v1/teams/resolve` | `{ orgs: [{ orgId, name }] }` | `{ teams: Team[] }` | team switcher; unknown orgs become new empty teams |
| `GET /v1/teams/:team/health` | | `TeamHealth` | sidebar "0 wrong replays" |
| `GET /v1/teams/:team/overview` | `?range=7d\|30d\|90d` | `Overview` | Overview, Usage |
| `GET /v1/teams/:team/repos` | | `{ repos: RepoSummary[] }` | |
| `GET /v1/teams/:team/repos/:repo` | | `RepoDetail` | Repository |
| `GET /v1/teams/:team/runs` | `?repo=&limit=` | `{ asOf, runs: RunSummary[] }` newest first | Runs |
| `GET /v1/teams/:team/repos/:repo/runs/:number` | | `RunDetail` | Run |
| `POST /v1/teams/:team/repos/:repo/runs/:number/rerun` | `{ quod: false }` | `{ accepted, message }` | "Re-run without Quod"; any member |
| `GET /v1/teams/:team/steps` | `?repo=` | `{ steps: StepSummary[] }` | Trust, repo Steps |
| `GET /v1/teams/:team/repos/:repo/steps/:key` | | `StepTrust` | Step trust |
| `PUT /v1/teams/:team/repos/:repo/steps/:key/proofs` | `{ need }` (2..10) | `StepTrust` | "Runs needed" toggle; team admins only |
| `GET /v1/teams/:team/runners` | | `{ runners: Runner[] }` | Runners |
| `GET /v1/teams/:team/connect` | | `ConnectSetup` | Connect |
| `PUT /v1/teams/:team/connect` | `{ store, host }` | 204 | Connect (remembers the choice); team admins only |
| `GET /v1/teams/:team/search-index` | | `SearchIndex` | ⌘K menu |

## Shapes, by screen

- **Overview** (`Overview`): `totals` (saved minutes, steps replayed of
  total, est. cost saved and the per-minute price, wrong replays, spot checks
  and how many were identical), `days` (one point per day: replayed and ran
  minutes), `repos` (replay rate, saved minutes, last run, paused steps),
  `trust` (proven, proving, watching, spot checks passed, paused steps),
  `runners`.
- **Repository** (`RepoDetail`): summary, default branch, `latestRun`
  (a full `RunDetail`), `runTimes` (last runs: with Quod vs recorded time
  without, oldest first), `steps` (`StepSummary`: job, command, trust state,
  replay rate, ran and replay seconds, saved minutes).
- **Run** (`RunDetail`): commit, branch, actor, runner, took vs would have
  taken, counts, and `jobs[].steps[]`, each with `outcome` (replayed or ran)
  and `why`, a tagged union:
  `replayed` (source run, inputs matched, proof, divergences, saved),
  `spot-check` (compared run, identical or not),
  `changed` (first changed path with old and new hash, how many others,
  recorded inputs, proof for the new input set),
  `first-run`, `paused`, `watching`, `network`, `not-memoized`.
  The UI writes the receipt and the plain-language sentence from these.
- **Step trust** (`StepTrust`): `trust` is one of
  `replaying`, `proving {have, need}`, `watching {wouldHaveReplayed}`,
  `paused {consistent, need}`; plus `proofsNeeded`, `divergence`
  (recorded and re-check run, input count, stream and line, the differing
  lines, exit codes), `history` (tagged events, newest first) and `inputs`
  (files read, folders listed, paths looked for and not found, network
  access).
- **Connect** (`ConnectSetup`): the setup action name, runner labels,
  install command, a self-hosted server example, the first repo (or null)
  and the saved choice.

Members, invitations, SSO and directory sync are not in this API: they live
in WorkOS and the app calls WorkOS directly (`lib/auth/directory.ts`).

## Event sources: GitHub App

Live CI job status comes from the Quod CI GitHub App's `workflow_job` and
`workflow_run` webhooks, stored in Postgres by the app itself (not
quod-server): `POST /api/github/webhook` writes, `lib/github/live.ts`
reads. Types are in `lib/live-events.ts`; they mirror cloud/live's
`LiveJob` and the `job.*` members of `LiveEvent` field for field, plus a
few additions.

**Server-side functions** (`lib/github/live.ts`). Each takes a `TeamId`,
which only `teamIdOf(await requireTeam(slug))` makes (it is the team's
WorkOS organization id). A team sees rows only through an installation
linked to it, not uninstalled or suspended, in a repo still in that installation. No
function takes an installation id from a client. `db` is optional (the
app's database by default; PGlite in tests).

| Function | Returns |
| --- | --- |
| `runningJobs(teamId)` | `LiveJob[]` in progress, newest first (last 24 h) |
| `queuedJobs(teamId)` | `LiveJob[]` queued, waiting or pending, oldest first (last 24 h) |
| `jobById(teamId, jobId)` | `LiveJob` or null |
| `recentRuns(teamId, repo?, { limit })` | `LiveRun[]` newest first; `repo` is `owner/name` or a bare name |
| `teamInstallations(teamId)` | linked installations and their repos |
| `onTeamJobChange(teamId, { signal, after?, intervalMs? })` | `AsyncIterable<LiveJobEvent>` until `signal` aborts |
| `subscribeTeamJobs(teamId, listener, opts?)` | an unsubscribe function; the team's shared poller calls `listener(event)` |
| `jobEventFromRow(row)`, `jobEventFromPayload(payload, { version })` | the event for a stored row or a raw payload |

**Event.** `LiveJobEvent = { id, at, type, job }`. `type` is `job.queued`
(status queued, waiting, pending or requested), `job.started`
(in_progress) or `job.finished` (completed). `id` is `gh-<version>`:
increasing across all GitHub job changes, usable as the SSE id; pass the
client's Last-Event-ID as `after` to resume. `at` is when Quod stored the
change. A redelivery or a stale delivery produces no event.

**`LiveJob`.** `id` (GitHub job id, string), `repoSlug` (the GitHub repo
name), `repoName` (owner/name), `runNumber` (0 until the run's
`workflow_run` delivery arrives; the job is re-announced then), `workflow`,
`job`, `branch`, `sha`, `actor` (the run's actor, else the delivery's
sender), `runner` (`ubuntu-latest`, or `[self-hosted, quod]` for several
labels), `githubUrl`, `status` (`queued`, `in_progress`, `completed`),
`conclusion` (`success`, `failure` incl. timed_out and startup_failure,
`cancelled`; absent otherwise), `queuedAt`, `startedAt` (once running),
`finishedAt`, `currentStep` (`{ index, name }`, the step GitHub reported in
progress), `stepsTotal`. Additions: `source: "github"`, `runId`,
`runAttempt`, `labels`, `githubConclusion` (as GitHub sent it),
`runnerName`.

**Granularity.** GitHub sends `workflow_job` on queued, waiting,
in_progress and completed only, not per step. The step list in an in_progress
payload may be partial; it is complete with completed. So
`currentStep` from GitHub is coarse; Quod's own step events (runner hooks)
are the fine-grained source for jobs that run Quod.

**Ordering.** Deliveries can arrive late and out of order. Each job and
run row only moves forward on (run attempt, status rank, payload time):
queued-like 1, in_progress 2, completed 3. A late `in_progress` never
overwrites `completed`; a re-run (new `run_attempt`) may move a run back
to queued.

**Change feed: polling, not LISTEN/NOTIFY.** Neon's pooled connections
(PgBouncer, transaction mode) cannot LISTEN, and a direct connection held
open per instance would keep Neon from scaling to zero. So the feed polls
`workflow_jobs.row_updated_at`, with these rules:

- **One poller per team per function instance**, however many viewers.
  `subscribeTeamJobs(teamId, listener)` returns an unsubscribe function;
  `onTeamJobChange` is one consumer of it. The poller starts with the
  first subscriber and stops with the last. With no subscribers there are
  no queries, and the pool closes idle connections after 20 s, so Neon can
  scale to zero.
- **One query per poll.** The database clock is folded into the row query,
  so it also covers polls where nothing changed.
- **Back-off when idle.** Every 2 s while jobs change; after 3 idle polls,
  every 5 s; after 10, every 10 s. A change resets it.
- **Look-back.** Each poll looks back 15 s by database time and yields only
  versions it has not yielded, so a write that commits after a later one is
  not missed.
- **Resume.** `after` (Last-Event-ID) runs one catch-up query for that
  consumer, without repeating what the shared poller then delivers.

**Lost deliveries.** GitHub does not resend a failed webhook by itself.
`/api/github/reconcile` (Vercel Cron, every 15 min, `CRON_SECRET`;
`lib/github/reconcile.ts`) does three things:

- It asks GitHub to redeliver App deliveries of the last hour that have no
  successful attempt. Dedupe makes a redelivery that had in fact landed a
  no-op.
- It re-reads from GitHub's REST API any run, or the run of any job, still
  unfinished and not updated for 15 minutes. The REST objects have the
  webhook shape, so they go through the same monotonic upserts.
- It prunes runs and jobs after 30 days and delivery ids after 7.

The re-read and the prune run only when there were deliveries in the
window, so an idle App never wakes the database. Rows of suspended
installations are hidden from every query above.
