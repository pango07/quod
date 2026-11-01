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
