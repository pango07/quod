# The central ledger: quod.v1, quod-server and Quod Cloud

One ledger and one result store for every runner of a team, so a fresh
GitHub-hosted runner can replay a result recorded anywhere. This page is the
design and the build record for branch `exp/cloud-ledger` (stages P0 to P2)
, `exp/cloud-ledger-v2` (the same on trust v2 and ledger v6, and the
go-live on Railway below) and `exp/cloud-ledger-v3` (the same on main
5a4c18b: ledger v8, deferred audits, the network memory; see "On main's
semantics (v3)", deployed at api.quodci.com) and `exp/cloud-p4` (P4:
Postgres and an S3-compatible bucket, the importer, and the v3 review's
start-up fixes). DECISIONS.md has one line per choice; this page
explains how they fit.

## Status

| Stage | What | State |
|---|---|---|
| P0 | Ledger and CAS behind traits; trust transition as a pure function | built |
| P1 | quod-wire (quod.v1), quod-server (SQLite + filesystem), the client, dashboard reads | built |
| P2 | Runner auth: GitHub OIDC, scopes, static tokens, team isolation; viewer auth: WorkOS access tokens, static viewer tokens | built |
| Go-live | One instance on Railway: SQLite + files on a persistent volume, instance lock, signing key generated on the volume | deployed at api.quodci.com (v2, then v3; see "Go-live on Railway") |
| P3 | Encryption: keyed content addressing per team, convergent AEAD | designed below |
| P4 | Hosting: Postgres + S3-compatible store, the SQLite importer | built on exp/cloud-p4, tested against Postgres and MinIO; not deployed (see "Cut-over runbook") |
| P5 | Dashboard setup, team links from the GitHub App, `quodci/setup` | designed below |

## Open core: one protocol, three deployments

| Deployment | Ledger | Blobs | Who runs it |
|---|---|---|---|
| Local (today) | SQLite in the state dir | the state dir's CAS | `quod run`, the runner hooks |
| Self-hosted quod-server (open source) | SQLite per scope | a directory per team | the team |
| Quod Cloud | Postgres | S3-compatible object store | us, at api.quodci.com |

All three run the same trust code (quod-trust) on the same rows. The two
server deployments speak the same protocol, quod.v1 (crate quod-wire), and
differ only in the `LedgerStore` and `BlobStore` implementations behind it.

## P0: the trait split

- `quod_cas::BlobStore`: has, read, open a reader, put bytes, put a file
  (streamed). `Cas` implements it. The contract is content addressing: an
  implementation never stores bytes under a name it did not compute from
  them.
- `quod_ledger::LedgerStore`: the reads and writes the trust layer and the
  lookup make (find_match, manifest, latest_result, results_for_run,
  put_step_key, run rows, trust/proof/audit rows) plus `atomically`.
  `Ledger` (SQLite) implements it by delegation. `atomically` is the contract
  an implementation must keep exactly: all or nothing, and no writer
  interleaves (SQLite BEGIN IMMEDIATE; Postgres: one transaction that locks
  the key's trust row, `SELECT ... FOR UPDATE`, or a transaction-scoped
  advisory lock on (team, scope, key) when the row does not exist yet).
- `quod_trust::Trust<L = Ledger, B = Cas>`: generic over both, so the server
  runs this exact code. Existing callers compile unchanged.
- The trust transition is pure. `execution_transition(policy, input)` takes
  the stored facts (trust row, the manifest's proof row, recent divergences)
  and what the comparisons found (`Compared`: divergence items, the run
  compared with, byte identity, the previous execution's fingerprint under
  the current rules or that its result is gone) and returns the rows to
  write: reset every proof of the key or not, this manifest's proof row, the
  trust row, the audit row, and the verdict. `divergence_transition` and
  `replay_transition` do the same for a divergence without a result and a
  served replay. `decide` was already pure. The I/O shell (`Trust::record_*`)
  gathers, calls, and writes in the order and transaction it always did.
- `quod_trust::StepLedger`: the step-grain interface `quod run` and the hooks
  use: `claim` (look up, decide, and on a replay record it with its run row),
  `commit_execution`, `commit_divergence`, `commit_incomplete`, `record_run`,
  `put_step_key`. `LocalLedger` makes exactly the calls the two callers made
  inline before; quod-wire's `RemoteLedger` makes them over HTTP.

"No behavior change" is checked by the existing suites: every quod-trust,
quod-ledger and quod-cas test, the CLI's trust, step, execute and replay
suites, and the workspace lint gate pass unchanged.

## P1: quod.v1

### Encoding

JSON over HTTPS, paths under `/v1`, `quod-protocol: quod.v1` on every
request and response. Why JSON and not protobuf or a binary framing:

- Every value already has a canonical JSON form. Manifests, results and step
  keys are hashed as canonical JSON (`quod_cas::canonical_json`), and the
  receipt signs a hash of the result's canonical JSON. A second encoding
  would be one more thing to keep byte-for-byte in step.
- The dashboard is TypeScript and reads the same bodies.
- A captured exchange is readable, which matters for a protocol whose job is
  to be auditable.
- Volume is in blobs, which never go as JSON: they are raw bytes under their
  hash. The one large JSON body is the claim's tree (below), a P1 shortcut
  the lookup index replaces.

The server speaks HTTP/1.1 and HTTP/2 (h2c, or h2 behind a TLS terminator);
the runner client is HTTP/1.1 (ureq, blocking, rustls), which is what a CLI
step needs: a handful of sequential calls per step. Nothing in the protocol
depends on HTTP/2.

Versioning: within v1, fields are only added, each with a default, and
readers ignore what they do not know. A change that could make an older
client replay something it should not is v2 (a new path prefix and header
value). A server answers a request that does not name quod.v1 with 400
`{"error":"protocol"}`.

### Endpoints (runners)

| Method and path | Body | Answer |
|---|---|---|
| `POST /v1/claim` | step (key, cmd, ignore), policy, the workspace tree, the run row a replay records | candidates, closest mismatch, the hit (scope, manifest, result), `claimed` (replay with facts, or execute with a reason), a signed receipt on a replay |
| `POST /v1/commit` | step, policy, manifest, result, the hit it was compared with (`HitRef`), reason, run row | the verdict; 409 `missing_blobs` naming any blob not uploaded |
| `POST /v1/divergence` | a hit's execution behaved otherwise, without a result | the verdict |
| `POST /v1/incomplete` | the step did not finish | the banner line |
| `POST /v1/runs`, `POST /v1/step-keys` | a run row; a key's canonical JSON | `{}` |
| `POST /v1/blobs/missing` | hashes | the ones the team's store lacks |
| `PUT /v1/blobs/{hash}` | the whole blob | 201 |
| `PATCH /v1/blobs/{hash}` | a chunk at `upload-offset`; `upload-complete: true` on the last | 202 with the new offset, 201 when done, 409 with the server's offset on a mismatch |
| `HEAD /v1/blobs/{hash}` | | 200 present; 404 with `upload-offset` of a partial upload |
| `GET /v1/blobs/{hash}` | | the bytes, streamed |
| `GET /v1/info` | | protocol, version, the signing key |
| `GET /v1/keys` | | the public key(s) receipts are signed with, each with its fingerprint (no auth, no protocol header needed) |
| `GET /healthz` | | `{"status":"ok", protocol, version, keys}` |

The trust transitions are trust v2's (docs/trust-v2.md): proofs 1, the
shadow phase and its 1/k decay to the audit floor, determinism classes
(`volatile` counts), and at a quarantine the key's earlier replays are
marked suspect in the scope's ledger. `execution_transition` and
`divergence_transition` return all of it, the shell writes it, and the
server's ClaimReplay and CommitExecution call the same functions over the
scope's ledger, so a server's verdicts are a laptop's (test S9 runs one
scenario both ways and compares every verdict and trust row). The policy's
`shadow` travels with every call; the server applies the stricter of the
client's and the team's.

ClaimReplay is atomic on the server: inside one `atomically` on the scope's
ledger it runs `find_match`, `decide`, and on a replay `record_replay` and
the run row. Two runners claiming at once are serialized, so an audit slot,
a quarantine or a replay counter is seen by exactly one (test S2: eight
concurrent claims under a 50% audit give one replay and seven audits).
CommitExecution is `Trust::record_execution` plus the run row, manifest and
result in one transaction (test S2: eight concurrent identical commits chain
their streaks 1..n with no lost update).

Blobs go up before the commit (`missing`, then PUT or resumable PATCH) and
are streamed to disk, hashed as they arrive, and stored only under the hash
they have. A commit names its hit by `HitRef` (scope, manifest id, run id);
the server reloads it from a scope the runner may read, never trusting the
client's copy.

### Signed results

The server signs every replay it serves with Ed25519: the statement is
`{protocol, team, scope, step_key, manifest_id, run_id, result, issued_at,
nonce}` where `result` is the BLAKE3 of the result's canonical JSON and
`nonce` is a value the runner made for this one claim. The runner verifies
the signature under the key it trusts (`QUOD_SERVER_KEY`, or the key from
`/v1/info` over the same TLS connection when not pinned) and that the
statement names exactly the key, manifest, run and result bytes it was
handed, and its own nonce: a genuine answer to an earlier claim, recorded
and played back (by a proxy, a cache, anyone on the path), does not verify,
so a result the server has since stopped serving (a quarantined key) cannot
come back that way. Plain `http://` is accepted only for a server on the
same machine (127.0.0.1, localhost, [::1]). The signing key is the key
file on the data dir's persistent volume (`QUOD_SIGNING_KEY_FILE`, default
`$QUOD_DATA_DIR/signing.ed25519`), generated there at first boot and reused
on every later one, or `QUOD_SIGNING_KEY` when self-hosting sets it; never
a key per instance, which would make every runner that pinned the last one
refuse its replays ("Go-live on Railway" has the details).

What a signature buys: a runner that pins the key replays only what the
holder of that key served in answer to that very claim, even if the
endpoint, its certificate or a proxy in front of it is compromised. It does
not make a compromised server honest; see the open questions.

### The client: never a wrong replay; on any doubt, run

`RemoteLedger` replays only when every check passes on the runner itself:

1. the receipt verifies and names exactly what was served, and this claim's
   nonce;
2. the result and manifest are for this step key, from a schema version this
   binary reads (as `Ledger::find_match` filters), and the result is for that
   manifest (`manifest_id`);
3. the manifest matches this runner's own tree (`quod_manifest::matches`,
   the same check the local ledger makes), so a server bug or lie about the
   lookup cannot replay a result for other inputs;
4. every blob the replay serves (stdout, stderr, command files, the outputs
   it writes) is fetched and hashes to its name.

Anything else (no server configured correctly, unreachable, a timeout, any
HTTP error, a bad answer, a failed check) makes the step execute. The ledger
then goes offline for the rest of the process: nothing is replayed or
recorded, and the banner says so once (`quod: the Quod server is not used
(...); the step runs, and nothing is replayed or recorded`). The step's own
exit code is what the job gets. A commit that fails after a successful
execution is the same: the step's result stands, unrecorded.

Configuration: `QUOD_SERVER` (base URL), `QUOD_SERVER_TOKEN` or GitHub's
`ACTIONS_ID_TOKEN_REQUEST_URL`/`_TOKEN` (the job needs `permissions:
id-token: write`) with `QUOD_SERVER_AUDIENCE` (default `quodci`),
`QUOD_SERVER_KEY`, `QUOD_SERVER_TIMEOUT` (seconds, default 30). The hooks
read these from the runner's own environment (the hook process's, which a
workflow cannot set), never the step's.

Known cost of the fallback: a replay the server recorded and the runner then
refused (a fetch failed) counts as a replay on the server that never
happened. Counters only; no result is affected.

### The lookup: the tree today, the index next

In P1 a claim sends the whole workspace tree (every entry, listings
included) and the server matches candidates with the same `find_match` a
local ledger runs. Simple and exactly the local semantics, but a few MB of
JSON per step on a large repo. exp/never-worse adds a per-key index to the
ledger (v5: content-addressed per-directory manifest nodes shared between
manifests; a lookup decodes only what differs). That lands under
`LedgerStore::find_match` unchanged. The protocol step after it, an additive
v1 change: a claim may send only the tree's root and directory listing
hashes; the server walks its per-key decision tree (sealed prefixes such as
a module cache collapse to one node whose listing hash decides them) and
answers either a decision or the next set of paths whose entries it needs.
Most claims then send kilobytes. The runner's own `matches` check stays, so
the index is an optimization the server cannot get wrong into a replay.

GOCACHEPROG is a later integration: Go's build and test cache served from
the team's store through `GOCACHEPROG`, keyed by Go's own action ids, so a
miss still starts warm on a fresh runner. Not designed here beyond noting
that it would be its own endpoint family (Go's action id to output id), with
the same scope rules: only writer jobs put, everyone in the repository gets.

### The dashboard's read API

quod-server implements the endpoints of cloud/docs/api.md its data
supports: `health`, `overview`, `repos`, `repos/:repo`, `runs`,
`repos/:repo/runs/:number`, `steps`, `repos/:repo/steps/:key`,
`search-index`, `activity`, and `events` (Server-Sent Events from an outbox
table read by id, so `Last-Event-ID` resumes exactly; `: ping` every 15 s;
`event: bye` after `QUOD_SSE_HOLD_SECS`, default 240). It sends
`step.finished` and `trust.changed`; `job.*` come from the GitHub App, which
the dashboard reads itself, and `run.*`/`stats.updated` are P5. Step keys in
paths are 64 hex digits (the `b3:` form has a colon, which the path id rule
does not allow). Trust state shown is the trusted scope's. Unbuilt
endpoints answer 501 with the reason: `teams/resolve` (WorkOS), `rerun`
(needs `actions: write`, which the App does not have), `PUT .../proofs`,
`runners`, `connect`.

For the dashboard contract's "Server changes needed" (cloud/docs/api.md)
this build serves: WorkOS viewers and teams by WorkOS org id (1, 2);
GitHub's job id (`QUOD_JOB_ID`, from `${{ job.check_run_id }}`), job name
(`QUOD_JOB_NAME`) and 1-based step number (`QUOD_STEP_INDEX`) on step rows,
`RunJob.id`/`name`, `RunStep.index` in `RunDetail` and `step.finished` (4);
runs grouped by GitHub run id and attempt, with `runId`, `runAttempt` and a
`githubUrl` naming both (5; the path still names the run number and picks
the newest run with it); `stepKey` null for a step without one (6); the
configured `defaultBranch` (per repository in the teams config, else the
team's first writer ref) (8); writer trust (9: `trustedWrites` per repo
with ref, `protected` true/false/null and `lastAt`, `writerTrustWarning`,
and `requireProtectedRef` on `/health` and `/repos/:repo`; the commit
answer carries the same warning as a `notice` the client prints once in
the job log); one `days` point per UTC day of the range (11).

Approximations, recorded: `StepWhy` `changed` is reported as `first-run`
(the claim's closest mismatch is not yet stored with the step row); the
divergence excerpt is the audit's detail text, not a line diff; repository
language is not known (empty).

## P2: who may write what

### Runners: GitHub OIDC

A runner on a GitHub-hosted machine authenticates with the job's OIDC token
for audience `quodci`. The server verifies it against the issuer's JWKS
(RS256 only; issuer, audience, `exp`, `iat`; 60 s leeway; an unknown `kid`
refetches the set at most once a minute) and maps it to a team by
`repository_owner_id` (and optionally `repository_id`), never by owner
login: a login can be renamed and re-registered by someone else, an id
cannot. Until P5 reads the GitHub App's installation links from the shared
database, the link is the team config's `github_owner_ids`.

The scope comes from the claims, with a stricter rule than the local hooks',
because here the client runs inside the job. With `quodci/setup` on a
GitHub-hosted runner, the job's own code can call the server with the job's
token and send anything the client could, including a fabricated result. So:

| Job | Reads (replays from) | Writes |
|---|---|---|
| Writer: `push`, `workflow_dispatch` or `schedule` on a writer ref (default `refs/heads/main`), optionally `ref_protected` and from an allowed `job_workflow_ref` | `trusted` | `trusted` |
| The repository's other jobs: `push` or dispatch on another ref, `pull_request`, `merge_group` | `trusted`, then its ref's own scope | `ref-<ref>-<hash>` |
| Events that run on input someone else controls: `pull_request_target`, `workflow_run`, `issue_comment`, anything else | `unverified` | `unverified` |

Fork pull requests get no OIDC token from GitHub. OIDC tokens carry no
default-branch claim, hence the configured writer refs. Only writer jobs
commit results that trusted jobs replay; a pull request replays what main
proved but never proves anything for main (test S3). When a non-writer's
claim does not replay and its own scope has no matching manifest, it is
given trusted's hit and reason (proving, an audit), so its execution is
compared with main's result; the verdict and any quarantine are written to
its own scope only (test S3b). Trusted's rows a non-writer touches are its
replay counters: a replay it is served advances trusted's `eligible` and
`replays`, which can pull trusted's next audit earlier, never skip one (an
audit decision does not advance the counter, so the next writer claim meets
the same slot).

### Runners: static tokens (self-hosted)

Stored as BLAKE3 hashes (`quod-server hash-token`). A runner token has a
fixed repository and scope (`trusted` by default). A delegated runner token
(`delegate: true`) is for infrastructure whose jobs cannot reach the token,
the runner hooks on a self-hosted runner: each request states the
repository and scope the hooks decided from the event payload (`trusted`,
`fork-<owner_repo>`, `unverified`), which is exactly the local rule, because
there Quod's runner, not the job, is the trusted computing base. Its
`writer` flag is informational, as locally: a delegated runner's jobs write
the scope it names. The hooks read the token from the hook process's
environment; a step never sees it: the sandbox starts every step with
`--clearenv` and an environment the hooks build from the job's protocol
data (quod-sandbox `env_clear`/`--clearenv`). A runner-surface action that
runs as root on the host could read the runner's environment, which is
already outside what a delegated token can protect (open question 11).

### Viewers

On a dashboard path a bearer that is a configured static viewer token is
one (`admin` or `member`); any other JWT is a WorkOS AuthKit access token,
verified as cloud/docs/api.md asks: RS256 only, keys from
`https://api.workos.com/sso/jwks/<client id>` (cached, refetched for an
unknown `kid` at most once a minute), `iss` exactly the configured issuer
(default `https://api.workos.com/user_management/<client id>`), `exp` and
`iat` with 60 s leeway, `sub` present, no audience. The path's team is the
team's slug or its `workos_org_id`. If the token's `org_id` is that org, the
viewer is a member with the token's `role`; otherwise, with
`WORKOS_API_KEY` set, the server asks WorkOS's membership API (active
memberships only, cached 60 s per user and org); without it, or with no
membership, 404. A bad or expired token is 401; a runner's token is 403.
`PUT .../proofs` and `PUT .../connect` need `admin` (403 otherwise), then
answer 501 as before. The client id is public; the API key is a secret and
optional.

### Team isolation

Every principal belongs to one team, and every handler reads only that
team's directory (self-hosted) or rows (P4: `team` in every key and every
query, and every object under the team's prefix; row-level security is
not used yet, see P4). A
dashboard path naming another team is 404, never 403. Test S4: team B's
runner with the same repository name, key and tree misses; B cannot GET or
learn the existence of A's blobs; A's results are untouched by B's; each
viewer sees only its own team.

## Self-hosted layout

```text
$QUOD_DATA_DIR/teams/<team>/cas/                          blobs
$QUOD_DATA_DIR/teams/<team>/uploads/                      resumable uploads
$QUOD_DATA_DIR/teams/<team>/scopes/<repo>-<hash>/<scope>/ledger.sqlite
$QUOD_DATA_DIR/teams/<team>/team.sqlite                   step rows, event outbox, trusted writes
$QUOD_DATA_DIR/signing.ed25519                            the receipt signing key (0600, generated at first boot)
$QUOD_DATA_DIR/quod-server.lock                           the instance lock
$QUOD_DATA_DIR/teams.json                                 the teams document, if not in the environment
```

Scope ledgers are ordinary quod ledgers (v8 today): a schema migration in
quod-ledger migrates them the same way it migrates a laptop's, at start
(below), and the server adds no tables to them. v7's `audits_pending` and
v8's `network` exist in them and stay empty (see "On main's semantics
(v3)"). What only the server keeps lives in
`team.sqlite`, versioned on its own (`PRAGMA user_version`, now 2: GitHub's
job id and name on step rows, and `trusted_writes`); a newer version than
the binary knows is refused, as quod-ledger refuses a newer ledger.

Server configuration. Secrets are marked; everything else may be shown.

| Variable | Secret | Meaning |
|---|---|---|
| `PORT`, `QUOD_BIND` | no | listen address (default `0.0.0.0:8080`; Railway sets `PORT`) |
| `QUOD_DATA_DIR` | no | the persistent directory above; absolute; required |
| `QUOD_DATA_DIR_PERSISTENT` | no | `1`: the operator attests the data dir survives restarts and redeploys. Not needed on Railway when the service's volume is mounted at (or above) `QUOD_DATA_DIR` (`RAILWAY_VOLUME_MOUNT_PATH`, which Railway sets) |
| `QUOD_SIGNING_KEY_FILE` | no (the path) | the key file, default `$QUOD_DATA_DIR/signing.ed25519` |
| `QUOD_SIGNING_KEY` | **yes** | a seed in the environment instead of the file (self-hosting only; leave unset on Railway) |
| `QUOD_TEAMS_JSON` | no, unless it holds static token hashes you want private | the teams document inline (`QUOD_SERVER_CONFIG_JSON` is the old name) |
| `QUOD_SERVER_CONFIG` | no | a path to the teams document; else `$QUOD_DATA_DIR/teams.json` if present |
| `WORKOS_CLIENT_ID` | no | turns on WorkOS viewer auth when the document has no `viewers.workos` |
| `WORKOS_ISSUER` | no | the access tokens' issuer, if a custom auth domain is set |
| `WORKOS_API_KEY` | **yes**, optional | the membership fallback for a viewer whose token names another organization |
| `QUOD_LOCK_WAIT_SECS` | no | how long a second instance waits for the lock (default 30) |
| `QUOD_MAX_BLOB_BYTES`, `QUOD_MAX_JSON_BYTES`, `QUOD_SSE_HOLD_SECS` | no | limits (defaults 1 GiB, 32 MiB, 240 s) |
| `QUOD_DEV=1` | no | development only: a relative data dir, no persistence check |
| `QUOD_ALLOW_ROOT=1` | no | serve as root (the image never needs it) |
| `DATABASE_URL` | **yes** | with the `S3_*` below: Postgres and the bucket instead of SQLite and files (P4, "Backend selection"); one without the other is refused |
| `S3_ENDPOINT`, `S3_BUCKET`, `S3_REGION`, `S3_FORCE_PATH_STYLE`, `S3_PREFIX` | no | the bucket (P4) |
| `S3_ACCESS_KEY_ID`, `S3_SECRET_ACCESS_KEY` | **yes** | the bucket's credentials (P4) |
| `QUOD_BACKEND` | no | `sqlite` keeps SQLite whatever else is set; `postgres` insists; unset: chosen by the variables above |
| `QUOD_PG_POOL_SIZE` | no | Postgres connections (default 10) |
| `QUOD_IMPORT_SQLITE` | no | `1`: at boot, import the volume's SQLite data into Postgres and the bucket (once per team) |
| `QUOD_SKIP_MIGRATION` | no | `1`: no start-up pass over the SQLite files (each still migrates when first opened) |

```json
{
  "teams": [{"slug": "acme", "workos_org_id": "org_01H...",
             "github_owner_ids": [123456], "github_repository_ids": [],
             "writer_refs": ["refs/heads/main"], "require_protected_ref": false,
             "writer_workflows": [],
             "repos": [{"full_name": "acme/app", "default_branch": "main",
                        "require_protected_ref": true}],
             "policy": {"proofs": 1, "shadow": 2, "audit": 0.05}}],
  "tokens": [{"blake3": "<64 hex>", "team": "acme", "kind": "runner", "delegate": true},
             {"blake3": "<64 hex>", "team": "acme", "kind": "viewer", "role": "admin"}],
  "oidc": {"audiences": ["quodci"]},
  "viewers": {"workos": {"client_id": "client_01H..."}}
}
```

A team's policy is a floor: the server applies the client's mode and the
stricter of the two for proofs, audit rate and replaying failures.

## On main's semantics (v3)

`exp/cloud-ledger-v3` is `exp/cloud-ledger-v2` merged onto main 5a4c18b,
which had moved on with ledger v7 (deferred audits) and v8 (the network
memory), the withheld-name scan redesign, pyc rewriting, image-env rules,
zero-message opens for go test/vet and the head-replay receipts. Everything
a runner does locally that is not a trust decision (the scan, the
observer, the receipts, the `mismatches` run-event field) needs nothing
from the server; what follows is what does.

### Deferred audits and remote runners

On main, `quod run` and `quod step` defer decay-phase audits by default:
the replay is served, a pending audit row is written in the same
transaction, and a detached worker on the same machine (`quod audit`)
executes and settles it later. That model assumes the queue and the worker
share a machine and a state dir. A remote runner breaks both: the ledger
is the server's, and a GitHub-hosted runner is gone at the end of its job,
usually before its worker would run.

Options considered:

- (a) The runner's worker reports back (`POST /v1/audit-result`): the
  server would hold the pending row, the runner the tree and the request.
  An ephemeral runner never runs its worker, so every such row would turn
  owed after `PENDING_TTL` and come back inline a day later on some other
  runner, after up to `MAX_DEFERRED_PER_KEY` audit intervals served
  without a verdict. It also needs the server to trust a verdict from the
  job whose result it audits (the open question 2 problem, once more) and
  the input tree shipped or stored server-side.
- (b) A server-side or pool-side executor (trust v2's option (c)): needs a
  sandboxed executor with the job's environment and secrets. Not now.
- (c) **Chosen: remote runners audit inline.** `quod_wire::effective_policy`
  sets `defer: false` whatever the client and the team say; the server's
  ClaimReplay therefore never decides `Defer`, and if it ever did
  (`Replayed::Deferred`), the transaction is rolled back with a 500
  rather than serve a replay owing an audit. The client turns deferral off
  too (`step_ledger_from_env`) and passes no deferral callback, and it
  treats a claim answer naming a deferred audit as a bad answer (the step
  runs). A commit or divergence naming `Reason::DeferredAudit` is refused
  (400): the server has no pending row for it to settle. So a remote runner
  can never leave a pending audit that cannot be settled; it pays the
  decay audits in job time, as every runner did before deferral (the hooks
  still do, locally too).

`StepLedger::claim` takes the caller's deferral callback (`Option<Defer>`):
`quod run` passes one for its local ledger (it builds the tree document and
the audit request, and `LocalLedger` enqueues the pending row in the
claim's transaction, as main's run did inline); the hooks and every remote
caller pass none, and `LocalLedger` refuses a deferring policy without one.
`Claimed::Replay` carries `deferred` (`skip_serializing_if` none, so the
wire is unchanged when nothing is deferred). The pure transitions carry
main's deferred-audit outcomes: the shell settles a `DeferredAudit`
(`finish_audit`), lets an inline audit repay an owed one (`repay_audit`)
and drops a key's other pending audits at a quarantine
(`supersede_pending`); those are `LedgerStore` methods, so a server's
ledger answers them too, from empty rows. Revisit with (a) or (b) when a
runner pool with a shared state (self-hosted) wants deferral.

### The network memory

The `network` table (v8) remembers per step key, per inputs, that a hooks
step's offline try reached for the network, so the next runs skip the
offline try. It stays on the runner: the hooks read and write it in the
local scope ledger even when `QUOD_SERVER` is set. It is a speed hint, not
a trust decision (a step that skips the offline try runs with the network
and is not recorded, which is the safe direction), so it needs no
server-side authority, and sharing it would let one job's network failure
change how another job's step runs. On an ephemeral runner it is forgotten
each job, as the deferred-audit queue would be. The server's scope ledgers
have the table (they are ordinary v8 ledgers) and never write it.

### Versions end to end

- quod.v1 is unchanged: no path, header or required field changed. New
  fields are optional with defaults: `Policy.defer` (`#[serde(default)]`,
  off), `Verdict.repaid` and `Verdict.superseded` (default false and 0),
  `Claimed::Replay.deferred` (omitted when none). A v2-built client and
  this server, or a main-built client and the deployed v2 server, parse
  each other's bodies (readers ignore unknown fields). The one new enum
  value, `Reason::DeferredAudit`, is never sent by a client to a server.
- Manifests and results are still schema v3; the client keeps refusing a
  hit from a newer schema, and the server a commit from one.
- Scope ledgers: v8, migrated at start (below). The deployed v2 build
  (ledger v6) refuses a v8 ledger (`TooNew`), so once this build has
  started on the volume, rolling Railway back to the v2 branch makes every
  scope ledger unreadable to it (claims 500, runners execute); roll back by
  restoring the volume, or forward. `team.sqlite` stays at 2.

### Start-up migration

After the instance lock and the signing key, before listening,
`Store::migrate` reads every SQLite file's version (`Store::plan`), then
opens every scope ledger of every team directory on the volume (each
migration in its own `BEGIN IMMEDIATE` transaction, so a failure leaves
that file as it was) and every `team.sqlite`, and logs `ledgers ready`
with how many were upgraded. Only a file newer than the binary stops the
boot (`data dir migration: team <t>, <repo> scope <s> (<path>): schema
version ...`), before anything is migrated, instead of serving 500s for
that scope. Any other failure (a corrupt or unreadable file, a failed
migration) is logged with the team, scope and path and counted
(`failed`); the boot goes on, the other scopes serve, and that scope
answers 500 until it is repaired (from the v3 review, MAJOR 2).
`QUOD_SKIP_MIGRATION=1` skips the pass (each file still migrates when a
request first opens it); `quod-server migrate --dry-run` prints each
file's version and what the pass would do without writing, and
`quod-server migrate` runs the pass alone under the instance lock. With
Postgres the files are not touched; Postgres migrates in one locked
transaction (P4) and `migrate --dry-run` reports its version instead.
Nothing is deleted. The signing key file is only ever created
when absent (never replaced or regenerated: a key that cannot be read or
parsed stops the boot). Test S14 builds the deployed build's data dir (v6
ledgers equal to that build's golden schema, a proof recorded, the key
generated on first boot), boots this build on it, checks every ledger is
at v8 with main's golden schema, the key file byte for byte, and that a
fresh runner pinned to the old key replays the pre-upgrade proof; a third
boot migrates nothing; a ledger at version 99 refuses the boot and leaves
the key alone, and the error names the team, scope and file; a corrupt
ledger does not stop the boot and is left as it was. Opens of one SQLite
file are serialized in the process, one lock per path (`store::path_lock`,
scope ledgers and `team.sqlite` alike): a new file's first open switches
it to WAL, which SQLite answers with SQLITE_BUSY, not the busy timeout,
when it races another first open (S2 under load showed it as a 500);
opens of different files never wait on each other.

Downgrading a volume by hand (v8 back to v6, for the v2 build): v7 and v8
only added the tables `audits_pending` and `network`, which a server never
writes. When both are empty in every scope ledger, `DROP TABLE
audits_pending; DROP TABLE network; PRAGMA user_version = 6;` on each
`ledger.sqlite` (with the server stopped) gives a ledger the v2 build
opens. Not safe if either table has rows; restoring the volume's backup
is the other way back.

## Compatibility with the branches in flight

- exp/never-worse (ledger v5, the lookup index; a Go cache inside Quod): the
  index lives behind `Ledger::find_match`, which `LedgerStore::find_match`
  delegates to, so it slots in with no protocol change. Its CANDIDATES bump
  applies on the server too. Its v5 migration and trust-v2's v5 collide with
  each other, not with this branch (no schema change here).
- exp/trust-v2 (proofs 1, shadow audits, suspect marking, cross-branch
  pooling): its rule changes land in the pure transitions
  (`decide`, `execution_transition`, `divergence_transition`), which local
  and server share, so they cannot drift. Its new trust columns
  (`shadow_from`, `volatile`) need `#[serde(default)]` on `TrustRow`, and
  `mark_suspect_replays`/`suspect_runs` join `LedgerStore`. Its
  cross-branch pooling is what the read-scope list already expresses: a
  claim tries scopes in order; pooling evidence across scopes is a server
  query over them inside one claim.

## P3: encryption (design)

Goals: hashes never leak across teams, blobs are encrypted at rest with
per-team keys, and the server can still run the trust layer (it must read
stdout and stderr to normalize and compare them, so this is server-side
encryption, not end-to-end; see the open questions).

- Per-team keys: a 32-byte team root key, wrapped by a KMS key and stored
  with the team row; never in the object store. Derived subkeys with
  `blake3::derive_key`: `"quod v1 address"` and `"quod v1 blob"`.
- Content addressing at rest: an object's name is
  `blake3::keyed_hash(address_key, plain_hash)`; the object store and any
  index outside the team's rows never see a plain hash. Two teams with the
  same file have unrelated object names, so there is no cross-team dedupe
  and no "does anyone have this file" oracle. Within quod.v1 the plain BLAKE3
  stays the name (the runner computes it, the manifest is made of it, and the
  wire is already team-scoped and TLS); the server maps it.
- Convergent AEAD for blobs: per blob, `k = blake3::keyed_hash(blob_key,
  plain_hash)`, sealed with XChaCha20-Poly1305 under `k` with a nonce derived
  from `k` (deterministic, safe because `k` is unique per content), and the
  plain hash as associated data. Convergent within a team (one object per
  content, so dedupe and idempotent uploads still work), unrelated across
  teams. Chunked (1 MiB records, the record index in the AD) so large blobs
  stream and range reads stay possible.
- Manifests and results in Postgres: encrypted columns for stdout-bearing
  JSON is not needed (they hold hashes and paths); paths can reveal repo
  layout and are covered by Postgres at-rest encryption and the team boundary.
- Rotation: a new root key re-wraps; objects are re-sealed lazily on read, or
  by a sweep.

## P4: Postgres and an S3-compatible bucket (built on exp/cloud-p4; not deployed)

Quod Cloud's storage: the ledger in Postgres, blobs in an S3-compatible
bucket. The SQLite and filesystem backends stay for self-hosting and
development; the environment picks one (below). Handlers see one
interface for both (`store::ScopeLedger`, `store::TeamBlobs`,
`store::Activity`), and the trust code is the same quod-trust over
`LedgerStore` and `BlobStore`.

### The ledger in Postgres (`pg.rs`)

- Schema: quod-ledger's v8 tables (runs, manifests, results, blobs,
  step_keys, trust, proofs, audits, manifest_nodes, audits_pending,
  network) and team.sqlite v2's (steps, events, trusted_writes), one
  database for every team. A scope ledger is a row of `scopes`, keyed
  (team, repository key, scope) exactly as the self-hosted directory is
  (`teams/<team>/scopes/<repo key>/<scope>`: `github-<id>` or the name
  plus a hash), so the importer maps one onto the other. Every ledger
  table carries `team` and `scope_id` in its key and every query filters
  on both; team tables carry `team`. Text columns are `COLLATE "C"`, so
  ordering and comparisons (RFC 3339 timestamps as strings, names) are
  bytewise as in SQLite; SQLite's rowid tie-breaks are a `seq` column
  (a re-recorded manifest gets a new one, as REPLACE gives a new rowid).
  `manifest_nodes` are content-addressed and shared within a team.
  `PG_LEDGER_LEVEL` names the quod-ledger version the tables mirror, and
  a compile-time check fails the build when quod-ledger gains a migration
  that `pg.rs` has not mirrored.
- `atomically`: one transaction whose first statement is
  `pg_advisory_xact_lock` on a hash of (team, scope id), held until COMMIT
  or ROLLBACK. Every write to a scope's rows takes the same lock (a write
  outside `atomically` is its own one-statement `atomically`), so writers
  of one scope are serialized exactly as SQLite's BEGIN IMMEDIATE
  serializes writers of one file, across connections and instances. A
  lock per scope is a superset of the design's lock per (team, scope,
  key): `atomically` is not handed the key, and per-scope serialization
  is what the SQLite server already does. Isolation is READ COMMITTED,
  which under the lock reads every committed write of the scope. A
  failure inside rolls everything back; a panic inside rolls back when
  the ledger is dropped. Lock waits are bounded (`lock_timeout` 60 s, as
  SQLite's busy timeout); statements by `statement_timeout` 120 s.
- `find_match` is `Ledger::find_match`'s algorithm on the same rows: the
  newest candidate decoded whole; older ones through the index, whose
  node building and evaluation moved into quod-ledger's `index`
  (`manifest_nodes`, `NodeEval` over a loader) so both stores run the same
  code; every index hit checked again with `matches`; a newer schema's
  manifest or result never matches. P3 checks it answers as SQLite does
  on the same history.
- The event outbox: ids come from one sequence, and inserts of one team
  take a per-team advisory lock, so a team's events commit in id order
  and `Last-Event-ID` resumption still never skips one.
- Connections: r2d2 over the sync `postgres` client, inside
  `spawn_blocking` like every other store call; `QUOD_PG_POOL_SIZE`
  (default 10). A ledger holds a connection only inside `atomically`, so
  one request never sits on several. No TLS: the private-network URL is
  required (Railway's private network is encrypted; `sslmode=require` is
  refused with a message, not silently downgraded).
- Migrations (`quod_schema`): one transaction under an advisory lock, each
  version applied once and recorded; instances starting together wait
  and find the work done (P1); a database newer than the build is refused
  at start; `quod-server migrate --dry-run` reports, `migrate` applies.
- Not used: Postgres row-level security as a second guard. The service
  connects as Railway's `postgres` superuser, which bypasses RLS, so
  policies would only look like a guard; doing it properly needs a
  non-owner role for the server. Isolation is `team` in every key and
  query (S4 and P2 check it).

### Blobs in the bucket (`s3.rs`)

- Objects at `<S3_PREFIX>teams/<team>/blobs/<blake3 hex>`: a team's own
  prefix, no request names another team's.
- Content addressing kept: bytes in memory are hashed before the PUT; a
  file is hashed to name the object and hashed again while its parts go
  up, and the multipart upload is completed only when both agree (else
  aborted, `CasError::Changed`). Each request is signed over its body's
  SHA-256, so the store rejects bytes changed in flight. Reads check the
  bytes against the name (whole reads fail, streams fail at their end).
- Single PUT up to 16 MiB, multipart above (16 MiB parts).
- Resumable uploads (`PATCH`) still stage on the instance's data dir and
  go to the bucket when complete; `GET` streams from the bucket through
  the server. Presigned GETs (bytes skipping the server) are later.
- The client is quod-server's own SigV4 over ureq (HEAD, GET, PUT,
  multipart), checked against AWS's published key-derivation example and
  against MinIO in CI; virtual-hosted URLs by default, path-style with
  `S3_FORCE_PATH_STYLE=1`.

### Backend selection

| Environment | Backend |
|---|---|
| neither `DATABASE_URL` nor any `S3_*` | SQLite and files under `QUOD_DATA_DIR` (as before) |
| `DATABASE_URL` and `S3_ENDPOINT`, `S3_BUCKET`, `S3_ACCESS_KEY_ID`, `S3_SECRET_ACCESS_KEY` | Postgres and the bucket |
| one without the others | refused at start, naming what is missing |
| `QUOD_BACKEND=sqlite` | SQLite whatever else is set (a rollback without unsetting) |
| `QUOD_BACKEND=postgres` | Postgres and the bucket, or refused |

`S3_REGION` defaults to `us-east-1`; the AWS SDK names
(`AWS_ENDPOINT_URL_S3`, `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`,
`AWS_REGION`) are read when the `S3_*` ones are absent, but the bucket is
always `S3_BUCKET`. At start, the cloud backend connects, migrates, and
puts and reads back a probe blob (`teams/quod-probe/...`); any failure
stops the boot rather than serve 500s.

The volume stays: it holds the signing key and upload staging. So does
the instance lock, now for those two (the key file's first generation,
staging files two processes must not share), not for the ledger, whose
writers serialize in Postgres. Replicas stay at 1. Going to more needs
the key in `QUOD_SIGNING_KEY` (a sealed variable, the same key) and
staging that is not shared (a Railway volume attaches to one replica
anyway); then the lock can go.

### Moving the live data (`import.rs`)

`quod-server migrate-sqlite-to-postgres` (or `QUOD_IMPORT_SQLITE=1`, which
runs the same import at boot, before listening): per team, every CAS blob
to the bucket (named by its hash), then every scope ledger and
team.sqlite read-only and copied row by row in their own order in one
Postgres transaction with an `imports` row. A team already imported is
skipped; a team Postgres already has scopes for (a server ran on Postgres
first) is refused, never merged. Ledgers must be at this build's schema
(start it once on SQLite first). Audit and event ids are renumbered
(a dashboard holding an old `Last-Event-ID` resumes at the stream's end).
Nothing on the volume is changed or deleted. P5 checks a proof recorded on
SQLite replays from Postgres after the import.

### Railway variables (quod-server, production)

Changes to make on the `quod-server` service for the cut-over (nothing
here has been set by this branch):

| Variable | Value | Secret |
|---|---|---|
| `DATABASE_URL` | `${{Postgres.DATABASE_URL}}` (the private-network URL; not `DATABASE_PUBLIC_URL`) | **yes** (holds the password; a reference, so Railway resolves it) |
| `S3_ENDPOINT` | `${{quod-results.ENDPOINT}}` (`https://t3.storageapi.dev` today) | no |
| `S3_BUCKET` | `${{quod-results.BUCKET}}` (the S3 name with its hash suffix, not `RAILWAY_BUCKET_NAME`) | no |
| `S3_REGION` | `${{quod-results.REGION}}` (`auto`) | no |
| `S3_ACCESS_KEY_ID` | `${{quod-results.ACCESS_KEY_ID}}` | **yes** |
| `S3_SECRET_ACCESS_KEY` | `${{quod-results.SECRET_ACCESS_KEY}}` | **yes** |
| `S3_FORCE_PATH_STYLE` | `1` only if the bucket's Credentials tab says path-style; else unset (virtual-hosted, Railway's default for new buckets) | no |
| `QUOD_IMPORT_SQLITE` | `1` for the cut-over deploy (imports the volume's data once, at boot); remove afterwards (harmless if left: imported teams are skipped) | no |
| `QUOD_PG_POOL_SIZE` | unset (10) | no |

Unchanged: `QUOD_DATA_DIR=/data` and the volume at `/data` (the signing
key `signing.ed25519` stays there), `QUOD_TEAMS_JSON`, `WORKOS_*`,
`RUST_LOG`, `numReplicas` 1. Not set: `QUOD_SIGNING_KEY`, `S3_PREFIX`,
`QUOD_BACKEND`. The bucket is reachable only over the public network
(Railway buckets have no private networking; egress to it is free).

### Cut-over runbook

1. Deploy this build with variables unchanged (it runs on SQLite as the
   live build does). Check `/healthz` and that the key's fingerprint in
   the boot log (`signing key ready`) matches `QUOD_SERVER_KEY`.
2. Take a backup of the `quod-server` volume (Railway: the volume's
   Backups tab). Note the Postgres service's backup state too.
3. Dry run, from the service's shell (`railway ssh` into quod-server):
   `DATABASE_URL=... S3_...=... quod-server migrate --dry-run` with the
   values from the Postgres and bucket services prints the Postgres
   schema version (0 on a fresh database: "would migrate"); without them
   it prints each SQLite file's version.
4. Set the variables in the table above, including `QUOD_IMPORT_SQLITE=1`,
   and deploy. The new instance waits for the old one's lock, connects
   to Postgres (migrating it), probes the bucket, imports the volume's
   teams, then listens. Boot log: `postgres and object store ready`,
   `SQLite data imported` with per-table counts, `ledgers ready` with
   `backend: postgres+s3`. Any failure stops the boot (Railway keeps
   restarting; the previous deployment is gone by then, so roll back by
   step 7).
5. Check: `/healthz` ok with the same key; the dashboard shows the same
   runs; a push to `main` of `pango07/quod` replays a step proved before
   the cut-over (the job log has no "not used" line).
6. Remove `QUOD_IMPORT_SQLITE` (a later boot would skip imported teams
   anyway). Keep the volume's SQLite files: they are the rollback.
7. Rollback: set `QUOD_BACKEND=sqlite` (or remove `DATABASE_URL` and
   `S3_*`) and redeploy: the server serves the volume's SQLite files as
   they were at the cut-over. Results recorded on Postgres after the
   cut-over are not on the volume (runners re-execute those steps; nothing
   wrong replays). Rolling the build back to v3 is the same.

### Residuals

- Upload staging is per instance (on its volume): a resumable upload
  continues only on the instance that has its first chunks.
- No retention: blobs and `ref-...` scopes accumulate (open question 12).
- P3's keyed object names and encryption are not applied yet: object
  names are plain BLAKE3 hashes under the team's prefix.

## Go-live on Railway

One instance of quod-server, SQLite and files on a Railway volume, at
`api.quodci.com`: deployed (v2, then v3). These are its settings; P4's
cut-over to Postgres and the bucket is "Cut-over runbook" above.

Before any deploy that changes a schema (a new quod-ledger version, a
new team.sqlite version, the first P4 boot), take a backup of the
`quod-server` volume (Railway: the volume's Backups tab) and check the
plan: `quod-server migrate --dry-run` in the running service's shell
prints each file's version and what the new build would do. A build
newer than the volume migrates it at start; rolling back to an older
build afterwards needs the backup, or the manual downgrade in "Start-up
migration".

### The service

| Setting | Value |
|---|---|
| Project / environment | `quod-cloud` / `production` |
| Service | `quod-server`, source: GitHub repo `pango07/quod`, branch to deploy, root directory `/` |
| Config as code | `railway.json` at the repo root (Railway reads it): builder `DOCKERFILE`, `dockerfilePath` `Dockerfile.server`, `healthcheckPath` `/healthz`, restart `ON_FAILURE` (10 retries), `numReplicas` 1, watch paths limited to the Rust workspace |
| Start command | none (the image's `ENTRYPOINT quod-entrypoint`, `CMD serve`) |
| Volume | attach one to the service, mount path `/data` (size to taste; results and blobs grow with use) |
| Replicas | exactly 1. The instance lock refuses a second one; a redeploy's new instance waits up to `QUOD_LOCK_WAIT_SECS` for the old one to stop, then exits non-zero and Railway retries |
| Region | us-east, near GitHub's runners |
| Networking | public domain: custom domain `api.quodci.com` (CNAME to the Railway domain Railway shows); TLS ends at Railway's edge. Target port: `PORT` (Railway sets it) |

Variables:

| Variable | Value | Secret |
|---|---|---|
| `QUOD_DATA_DIR` | `/data` (the image's default) | no |
| `QUOD_TEAMS_JSON` | the document below | no |
| `WORKOS_CLIENT_ID` | the dashboard's WorkOS client id (if not in the document) | no |
| `WORKOS_API_KEY` | WorkOS API key, only for the membership fallback | **yes**, optional |
| `RUST_LOG` | `quod=info` | no |

Not set: `QUOD_SIGNING_KEY` (the key is generated on the volume),
`QUOD_DATA_DIR_PERSISTENT` (Railway's `RAILWAY_VOLUME_MOUNT_PATH` proves the
volume; set it only on a host that cannot), `DATABASE_URL` and `S3_*`
(they select Postgres and the bucket: set them only at the P4 cut-over).

The teams document for go-live: one team, the product owner's WorkOS
organization "quodci", linked to GitHub App installation 177628965 for
`pango07/quod` (owner id 104618549, repository id 1440517147, read from
GitHub's API). Fill in the WorkOS organization id and client id:

```json
{
  "teams": [{
    "slug": "quodci", "name": "quodci",
    "workos_org_id": "<the quodci organization's id, org_...>",
    "github_installation_id": 177628965,
    "github_owner_ids": [104618549],
    "github_repository_ids": [1440517147],
    "writer_refs": ["refs/heads/main"],
    "repos": [{"full_name": "pango07/quod", "default_branch": "main"}]
  }],
  "oidc": {"audiences": ["quodci"]},
  "viewers": {"workos": {"client_id": "<WORKOS_CLIENT_ID, client_...>"}}
}
```

`require_protected_ref` stays off (DECISIONS 2026-11-02); writes from an
unprotected or unknown ref are recorded and warned about.

### What happens at start

1. The backend: SQLite and files, or Postgres and the bucket ("Backend
   selection"); half of the cloud variables: refused.
2. `QUOD_DATA_DIR` created if needed and probed for writing (create, write,
   fsync, remove). Not writable: refused.
3. Persistence: on Railway (`RAILWAY_PROJECT_ID`), the data dir must be
   inside the service's volume (`RAILWAY_VOLUME_MOUNT_PATH`); with no volume
   it is refused unless `QUOD_DATA_DIR_PERSISTENT=1` overrides (logged).
   Elsewhere `QUOD_DATA_DIR_PERSISTENT=1` is required. A volume that is
   ephemeral is the one failure mode the server cannot detect itself, so it
   asks for the attestation rather than guess.
4. The instance lock (`flock`-style exclusive lock on
   `quod-server.lock`, released when the process ends however it ends).
5. The signing key: read from `QUOD_SIGNING_KEY_FILE` (refused if group or
   world can read it), or generated: 32 bytes from the OS's randomness,
   written to a temporary file created 0600 in the same directory, fsynced,
   hard-linked to its name (never replacing an existing key), the directory
   fsynced. The log carries only the public key's fingerprint and whether
   it was generated.
6. The teams document.
7. With Postgres and the bucket: connect, migrate Postgres (one locked
   transaction), probe the bucket, and with `QUOD_IMPORT_SQLITE=1` import
   the volume's SQLite data; any failure stops the boot.
8. The migration ("Start-up migration"): with SQLite, every scope ledger
   and `team.sqlite` brought to this build's schema; a file newer than the
   build stops the boot, a corrupt one is logged and its scope answers 500
   (`QUOD_SKIP_MIGRATION=1` skips the pass). `ledgers ready` names the
   backend and the counts.
9. Listen, and sweep upload staging files older than a day (then hourly).

Redeploys and the lock. A redeploy's new instance starts while the old one
may still hold the lock; it waits up to `QUOD_LOCK_WAIT_SECS` (30 s) and
otherwise exits non-zero for Railway to retry. On SIGTERM the old instance
stops accepting at once and drops open connections, event streams
included (the dashboard reconnects with `Last-Event-ID`; `QUOD_SSE_HOLD_SECS`
does not delay shutdown), and gives in-flight blocking work at most 5 s, so
it lets go of the lock well inside the new instance's wait.

Limits (from the go-live review): runner JSON endpoints authenticate from
the headers before the body is read, so an unauthenticated caller costs no
body buffering; JSON bodies are at most `QUOD_MAX_JSON_BYTES` (32 MiB) and
must finish within 120 s; headers must arrive within 20 s; blobs are at
most `QUOD_MAX_BLOB_BYTES` (1 GiB), and an upload that sends nothing for
60 s is dropped; a result whose stdout or stderr is over 64 MiB is refused
at commit, by file size, before it is read; uploads are refused (507)
while the volume has less than 2 GiB or 10% free; client strings kept for
the dashboard (job and step names, runner, actor, a divergence's why, an
incomplete step's what) are cut at 4 KiB; step-key JSON is stored only if
it hashes to its key (JSON with masked secrets does not, and is not kept on
the server); OIDC repositories are stored under GitHub's repository id
(`scopes/github-<id>`), so a rename keeps them and a new repository under an
old name starts empty.

The image starts as root only long enough to give the volume (mounted
root-owned by Railway) to the `quod` user (uid 10001), then drops to it with
`setpriv --no-new-privs`; the server itself refuses to serve as root.

### Pinning the server's key

Every replay a client takes carries the server's Ed25519 signature over
exactly what it served, and the client checks it before replaying. The key
the client checks against:

- **Pinned** (`QUOD_SERVER_KEY`, the 64-hex public key): the production
  setting. A compromised endpoint, certificate or proxy still cannot make a
  runner replay. Set it as an organization or repository Actions variable
  (not a secret; it is public) from `GET https://api.quodci.com/v1/keys`,
  and check its `fingerprint` against the server's boot log line
  `signing key ready` before saving it.
- **Not pinned**: the client takes the key from `GET /v1/info` over the same
  TLS connection that authenticates `api.quodci.com`, once per process, and
  prints a one-line notice in the job log with the fingerprint and the
  `QUOD_SERVER_KEY=` value to pin. On ephemeral GitHub-hosted runners a
  stored trust-on-first-use pin would be forgotten after every job, so it
  is not offered; pinning by config is.

Rotation: the key lives on the volume and survives redeploys. Losing the
volume generates a new key; pinned runners then refuse every replay (and
execute, recording nothing they cannot verify) until `QUOD_SERVER_KEY` is
updated. Nothing ever serves or logs the private key; there is no command
that prints it.

### Clients (runners)

In the workflow of `pango07/quod`:

```yaml
permissions:
  id-token: write   # GitHub OIDC, audience quodci
  contents: read
env:
  QUOD_SERVER: https://api.quodci.com
  QUOD_SERVER_KEY: ${{ vars.QUOD_SERVER_KEY }}
  QUOD_JOB_ID: ${{ job.check_run_id }}       # GitHub's job id, for the dashboard
```

`QUOD_JOB_NAME` (the job's display name) and `QUOD_STEP_INDEX` (GitHub's
1-based step number) are optional and set per step by `quodci/setup` when it
exists. The audience defaults to `quodci` (`QUOD_SERVER_AUDIENCE`).

### The dashboard

On Vercel (quod-cloud), Production:

| Variable | Value | Secret |
|---|---|---|
| `QUOD_API` | `http` | no |
| `QUOD_SERVER_URL` | `https://api.quodci.com` | no |
| `QUOD_SERVER_VIEWER_AUTH` | `workos`: the dashboard sends the viewer's WorkOS access token and the org id as the team | no |
| `QUOD_SERVER_TEAMS` | not needed with WorkOS viewer auth; unset it | (was **yes**) |

The dashboard's `WORKOS_CLIENT_ID` must be the one the server is configured
with, since the server verifies tokens against that client's JWKS. Static
viewer tokens remain for self-hosting: configure them in the teams document
as BLAKE3 hashes (`quod-server hash-token`) and hand the token to the
dashboard's `QUOD_SERVER_TEAMS`.

### Checks after deploy

- `curl https://api.quodci.com/healthz`: `status` `ok` and the key.
- `curl https://api.quodci.com/v1/keys`: the fingerprint matches the boot
  log; save `public_key` as the `QUOD_SERVER_KEY` variable.
- A push to `main` of `pango07/quod` with the env above: the job log shows no
  "not used" line; the dashboard shows the run under the quodci team.

## P5: dashboard and setup (design)

- The dashboard's `lib/api/http.ts` against these endpoints, with WorkOS
  access tokens verified by quod-server (JWKS) and membership read from
  WorkOS per request; `teams/resolve` maps organizations to teams.
- Team links from the GitHub App's installations (the dashboard's tables)
  instead of `github_owner_ids` in config; writer refs from each repository's
  default branch through the App.
- `quodci/setup`: a GitHub Action that installs `quod`, sets `QUOD_SERVER`,
  `QUOD_SERVER_KEY`, and wraps steps; needs `id-token: write`.
- Events: `run.*` and `stats.updated`; per-step proofs (`PUT .../proofs`).

## Tests

- quod-trust: unit tests for each pure transition (first execution, proof,
  divergence and quarantine, unprovable, a purged previous result, a
  divergence without a result) and wire round trips.
- quod-wire: receipts verify only for what they signed (another result,
  another key, a tampered statement, another signer, a forged key field);
  the stricter policy wins; unknown fields are ignored; config parsing; an
  unreachable server means execute.
- quod-server `tests/server.rs`, an in-process server and runners with their
  own state dirs (synthetic steps, so it runs on any OS):
  S1 a fresh runner replays what another proved; S1b a receipt signs its
  claim's nonce and verifies for no other; S2 concurrency (commits and
  claims); S3 forks and non-writers never poison trusted; S3b a pull
  request's execution on main's inputs is compared with main's result and a
  difference quarantines only its own scope; S4 team isolation;
  S5 untrusted key, dead server, refused token: the step runs; S6 zero false
  hits over 133 mutations of every recorded input (content and mode of each
  file, each file removed, each directory listing, each absent probe), A7's
  method through the server; S7 OIDC against a local JWKS (expired, wrong
  audience, unlinked owner, tampered, `alg: none`, HS256 under the RSA key's
  id); S8 the dashboard reads and the event stream; resumable large blobs;
  the protocol header; 409 on unuploaded blobs; S14 a v6 data dir migrates at
  start and keeps its key (a newer file stops the boot and is named, a
  corrupt one does not, `migrate --dry-run` reports both); S15 a remote
  runner audits inline and the server never holds a pending audit (and
  refuses a commit naming one).
- The same suite on Postgres and S3: with `QUOD_TEST_DATABASE_URL` and
  `QUOD_TEST_S3_*` set, every server in tests/server.rs runs on its own
  Postgres schema and bucket prefix (S14, which is about SQLite files,
  returns at once), so S1 to S15, the S2 races included, run against both
  backends. tests/cloud.rs adds P1 migrations (four instances at once, a
  second run a no-op, a newer database refused), P2 `atomically` (a
  failure rolls back every write; eight writers times ten
  read-modify-writes with a pause inside lose no update; another team's
  rows apart; nesting joins), P3 lookups equal to SQLite's on the same
  history, P4 blobs (per team, multipart, streamed, a misnamed object
  refused on read) and P5 the import (a SQLite proof replays from
  Postgres; a second import skips; the volume's files unchanged; a team
  Postgres already holds is refused). CI: the `server-cloud` job of
  gate.yml, with a postgres:17 service and MinIO.
- quod-cli `tests/server.rs` (Linux, `QUOD_INTEGRATION=1`): the real `quod
  run` with two state dirs; runner 2 replays runner 1's proof byte for byte
  while neither local ledger holds a manifest; a changed input executes; a
  dead server executes; Q4 a decay-phase audit, deferred by default
  locally, runs inline against the server, and no pending audit exists on
  either side.

## Open security questions

1. A compromised Quod Cloud is a compromised CI for every team that
   replays: it can serve any result it signs. Receipts protect against a
   compromised endpoint, not a compromised signer. Options: per-team signing
   keys held in a KMS the API process can only ask to sign; a transparency
   log of signed statements runners can audit; runner-side audits whose
   divergences are reported to the team, not only to the server.
2. A writer job can commit anything (a fabricated result), because the
   client runs inside the job. That is inherent in GitHub-hosted runners: the
   same job could push to main. The mitigation is the writer definition
   (protected refs, allowed workflows); should `require_protected_ref`
   default to true?
3. `ignore` (output prefixes left out of comparison) comes from the client.
   A writer can widen it to hide nondeterminism from audits. Should the
   server derive it from the step key's profile instead?
4. Blob reads are team-wide: any runner of a team can GET any blob of the
   team by hash, including a fork scope's runner (self-hosted delegated
   tokens) reading a trusted output it can name. Locally the store is shared
   the same way. Scope-restricting reads means indexing blobs by scope.
5. The tree in a claim reveals the team's file hashes to the server (as
   manifests do). Server-side encryption (P3) protects at rest, not from the
   operator. End-to-end encryption would keep stdout from the server and so
   move trust decisions back to runners; not proposed.
6. OIDC team mapping by `repository_owner_id` from config is coarse (every
   repo of the owner); P5's installation links narrow it.
7. JWKS fetches are unauthenticated beyond TLS to
   token.actions.githubusercontent.com; a pinned JWKS (`jwks_file`) is
   supported for air-gapped GHES. GHES issuers need their own `issuer`.
8. Static tokens never expire. Rotation is a config change; there is no
   revocation list or audit log of token use yet.
9. Dashboard viewers use static tokens until P5; quod-server must not be
   exposed to browsers (the dashboard re-serves the stream server-side).
10. A replay the runner refused after the server recorded it inflates the
    replay counter, which feeds the audit schedule. Harmless for safety (the
    runner executed), but a hostile runner could shift which hit is audited.
    Recording replays only on a second confirm call would close it at one more
    round trip.
11. A delegated runner token lives in the self-hosted runner's environment.
    Steps never see it (`--clearenv`), but whatever runs on the runner host
    outside the sandbox does; a leaked delegated token can write any scope of
    the team, `trusted` included. Per-repository delegated tokens, or the
    runner proving its job's identity with the job's OIDC token alongside,
    would bound it.
12. Nothing is cleaned up yet: abandoned `uploads/<hash>.part` files stay,
    and every branch or pull request leaves a `ref-...` scope ledger. A
    retention sweep (age, and scopes whose ref is gone) is still to do;
    with P4 it is rows and objects, not files.
