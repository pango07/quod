# quod.v1: the contract between the dashboard and quod-server

The dashboard reads everything through one TypeScript interface, `CloudApi`
in `lib/api/types.ts`. quod-server (`crates/quod-server`, its dashboard
routes in `src/dash.rs`; branch `exp/cloud-ledger`) serves the HTTP side.
This page is the contract between the two: what the server serves today,
what the dashboard does with it, what does not match yet, and what the
server should change.

The contract is tested. `lib/api/contract/quod-server/` holds quod-server's
own answers, recorded from the server by its test harness (`dump.rs` there
says how to record them again), and `lib/api/contract.test.ts` runs them
through the dashboard's HTTP adapter: every endpoint, the 4xx and 501
bodies, and the event stream. When the server changes a field, an enum or an
error shape, record again and these tests say what broke.

## Adapters and the switch

| `QUOD_API` | Quod's data | GitHub's data |
| --- | --- | --- |
| unset | none (`lib/api/none.ts`): "Appears once Quod runs" | Postgres, as always |
| `http` | quod-server at `QUOD_SERVER_URL` (`lib/api/http.ts`) | Postgres |
| `fixture` | sample data for the demo session's sample team only (`lib/api/fixture.ts`) | Postgres for real teams |

`lib/api/composite.ts` wraps whichever it is and adds GitHub's data. Any
other value, `QUOD_API=http` without a usable `QUOD_SERVER_URL` (https; plain
http only for `localhost` or a private `*.internal` host), or a malformed
`QUOD_SERVER_TEAMS` is misconfigured, and `next start` refuses to boot
(`lib/api/mode.ts`). The stub's old `QUOD_API_URL` is refused by name.
Switching between the three is safe at any time: nothing is stored on the
dashboard's side for Quod's data.

## Protocol (as quod-server serves it)

- Paths under `/v1`, on `QUOD_SERVER_URL`. Every request sends
  `quod-protocol: quod.v1` (the server refuses anything else with 400
  `protocol`; only `/v1/info` and `.../events` may omit it) and
  `authorization: Bearer <token>`. Nothing else: no cookie, no forwarded
  header; `credentials: "omit"`, redirects are errors. Server-side only; no
  token reaches a browser.
- JSON only. Times are RFC 3339 UTC strings, durations are seconds (floats),
  saved time is whole minutes, shares are 0..1.
- Errors: `{ "error": <code>, "message": <text> }` (plus `missing` for
  `missing_blobs`). Codes: `protocol`, `unauthorized`, `forbidden`,
  `not_found`, `bad_request`, `conflict`, `missing_blobs`,
  `not_implemented`, `internal`.
- Path ids: `^[A-Za-z0-9._-]{1,120}$`, never `.` or `..` (the dashboard
  sends at most 100). Step keys are 64 hex digits (the `b3:` form has a
  colon). Run numbers are integers.
- `/v1/teams/{team}`: quod-server's own team id. The dashboard's slugs are
  per viewer and never sent (see "Viewer auth").
- Repo slugs are quod-server's: the repository name lowercased, characters
  other than letters, digits, `-` and `_` turned into `-`, and `owner-name`
  when two owners share a name or the name is a reserved page
  (`repositories`, `runs`, `trust`, `runners`, `usage`, `settings`,
  `connect`). The dashboard maps them to its own (below).

### How the dashboard treats each answer (`lib/api/http.ts`)

| Answer | Dashboard |
| --- | --- |
| 2xx | Shape-checked (the fields pages rely on); a body that is not quod.v1 is "unavailable", never a crash in a component |
| 404 on an entity (repo, run, step) | `null`: the page's not-found, or GitHub's data alone |
| 404 on a team-level read | the team has no Quod data yet (empty), so a team the server does not have is not an error |
| 401 | sign in again only when the viewer's own token has expired; otherwise (a static token, or a fresh token refused) "unavailable": the server's viewer auth is misconfigured, and a viewer is never sent round a sign-in loop |
| 403 | not found (existence does not leak) |
| 501 | `ApiNotAvailableError`: the section says "Not available yet", with the server's reason |
| 400, 409, 5xx, timeout (8 s), network | `ApiUnavailableError`: GitHub's data with a "Quod data unavailable" note, or a section card |

GETs, and only GETs, are retried twice (250 ms, 750 ms) on a network error,
a timeout, 429, 502, 503 or 504. A 500 (`internal`) is not retried.

Pages never crash on quod-server: Overview, a repo, Runs and the GitHub run
page fall back to GitHub's data with a note (`QuodStatus` on `Overview`,
`RepoDetail`, `Activity` and `listRuns`); run, step, Steps, Trust, Runners
and repo settings render a section card (`lib/api/section.ts`,
`components/section.tsx`); the sidebar's health and the ⌘K index fall back
to zeros and GitHub's repos; actions answer with a message.

## Viewer auth

The dashboard calls quod-server only from its server side, after its own
membership check (`lib/dal.ts`: the viewer belongs to the team's WorkOS
Organization), with credentials per team (`lib/api/credentials.ts`):

1. **Static viewer token** (quod-server as built today). `QUOD_SERVER_TEAMS`
   (secret) maps a WorkOS org id to the server's team id and a viewer
   token made with `quod-server hash-token`:
   `{"org_01H...": {"team": "acme", "token": "<viewer token>"}}`.
   The dashboard has already checked membership; the token is the team's,
   not the viewer's. Use a `member` token (the server enforces nothing for
   admins yet).
2. **The viewer's WorkOS access token** (`QUOD_SERVER_VIEWER_AUTH=workos`),
   once quod-server verifies them ("Server changes needed", 1). The path's
   team id is then the team's WorkOS org id.

A team with neither is not sent to quod-server: its Quod numbers are empty.
Runners are unaffected: they keep GitHub OIDC (audience `quodci`) or their
runner tokens; a viewer token is refused as a runner and a runner token on
a dashboard path is 403.

## Who may do what

WorkOS roles: `admin` and `member`. The server enforces these; the UI only mirrors them.

- Read anything in the team: any member.
- Re-run without Quod: any member, as anyone with access can re-run a workflow in GitHub.
- Change proofs needed for a step: admins.
- Save the team's Connect choice: admins. Members can still view both setups.

## Endpoints

"Served" is quod-server on `exp/cloud-ledger`.

| Method and path | Body / query | Returns | Served | Dashboard |
| --- | --- | --- | --- | --- |
| `POST /v1/teams/resolve` | `{ orgs: [{ orgId, name }] }` | `{ teams: Team[] }` | 501 | not called: the dashboard makes team slugs itself (`lib/api/teams.ts`) |
| `GET /v1/teams/:team/health` | | `TeamHealth` | yes | sidebar; zeros on failure |
| `GET /v1/teams/:team/overview` | `?range=7d\|30d\|90d` (else 400) | `Overview` | yes | Overview, Usage, Repositories |
| `GET /v1/teams/:team/repos` | | `{ repos: RepoSummary[] }` | yes | repo list, slug map |
| `GET /v1/teams/:team/repos/:repo` | | `RepoDetail` | yes | Repository |
| `GET /v1/teams/:team/runs` | `?repo=&limit=` (default 50, at most 500; no paging) | `{ asOf, runs }` newest first | yes | Runs |
| `GET /v1/teams/:team/repos/:repo/runs/:number` | | `RunDetail` | yes | Run; a GitHub run's step marks |
| `POST /v1/teams/:team/repos/:repo/runs/:number/rerun` | `{ quod: false }` | `{ accepted, message }` | 501 | "Not available yet" message |
| `GET /v1/teams/:team/steps` | `?repo=` | `{ steps: StepSummary[] }` | yes | Trust, repo Steps |
| `GET /v1/teams/:team/repos/:repo/steps/:key` | | `StepTrust` | yes | Step trust |
| `PUT /v1/teams/:team/repos/:repo/steps/:key/proofs` | `{ need }` (2..10) | `StepTrust` | 501 | "Not available yet" message |
| `GET /v1/teams/:team/runners` | | `{ runners }` | 501 | none from Quod; GitHub's "Seen in your CI" |
| `GET /v1/teams/:team/connect` | | `ConnectSetup` | 501 | the built-in setup |
| `PUT /v1/teams/:team/connect` | `{ store, host }` | 204 | 501 | "Not available yet" message |
| `GET /v1/teams/:team/search-index` | | `SearchIndex` | yes | ⌘K, plus GitHub's repos |
| `GET /v1/teams/:team/activity` | `?repo=` (ignored) | `Activity` (`jobs: []`, `cursor`) | yes | "Now running" with GitHub's jobs |
| `GET /v1/teams/:team/events` | `?scope=`, `Last-Event-ID` | `text/event-stream` | yes | everything live |

## Shapes, by screen

The types are `lib/api/types.ts`; the recorded bodies in
`lib/api/contract/quod-server/` are real examples.

- **Overview** (`Overview`): `totals` (saved minutes, steps replayed of
  total, est. cost saved and the per-minute price, wrong replays, spot checks
  and how many were identical), `days` (one point per day: replayed and ran
  minutes), `repos` (replay rate, saved minutes, last run, paused steps,
  `hasQuodData`), `trust` (proven, proving, watching, spot checks passed,
  paused steps), `runners`.
- **Repository** (`RepoDetail`): summary, default branch, `latestRun`
  (a full `RunDetail`), `runTimes` (last runs: with Quod vs recorded time
  without, oldest first), `steps` (`StepSummary`: job, command, trust state,
  replay rate, ran and replay seconds, saved minutes).
- **Run** (`RunDetail`): commit, branch, actor, runner, took vs would have
  taken, counts, and `jobs[].steps[]`, each with `outcome` (replayed or ran),
  optionally `index` (GitHub's step number), and `why`, a tagged union:
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

## The event stream (quod.v1)

`GET /v1/teams/{team}/events?scope=team|repo|run|step` streams Server-Sent
Events. Scope parameters: `scope=repo&repo=`, `scope=run&repo=&run=`,
`scope=step&repo=&key=`; the default is the whole team. Same authorization
as every other endpoint.

A client first reads `GET /v1/teams/{team}/activity`. Its `cursor` is the
stream position that snapshot is consistent with (quod-server: the newest
event id, `"0"` when there is none): subscribe with it as `Last-Event-ID`
and the stream starts exactly after the snapshot. Without `Last-Event-ID`
the server starts at its newest event.

Each message is `id: <event id>`, `event: <type>`, `data: <LiveEvent JSON>`
(the data carries `id` and `type` too). quod-server's ids are its outbox
row ids: increasing integers, as strings. It sends `: ping` every 15 s and,
after `QUOD_SSE_HOLD_SECS` (default 240), `event: bye` with `data: {}`.

The HTTP adapter's `subscribe` reconnects by itself, with the last id it saw
as `Last-Event-ID`: at once after `bye` (never in a tight loop), with
back-off (1 s doubling to 30 s) after a failure, and when a connection sends
nothing, not even a ping, for 45 s. A refused viewer (401/403) ends the
stream; a team or endpoint the server does not have (404/501) holds it open
quietly and looks again every minute. Events it cannot read (an unknown
type, broken JSON, missing fields) are skipped, and `bye` is never passed on
as an event.

| Event | Carries | Source | Served |
| --- | --- | --- | --- |
| `job.queued`, `job.started`, `job.finished` | `LiveJob` | GitHub App webhooks, read by the dashboard itself | n/a |
| `run.started`, `run.finished` | `RunSummary` | Quod runner hooks | not yet |
| `step.started` | repo, run, job id, step index and name, step key | Quod runner hooks | not yet |
| `step.finished` | the `RunStep`, `jobId`, `index`, `stepKey` | Quod runner hooks | yes |
| `trust.changed` | step key, new `TrustState`, the `TrustEvent` | quod-server's trust decisions | yes (`proven` and `paused` only) |
| `stats.updated` | repo, day, a `StatsDelta` | quod-server, as steps finish | not yet |

The dashboard re-serves the merged stream to the browser from
`GET /[team]/events` (a route handler with the same session and membership
check as the pages, closed after about 4 minutes, under the 300 s function
limit). On the sample team the simulator (`lib/live/sim.ts`) produces the
same events from the clock.

Members, invitations, SSO and directory sync are not in this API: they live
in WorkOS and the app calls WorkOS directly (`lib/auth/directory.ts`).

## Contract mismatches found, and how each was fixed

Found by recording quod-server's answers and running them through the
dashboard (`contract.test.ts`). "Dashboard" means fixed on this side;
"server" means listed under "Server changes needed".

| # | Mismatch | Fixed on |
| --- | --- | --- |
| 1 | `POST /v1/teams/resolve` is 501, and every page resolved its team through it: on the HTTP adapter every team page would have failed | dashboard: slugs are made locally (`lib/api/teams.ts`), the endpoint is not called |
| 2 | The path's team was the dashboard's slug (per viewer, from the org's name); the server's is its configured team id | dashboard: credentials per team carry the server's team id; server (1, 2): teams by WorkOS org id |
| 3 | The stub sent the viewer's WorkOS token; the server accepts only static viewer tokens | dashboard: `QUOD_SERVER_TEAMS` static tokens now, WorkOS tokens behind `QUOD_SERVER_VIEWER_AUTH`; server (1) |
| 4 | Repo slugs: the server lowercases, turns `.` into `-` and prefixes reserved names with the owner; the dashboard keeps GitHub's name and leaves reserved ones out. `acme/My.App` was `my-app` on one side and `My.App` on the other, so Quod's numbers and GitHub's runs would not meet | dashboard: repos meet by full name, and quod-server's slugs are mapped both ways (`lib/api/repo-map.ts`); server (3, optional) |
| 5 | Error bodies (`{error, message}`) were ignored; 501 and 400 `protocol` were plain errors that crashed the page | dashboard: typed errors and section cards |
| 6 | Any 401 sent the viewer to sign in, so a server that refused good tokens would loop them through sign-in | dashboard: sign in only when the viewer's own token has expired |
| 7 | A team the server does not have is 404 on every endpoint; the stub threw ("not found") on team-level reads | dashboard: no Quod data for that team (empty) |
| 8 | The stream's `event: bye` (`data: {}`) was yielded as a LiveEvent; the stub never reconnected | dashboard: `bye` ends a connection, the adapter reconnects with `Last-Event-ID` |
| 9 | `step.finished.stepKey` is `""` when a step has no key; quod.v1 says `null` | dashboard normalizes; server (6) |
| 10 | `RunJob.id` and `step.finished.jobId` are the workflow's job id (`GITHUB_JOB`, "test"), not GitHub's job; matrix legs of one job are one Quod job; `RunStep` has no step number | dashboard: matches jobs by name and steps by name (or `index` when sent) (`lib/api/github-merge.ts`); server (4) |
| 11 | Runs are keyed by run number per repository, but GitHub numbers runs per workflow, and attempts are merged | dashboard: a GitHub run uses Quod's run only if its `githubUrl` names that run id; server (5) |
| 12 | `language` is `""`, `defaultBranch` always `"main"` | dashboard hides an empty language and takes the default branch from GitHub when it has it; server (8) |
| 13 | `runners` and `connect` are 501 | dashboard: none from Quod (GitHub's runners show), the built-in setup |
| 14 | The base URL was `QUOD_API_URL`, with no explicit off switch; unset refused to boot | dashboard: `QUOD_API=http` + `QUOD_SERVER_URL`, unset is "no Quod server" |
| 15 | `trust.changed` carries placeholders (`runs: [0, 0]`, `run: 0`); spot checks have `sourceRun: 0`; `changed` is reported as `first-run`; the divergence excerpt is not a line diff | shown as sent; server (7) |
| 16 | `runs` has no paging (only `limit`, at most 500) | dashboard clamps `limit`; GitHub's runs page and paginate |
| 17 | `overview.days` has only the days that had steps; the chart expects one point per day of the range (it drew one day across the whole width) | dashboard fills the missing days with zeros (`fillDays`); server (11) |

## Server changes needed

For quod-server (not done here; the Rust branch is unchanged). In order of
what blocks what.

1. **Viewer auth with WorkOS access tokens** (P5 in docs/cloud-ledger.md).
   A bearer that is a JWT and not a configured static token, on a dashboard
   path, is a WorkOS access token. Verify:
   - signature: RS256 only, keys from `https://api.workos.com/sso/jwks/<WORKOS_CLIENT_ID>`
     (cached; refetch for an unknown `kid` at most once a minute, as for
     GitHub OIDC);
   - `iss`: exactly the configured issuer, the value AuthKit's access
     tokens carry (`https://api.workos.com/user_management/<WORKOS_CLIENT_ID>`,
     or the custom auth domain if one is set; the dashboard's `WORKOS_ISSUER`
     when set is the same value);
   - `exp` and `iat`, 60 s leeway; `sub` (the WorkOS user id) present;
   - no `aud` check: WorkOS sets no Quod audience, and the JWKS is per
     client id, so only this app's tokens verify.
   Then the team: the path's `{team}` is a WorkOS org id; find the team
   whose `workos_org_id` (new in `TeamConfig`) it is, else 404. Membership:
   if the token's `org_id` is that org, the viewer is a member with the
   token's `role`; otherwise ask WorkOS
   (`GET /user_management/organization_memberships?user_id=<sub>&organization_id=<org>&statuses=active`,
   `WORKOS_API_KEY` on the server), cached at most 60 s per (user, org);
   none is 404, never 403. The token's `org_id` alone is not enough: a
   viewer opens any of their teams by URL. Role `admin` for the admin-only
   endpoints (`PUT .../proofs`, `PUT .../connect`), else 403. A bad or
   expired token is 401 `unauthorized`, never a redirect. Config:
   `"viewers": {"workos": {"client_id": "...", "issuer": "..."}}` plus
   `WORKOS_API_KEY` in the environment.
2. **Teams keyed by WorkOS org id** in paths (`workos_org_id` in
   `TeamConfig`, and the GitHub App's installation links for runners, P5).
   `POST /v1/teams/resolve` can stay 501 or go: the dashboard owns slugs.
3. **Repo slugs** (optional; the dashboard maps them): either the
   dashboard's rule (GitHub's name as is, `owner-name` only when two owners
   share it) or `fullName` next to every `repoSlug` in runs, steps, events
   and the search index, so no client has to look slugs up.
4. **GitHub's job and step identity**: `RunJob.id` and `step.finished.jobId`
   should be GitHub's job id (`${{ job.check_run_id }}`, which
   `quodci/setup` can pass as an environment variable), with `name` GitHub's
   display name, so matrix legs are separate jobs; `RunStep.index` should be
   GitHub's 1-based step number (document `QUOD_STEP_INDEX` as exactly
   that) and be sent in `RunDetail`, not only in events.
5. **Runs keyed by GitHub run id and attempt**, not run number per
   repository (two workflows share run numbers). Add `runId` and
   `runAttempt` to `RunSummary`, and always put the run id in `githubUrl`.
6. `step.finished.stepKey`: `null`, not `""`, for a step without a key.
7. Events: `step.started`, `run.started`/`run.finished` and
   `stats.updated`; real run numbers in `trust.changed`
   (`proven.runs`, `paused.run`), a spot check's `sourceRun`, `changed`
   with its first changed input, and the divergence as lines.
8. `defaultBranch` and `language` from the repository (the GitHub App
   knows both); `RepoSummary.runner` from the runs.
9. **Writer trust** (DECISIONS 2026-11-02): record each trusted write's
   ref and `ref_protected` claim, and expose them so the dashboard can say
   where trusted results come from: per repo
   `trustedWrites: { ref, protected: true | false | null, lastAt }` on
   `/repos` and `/repos/:repo`, and `requireProtectedRef` (team, with a
   per-repo override) on `/health` and `/repos/:repo`. With
   `requireProtectedRef` on, the dashboard drops the warning for that repo;
   until the server sends it, the dashboard warns from GitHub's branch
   protection alone (below). Also the per-repo opt-in itself
   (`require_protected_ref` per repository in `TeamConfig`).
10. Keep the error body (`error`, `message`) and the codes: the dashboard
    relies on `protocol`, `unauthorized` and `not_implemented`.
11. `overview.days`: one point per UTC day of the range, zeros included,
    oldest first (as the fixture and the chart have it).

## The writer-trust warning (branch protection)

DECISIONS 2026-11-02: trusted writes do not require a protected branch;
the dashboard warns instead. The dashboard reads each linked repo's default
branch protection through the GitHub App (installation token):
`GET /repos/{o}/{r}` (default branch), `GET .../branches/{b}/protection`
(classic protection; Administration: read; 404 "Branch not protected"
means none) and `GET .../rules/branches/{b}` (rules from repository and
organization rulesets; Metadata). Protected if either applies (as GitHub's
OIDC `ref_protected` claim means), unprotected only when both were read
and neither does, else unknown (`lib/github/protection.ts`).

It is cached in `repo_protection` (`db/migrations/0005_branch_protection.sql`,
**not applied yet**): read while someone views the team, in the background,
when a row is missing, older than 6 hours, or marked stale by a
`branch_protection_rule`, `repository_ruleset` or `repository` (default
branch changed, renamed, archived, transferred) webhook; at most 10 repos per
view, one instance per repo (a claim, abandoned after 2 minutes), stopping at
a rate limit. Before the migration is applied nothing is read and no badge
shows; webhooks for these events are accepted and do nothing.

On screen: a badge per repo (Repositories, Overview, the repo page:
protected, not protected, protection unknown); a warning on the repo and a
banner on every team page while Quod has data for a repo whose default
branch is unprotected ("Anyone who can push to main can record results that
every CI job will reuse; protect main or turn on require_protected_ref") or
unknown. "Learn more" opens `/[team]/trust#protected-branch`.

## Go-live

When quod-server is deployed:

1. Vercel (quod-cloud), Production:
   - `QUOD_API` = `http` (not secret)
   - `QUOD_SERVER_URL` = the server's https base URL (not secret)
   - `QUOD_SERVER_TEAMS` (**secret**): while the server has only static
     viewer tokens, `{"<WorkOS org id>": {"team": "<server team>", "token": "<viewer token>"}}`
   - later, `QUOD_SERVER_VIEWER_AUTH` = `workos` (not secret), once the
     server verifies WorkOS tokens (Server changes needed, 1); then
     `QUOD_SERVER_TEAMS` can go.
   Then redeploy. Unset `QUOD_API` to switch Quod's data off again.
2. The database: apply `0005_branch_protection.sql` (`npm run db:migrate`,
   after the product owner approves the SQL).
3. The GitHub App's settings, by hand (Permissions & events): Administration
   read (done), and subscribe to **Branch protection rule**, **Repository
   ruleset** and **Repository**. Push is not needed (it would also need
   Contents: read, which the App does not have); a change these events do
   not report (an organization ruleset, when the App cannot receive the
   organization's events) shows within 6 hours.
4. quod-server (Railway), for WorkOS viewer auth later: `WORKOS_CLIENT_ID`
   (not secret), `WORKOS_API_KEY` (**secret**), the issuer.

To try it locally against the recorded answers, without a server:

```sh
node scripts/quod-server-stub.mjs            # http://localhost:8787, dev only
QUOD_API=http QUOD_SERVER_URL=http://localhost:8787 \
  QUOD_SERVER_TEAMS='{"org_demo_jvaught":{"team":"acme","token":"acme-viewer-token"}}' npm run dev
```

## Two sources in one stream: the composite adapter

`lib/api/composite.ts` wraps the inner adapter (fixture, HTTP or none). For
a team with a linked GitHub installation (and `DATABASE_URL` set), "Now
running" (`getActivity`) and the `job.*` events come from the GitHub App's
tables, whatever `QUOD_API` is; Quod's run, step, trust and stats events
still come from the inner adapter.

Quod and GitHub meet per repository by full name (`owner/name`,
case-insensitive), never by slug. Overview, the repo list and the ⌘K index
show one row per repository: Quod's numbers when quod-server has data for it
(`hasQuodData: true`), with GitHub's latest run and branch protection next to
them; GitHub's run and "Appears once Quod runs" when it has not. So the empty
states switch to real numbers by themselves the first time quod-server has
data for a repo. The dashboard shows GitHub's slug (a repo name two linked
owners share is `owner-name`); quod-server's slugs are mapped both ways for
every call and answer, events included (`lib/api/repo-map.ts`). A GitHub
run's page asks quod-server for the same run and marks each step Quod
handled as Replayed or Ran (`lib/api/github-merge.ts`).

The sample team keeps the simulator and never gets GitHub data. Which team
is the sample team is decided by WorkOS org id (the demo session's
`org_demo_quodci`, and only with `QUOD_API=fixture`), never by slug or name:
a real org named "quodci" is a real team at /quodci, with its own GitHub
data or the Connect card. `Activity.github` says where a team stands:
`linked`, `unlinked` (with `canInstall`), `not-configured` (no database), or
`unavailable` (the database did not answer; pages still render and say so).
`quod` (`QuodStatus`) says the same for quod-server: `connected`, `off`, or
`unavailable`.

### Cursors

Every stream position is opaque to the browser, which only echoes it back as
`Last-Event-ID`. The formats:

| Source | Event id / cursor |
| --- | --- |
| Simulator | `<ms>-<cycle>-<seq>`; a snapshot's cursor is `<ms>-999999999-9999` (after every event at or before that millisecond) |
| quod-server | its outbox row ids: increasing integers, as strings; a snapshot's cursor is the newest id ("0" when none) |
| GitHub App | `gh-<version>`: `workflow_jobs.version` (the `live_version` sequence) |
| Merged (linked team) | `gh-<version>` or `gh-<version>,<quod id>`: both positions after this event (`lib/live/cursor.ts`) |

A linked team's snapshot cursor is `gh-<max version the team can see>`,
read before the job lists, so a change committed during the snapshot is
delivered by the stream (possibly again: repeats are harmless). A new stream
also replays the team's job changes of the last 30 s (by `row_updated_at`),
which covers a write that committed out of version order while the snapshot
was taken; the merged stream drops any event whose version is not newer than
the last one it sent for that job, so an older state never overwrites a newer
one. Job state in events is the whole job, so applying one twice is a no-op.

GitHub numbers steps from 1; `LiveJob.currentStep.index` counts from 0, and
the composite converts.

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
`lib/github/reconcile.ts` heals them. A pass does two things:

- **Redeliver.** It lists the App's failed deliveries (App JWT,
  `status=failure`) and asks GitHub to send again each GUID that Quod's
  `deliveries` table never recorded. Dedupe makes a duplicate harmless.
- **Re-read.** Any run, or the run of any job, still unfinished and not
  updated (or checked) for 15 minutes is re-read from GitHub's REST API.
  The REST objects have the webhook shape, so they go through the same
  monotonic upserts. `reconciled_at` rotates long healthy runs to the back.

Two triggers, because Vercel Hobby allows one cron a day:

- **Daily cron.** `/api/github/reconcile` (`vercel.json`, `CRON_SECRET`)
  covers every team, the last 25 hours of failures, and retention (runs and
  jobs after 30 days, delivery ids after 7).
- **Lazy, while someone watches.** A team's shared poller calls the
  reconcile hook on its first poll and then every 15 minutes per instance
  (`PollOptions.reconcile`, default `lib/github/lazy-reconcile.ts`). The
  hook runs in the background (Vercel `waitUntil`; the stream never awaits
  it) and first claims the team's `reconcile_state` row with one atomic
  statement:

  ```sql
  INSERT ... ON CONFLICT (key) DO UPDATE SET last_run_at = now()
  WHERE last_run_at < now() - interval '15 minutes' RETURNING key
  ```

  So across all instances, a team is reconciled at most every 15 minutes,
  scoped to its own installations, with the last 3 hours of failures.

Nobody watching means nothing runs between daily passes, so Neon can scale
to zero. Rows of suspended installations are hidden from every query above.
