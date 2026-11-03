# Test-level replay in Quod Cloud

Branch `cloud/test-replay` (exp/vitest-shadow + exp/cloud-ledger-v3 +
exp/cloud-p4). `quod vitest` keeps its ledger directory (docs/vitest-shadow.md)
in Quod Cloud instead of actions/cache, and every run reports what it ran, what
it reused and why, for the dashboard.

## Client

`--cloud[=URL]` (URL defaults to `QUOD_SERVER`) turns it on; without it nothing
is sent anywhere and actions/cache works as before.

| Run | What `--cloud` adds |
|---|---|
| critical path (`--no-record`) | before: remove the local ledger, look up and fetch the nearest one; after: post the run report |
| shadow (record) | the same before; after: save the ledger (`--cloud-no-save` skips it) and post the shadow report (spot-checks) |
| `--absorb` | after: save the ledger |
| `--cloud-only restore` / `save` | only that, exit 0 whatever happens (drop-in for actions/cache restore and save steps) |

A completed restore leaves `.quod-cloud-restore.json` in the ledger directory
naming the job (`GITHUB_RUN_ID`, `GITHUB_RUN_ATTEMPT`, `GITHUB_JOB`) and the
suite; a later `--cloud` run in the same job uses that ledger instead of
fetching it again. So the restore can run in the background during the
install, off the critical path:

```sh
quod vitest --dir .quod/vitest --cloud --cloud-only restore > restore.log 2>&1 &
pid=$!
npm ci
wait $pid; cat restore.log
```

A marker from any other job (a cache, a committed file, a persistent
runner) does not match and is removed with the rest of the local ledger.

Keep values that change per run or per commit out of the test steps'
environment: vite reads all of it, so such a variable is an input of every
test file. The runner's own per-run and per-step variables (`GITHUB_SHA`,
`GITHUB_RUN_ID`, `GITHUB_OUTPUT`, `GITHUB_ARTIFACTS`, `JOURNAL_STREAM`, ...)
are not inputs.

Auth is the job's GitHub OIDC token (`permissions: id-token: write`,
audience `quodci`) or `QUOD_SERVER_TOKEN`. `QUOD_SERVER_KEY` pins the key
lookups are signed with. `--suite` (or `QUOD_VITEST_SUITE`) names the vitest
project; the default is its directory relative to the repository root.

Nothing here can fail the job or make a wrong replay: a lookup is used only
when its signature verifies for this job and every file matches its hash; the
local ledger is removed first, so any failure means no ledger (every test
file runs); a save or report that fails is a log line.

## Protocol (quod.v1, additive)

| Method and path | Who | What |
|---|---|---|
| `POST /v1/test-ledgers/lookup` | any runner of the team | `{suite, ancestors, nonce}` -> the ledger to start from (file list) and a signed statement |
| `POST /v1/test-ledgers` | runners that may save | `{suite, files}` -> 409 `missing_blobs` until every file is uploaded, then a new row |
| `POST /v1/blobs/pack` | any runner of the team | `{hashes}` -> each blob as `<hex> <size>\n<bytes>`, or `<hex> -\n` |
| `POST /v1/test-runs` | runners that may save | a `TestReport` -> a row and the `test.run` event |
| `GET /v1/teams/{team}/test-runs[/{id}]`, `/test-stats` | viewers | the dashboard's reads (cloud/docs/api.md) |

Files go up through the existing `POST /v1/blobs/missing` and `PUT
/v1/blobs/{hash}`: content-addressed, so a save uploads only what changed.

On save the server also stores the whole ledger as one bundle object (pack
format, each blob once, ledgers up to 64 MiB) and the lookup names it
(`bundle`, unsigned). A restore is then one `GET /v1/blobs/{hash}`; the client
checks the bundle's hash as it streams and every file in it against the
signed list, and falls back to the pack on any failure. Measured on
vuejs/router (257 files, 3.7 MB, GitHub-hosted runner to Railway us-east):
per-file reads 5.8-8.6 s, the pack with one bucket request per blob 3.4 s,
the bundle 1.6 s.

## Trust

A ledger decides which test results are reused without running, so whoever can
write a ledger a job reads can hide a failure from that job.

| Job (from the OIDC claims) | Reads | Saves |
|---|---|---|
| writer: push, dispatch or schedule on a writer ref (`refs/heads/main`) | `trusted` | `trusted` |
| the repository's other jobs: another branch, a same-repository PR, a merge queue | its ref's scope, then `trusted` | its ref's scope |
| on someone else's input: `pull_request_target`, `workflow_run`, ...; fork scopes | `trusted` | nothing (403); no reports either |
| fork pull requests | GitHub gives them no OIDC token | nothing |

Unprotected writer branches are allowed with a warning (DECISIONS
2026-11-02); the save answer carries the same notice the step ledger's commit
does. Rows are only added; a non-writer can never change or replace a
`trusted` row. Delegated runner tokens cannot save (they state their own
scope, and the vitest client runs inside the job); a static runner token has a
fixed scope, so only give a `trusted` one to writer jobs.

Lookup order: the newest ledger of the first commit in `ancestors` (the
checkout's `git rev-list HEAD`, then a PR's head and base) that a readable
scope has, the job's own scope first; else the newest of its own scope; else
the newest of `trusted`.

The lookup answer is signed with the receipt key, over `quod.v1
test-ledger\n` + the statement's canonical JSON (never mistakable for a step
receipt): team, repository and its GitHub id, the scope the asking job writes,
the ledger's scope, id, commit, the hash of its file list, and the client's
random nonce. The client accepts it only if it verifies under its pinned key,
names its nonce, its repository (`GITHUB_REPOSITORY`, `GITHUB_REPOSITORY_ID`),
a ledger of `trusted` or of its own ref (`GITHUB_REF`), and was asked by its
own kind of job. So a proxy that relays a job's nonce to the server under
another job's token (another repository, or a branch with a poisoned ledger
of its own) gets an answer the job refuses. Every fetched file is checked
against its BLAKE3, paths are checked (relative, under `ledger.json`,
`results/`, `blobs/`, `tests/`, `render/`, no `..`, colon, trailing dot or
space, no two equal after case folding) and created with `create_new`; the
swap into place moves `ledger.json` last.

Limits: 100,000 files and 4 GiB per ledger; a pack carries blobs up to 64 MiB
and at most 4 GiB in all, 8 reads in flight, 16 packs at once per instance
(429 beyond); a report lists at most 5,000 files and stores at most 1 MiB of
detail.

## Storage

Quod Cloud: `test_ledgers` and `test_runs` in Postgres (`team` in every key
and query), created with `CREATE TABLE IF NOT EXISTS` at start, outside the
versioned migrations, so a rollback to exp/cloud-p4 still finds the schema
version it knows. Files are blobs in the bucket with every other blob.
Self-hosted: the same tables in `teams/<team>/test-replay.sqlite`, files in the
team's CAS.

## Reports and the dashboard

The critical path reports: test files in the run, ran, replayed (skipped
because unchanged), narrowed (source changed, but not the functions the file
runs), why each file ran (the first changed input, in plain words), select and
wall time, `est_plain_ms` (the recorded durations of every file, summed: test
file time, not wall time) and `saved_ms` (the replayed files' recorded
durations), exit code, and which ledger it started from. Commit, branch, run,
workflow and repository come from the OIDC claims. The shadow reports the
same plus its spot-checks (`audit_files`); a critical run shows the
spot-checks of the shadow of its own commit, suite and scope. "Wrong results"
on the Overview is the number of spot-checks whose fresh result differed.

## Deployment (api.quodci.com)

Railway service `quod-server` builds branch `cloud/test-replay` (the one
product branch). Backend Postgres + the `quod-results` bucket
(`QUOD_BACKEND=postgres`, `DATABASE_URL` and `S3_*` as references to the
Postgres and bucket services). The volume keeps only the signing key
(`QUOD_SIGNING_KEY_FILE=/data/signing.ed25519`, fingerprint
`ed25519:11142c7cc0d0dd7f`) and upload staging (`QUOD_DATA_DIR=/data/cloud`);
the pre-cut-over SQLite data under `/data/teams` is never read (kept as an
archive). Healthcheck timeout 1800 s; a warm-up listener answers `/healthz`
(`starting`, with progress) while boot runs, and 503 elsewhere.

Rollback: to SQLite on this build, `QUOD_BACKEND=sqlite`, `QUOD_DATA_DIR=/data`
and remove `QUOD_SIGNING_KEY_FILE` (the old data, as of the cut-over); to the
previous build, also remove `DATABASE_URL` and `S3_*` and redeploy deployment
`6d718894` (exp/cloud-ledger-v3). Results recorded on Postgres since are not
on the volume; runners re-execute those steps (nothing wrong replays).

## Open

- Retention: ledger rows and blobs accumulate (as the step ledger's do;
  cloud-ledger.md open question 12).
- Blob reads are team-wide (open question 4): any runner of the team can fetch
  a blob it can name.
