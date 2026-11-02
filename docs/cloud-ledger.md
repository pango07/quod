# The central ledger: quod.v1, quod-server and Quod Cloud

One ledger and one result store for every runner of a team, so a fresh
GitHub-hosted runner can replay a result recorded anywhere. This page is the
design and the build record for branch `exp/cloud-ledger` (stages P0 to P2).
DECISIONS.md has one line per choice; this page explains how they fit.

## Status

| Stage | What | State |
|---|---|---|
| P0 | Ledger and CAS behind traits; trust transition as a pure function | built |
| P1 | quod-wire (quod.v1), quod-server (SQLite + filesystem), the client, dashboard reads | built |
| P2 | Runner auth: GitHub OIDC, scopes, static tokens, team isolation | built (WorkOS for viewers is P5) |
| P3 | Encryption: keyed content addressing per team, convergent AEAD | designed below |
| P4 | Hosting: Postgres + S3-compatible store, Railway | designed below; nothing provisioned or deployed |
| P5 | Dashboard on the HTTP adapter, WorkOS viewers, `quodci/setup` | designed below |

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
| `GET /healthz` | | `ok` |

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
`{protocol, team, scope, step_key, manifest_id, run_id, result, issued_at}`
where `result` is the BLAKE3 of the result's canonical JSON. The runner
verifies the signature under the key it trusts (`QUOD_SERVER_KEY`, or the
key from `/v1/info` over the same TLS connection when not pinned) and that
the statement names exactly the key, manifest, run and result bytes it was
handed. The signing key comes from `QUOD_SIGNING_KEY` (or `_FILE`); the
server refuses to start without one unless `QUOD_DEV=1`, because a key
generated per instance would make every runner that pinned the last one
refuse its replays.

What a signature buys: a runner that pins the key replays only what the
holder of that key served, even if the endpoint, its certificate or a proxy
in front of it is compromised. It does not make a compromised server honest;
see the open questions.

### The client: never a wrong replay; on any doubt, run

`RemoteLedger` replays only when every check passes on the runner itself:

1. the receipt verifies and names exactly what was served;
2. the result and manifest are for this step key, and the result is for that
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

Approximations, recorded: `StepWhy` `changed` is reported as `first-run`
(the claim's closest mismatch is not yet stored with the step row); the
divergence excerpt is the audit's detail text, not a line diff; repository
language and default branch are not known (empty, `main`).

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
proved but never proves anything for main (test S3). A non-writer cannot
spend trusted's audit slots either: an audit decision does not advance the
counter, so the next writer claim meets the same slot.

### Runners: static tokens (self-hosted)

Stored as BLAKE3 hashes (`quod-server hash-token`). A runner token has a
fixed repository and scope (`trusted` by default). A delegated runner token
(`delegate: true`) is for infrastructure whose jobs cannot reach the token,
the runner hooks on a self-hosted runner: each request states the
repository and scope the hooks decided from the event payload (`trusted`,
`fork-<owner_repo>`, `unverified`), which is exactly the local rule, because
there Quod's runner, not the job, is the trusted computing base.

### Viewers

Static viewer tokens (`admin` or `member`) in this build. WorkOS access
tokens (JWKS-verified, membership read from WorkOS per request, as
cloud/docs/api.md requires) are P5.

### Team isolation

Every principal belongs to one team, and every handler reads only that
team's directory (self-hosted) or rows (P4: `team_id` in every primary key
and every query, plus Postgres row-level security as a second guard). A
dashboard path naming another team is 404, never 403. Test S4: team B's
runner with the same repository name, key and tree misses; B cannot GET or
learn the existence of A's blobs; A's results are untouched by B's; each
viewer sees only its own team.

## Self-hosted layout

```text
$QUOD_DATA_DIR/teams/<team>/cas/                          blobs
$QUOD_DATA_DIR/teams/<team>/uploads/                      resumable uploads
$QUOD_DATA_DIR/teams/<team>/scopes/<repo>-<hash>/<scope>/ledger.sqlite
$QUOD_DATA_DIR/teams/<team>/team.sqlite                   step rows, event outbox
```

Scope ledgers are ordinary quod ledgers: a schema migration in quod-ledger
migrates them the same way it migrates a laptop's.

Server configuration, all environment:

| Variable | Meaning |
|---|---|
| `PORT`, `QUOD_BIND` | listen address (default `0.0.0.0:8080`) |
| `QUOD_DATA_DIR` | the persistent directory above |
| `QUOD_SIGNING_KEY` / `QUOD_SIGNING_KEY_FILE` | Ed25519 seed, 64 hex digits (`quod-server keygen`) |
| `QUOD_SERVER_CONFIG_JSON` / `QUOD_SERVER_CONFIG` | teams, tokens, OIDC (below) |
| `QUOD_MAX_BLOB_BYTES`, `QUOD_MAX_JSON_BYTES`, `QUOD_SSE_HOLD_SECS` | limits |
| `QUOD_DEV=1` | development only: a data dir in the cwd and a per-process key |
| `DATABASE_URL`, `S3_ENDPOINT`, `S3_BUCKET` | refused in this build (P4), never silently ignored |

```json
{
  "teams": [{"slug": "acme", "github_owner_ids": [123456],
             "writer_refs": ["refs/heads/main"], "require_protected_ref": false,
             "writer_workflows": [], "policy": {"proofs": 2, "audit": 0.05}}],
  "tokens": [{"blake3": "<64 hex>", "team": "acme", "kind": "runner", "delegate": true},
             {"blake3": "<64 hex>", "team": "acme", "kind": "viewer", "role": "admin"}],
  "oidc": {"audiences": ["quodci"]}
}
```

A team's policy is a floor: the server applies the client's mode and the
stricter of the two for proofs, audit rate and replaying failures.

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

## P4: hosting (design; nothing provisioned or deployed)

Production storage is Postgres plus an S3-compatible object store. No local
disk is assumed: an instance may be replaced at any time. The SQLite and
filesystem backends stay for self-hosting and development.

- `PgLedger: LedgerStore`: the scope ledgers become tables keyed by
  `(team_id, repository, scope, ...)`; `atomically` is a transaction that
  takes `SELECT ... FOR UPDATE` on the key's trust row (or a
  `pg_advisory_xact_lock` on a hash of (team, scope, key) when it does not
  exist yet); row-level security on `team_id` as a second guard. The sync
  `postgres` crate inside `spawn_blocking`, matching the trait, or an async
  trait variant if the blocking pool becomes the bottleneck. Pooled
  connections (Neon's pooler, Railway's PgBouncer) work: nothing uses LISTEN.
- `S3Blobs: BlobStore`: objects at `teams/<team>/<keyed name>`; uploads as
  multipart (the resumable PATCH maps to parts); GET streamed; presigned GETs
  for large blobs later, so bytes skip the server.
- The event outbox is a Postgres table read by id (the dashboard's stream
  resumes exactly); the dashboard's own GitHub App tables stay where they are.
- Config: `DATABASE_URL` and `S3_ENDPOINT`, `S3_BUCKET`, `S3_REGION`,
  `S3_ACCESS_KEY_ID`, `S3_SECRET_ACCESS_KEY` (`S3_FORCE_PATH_STYLE`). The
  generic `S3_*` names are the interface; providers map onto them. Providers:
  Railway's own bucket (below), Cloudflare R2, Hetzner Object Storage; Neon
  or Railway Postgres (both work: plain `DATABASE_URL`).

### Railway

Planned on the existing project `quod-cloud`
(40abff26-c411-4d62-b1dc-d2f7de6cd4a8, environment `production`), which
already has a `Postgres` service and the bucket `quod-results`
(484ee3fc-aecf-4708-b7af-7712ca632a9e, region iad). Nothing here has been
deployed or connected to.

1. Service `quod-server` from this repository, builder Dockerfile,
   `RAILWAY_DOCKERFILE_PATH=Dockerfile.server` (multi-stage; the runtime image
   is debian-slim with the one binary, running as a non-root user). Region:
   us-east (iad), the bucket's region and near GitHub's runners.
2. Healthcheck path `/healthz`. Railway sets `PORT`; the server listens on it.
3. Variables:

   | Variable | Value |
   |---|---|
   | `DATABASE_URL` | `${{Postgres.DATABASE_URL}}` (the private-network URL; not `DATABASE_PUBLIC_URL`) |
   | `S3_ENDPOINT` | `${{quod-results.ENDPOINT}}` |
   | `S3_BUCKET` | `${{quod-results.BUCKET}}` |
   | `S3_REGION` | `${{quod-results.REGION}}` |
   | `S3_ACCESS_KEY_ID` | `${{quod-results.ACCESS_KEY_ID}}` |
   | `S3_SECRET_ACCESS_KEY` | `${{quod-results.SECRET_ACCESS_KEY}}` |
   | `QUOD_SIGNING_KEY` | sealed variable, from `quod-server keygen` run once offline |
   | `QUOD_SERVER_CONFIG_JSON` | teams and OIDC (tokens are for self-hosting; Cloud runners use OIDC) |
   | `RUST_LOG` | `quod=info` |

   The bucket's Credentials tab exposes `BUCKET`, `ACCESS_KEY_ID`,
   `SECRET_ACCESS_KEY`, `REGION` and `ENDPOINT` as referenceable variables
   (and says whether it wants path-style URLs, which sets
   `S3_FORCE_PATH_STYLE`). The server reads only the `S3_*` names, so the
   mapping above is the whole integration; a Hetzner or R2 bucket is the same
   five variables with other values.
4. Custom domain `api.quodci.com` (a CNAME to the Railway domain); TLS ends
   at Railway's edge.
5. Runners: `QUOD_SERVER=https://api.quodci.com`,
   `QUOD_SERVER_KEY=<the public key from keygen>`, and
   `permissions: id-token: write` in the workflow (what `quodci/setup` will
   set up).

Until P4's backends exist this build refuses `DATABASE_URL` and `S3_*`, so
deploying it to Railway as configured above would fail at start, on purpose,
rather than keep results on an ephemeral disk.

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
  S1 a fresh runner replays what another proved; S2 concurrency (commits and
  claims); S3 forks and non-writers never poison trusted; S4 team isolation;
  S5 untrusted key, dead server, refused token: the step runs; S6 zero false
  hits over 133 mutations of every recorded input (content and mode of each
  file, each file removed, each directory listing, each absent probe), A7's
  method through the server; S7 OIDC against a local JWKS (expired, wrong
  audience, unlinked owner, tampered, `alg: none`, HS256 under the RSA key's
  id); S8 the dashboard reads and the event stream; resumable large blobs;
  the protocol header; 409 on unuploaded blobs.
- quod-cli `tests/server.rs` (Linux, `QUOD_INTEGRATION=1`): the real `quod
  run` with two state dirs; runner 2 replays runner 1's proof byte for byte
  while neither local ledger holds a manifest; a changed input executes; a
  dead server executes.

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
