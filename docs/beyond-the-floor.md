# Beyond the floor: how low a GitHub Actions job can go, and how Quod gets there

2026-11-05, design memo for the founder. Companion to `whole-job-replay.md`.
Numbers are from the jobs API of the M0 runs on public pango07/ci-bench
(bench-job-one, run family 41945160971; Opus's 41962900243), read with
`created_at`, `started_at`, `completed_at` per job and per step.

## 1. The floor, measured

A hosted `ubuntu-latest` job whose body does nothing costs this (convex,
mcp-sdk and zustand jobs agree within a second):

| segment | seconds | who controls it |
|---|---:|---|
| dispatch → job created | 1–2 | GitHub |
| job created → runner started (queue) | 2 | GitHub |
| runner started → first step | 1 | GitHub |
| Set up job (fetches every action used) | 1–2 | GitHub; grows with action size |
| `actions/checkout` | 1–2 | customer step |
| `actions/setup-node` (convex: `.nvmrc` 24.18, not in the image) | 4 | customer step |
| pnpm/action-setup (zustand) | 3 | customer step |
| post steps + Complete job | 1–2 | GitHub + actions' post |

**Floor ≈ 11–13 s wall for convex-helpers** (queue included, body zero),
≈ 9 s for a job whose Node version is preinstalled. The target of 11–16 s is
the floor plus at most 3–5 s, so every one of the three customer commands
(`npm ci`, `npm run build`, `npm run test`) must return in ≤ 0.5 s and the
action itself must cost ≤ 1 s in main and ≤ 0.5 s in post.

What the shipped action costs instead: main 1–6 s (Node starting a 5.5 MB
bundle, a 5.8 MB quod tarball download, OIDC + Cloud session, ledger restore),
post 1–7 s (ledger save: mcp-sdk 5 s, convex 7 s), plus recording in the test
step (+30%: convex test 108–141 s vs 77–113 plain; mcp-sdk 143 vs 65 s). The
dependency cache never hit in M0 ("node_modules changed after `npm ci`; not
saved", then "no node_modules saved yet" on the next commit: the build writes
into node_modules, so the save is refused). That is why Quod loses whole jobs
today: +15–80 s of its own cost against 0 s of savings on commits that cannot
yet replay.

One billing fact changes the cost arithmetic: **GitHub bills hosted jobs per
job, rounded up to the minute.** Plain convex (137–184 s) bills 3 minutes; a
15 s hit bills 1. Runner-seconds say 0.09x, the invoice says 0.33x. For
self-hosted and Blacksmith-class runners the invoice is per second. Report
both.

## 2. Ideas, ranked

Estimates are for a convex-helpers hit commit (plain 165 s, 2.75 billed
minutes). Hit rate on convex's 12-commit window was 6/12.

### 1. The near-zero action (build this first)

Main becomes a 2 KB JavaScript stub that spawns a **static musl `quod`
shipped inside the action's `dist/`** (no download: GitHub fetches the action
tarball during Set up job anyway; +6 MB ≈ +0.3 s there). The binary does
everything main does today in Rust: restore the ledger, write the shim, write
`config.json`. **A hit job saves nothing**: no ledger change, no node_modules
save, no Cloud upload, so post is a stat and exit. The shim answers `npm ci`,
`npm run build` and `npm run test` from `hook.cjs` *before npm loads*
(`--require` runs first; today the hook lets npm start and only intercepts
vitest), so each is ~60 ms of Node start instead of ~400 ms. Recording stays
off until the verdict says "miss".

- Hit job: **12–14 s wall, ~12 runner-seconds (0.07x), 1 billed minute
  (0.36x)**. Over the window at 6/12 hits with misses at ≤ 1.05x: 0.56x
  runner-seconds; ≤ 0.5x needs ≥ 56% hits or misses at plain cost.
- Build: ~1 week. The binary exists; the work is moving main/post into it and
  trimming the Node bundle.
- Never-wrong: no change in what is proven.
- Drop-in: yes.

### 2. Git-tree verdict: keys as git blob OIDs (build this second)

Selection today hashes every live key (convex: 716 ms for 120 files, "select
716 ms") after downloading the ledger. Record, beside each workspace file's
blake3, its **git blob OID**. Then the verdict for a clean checkout is
`git ls-tree -r HEAD` (10–20 ms, no file reads; git hashed the tree already)
joined against the ledger's OIDs; a dirty tree or a path outside git falls
back to blake3. The ledger needed for a verdict shrinks to (OID set, env
fingerprint, install key) → render id: a few KB, one HTTP GET from Quod Cloud
keyed by `(repo, tree OID, env fingerprint)`, ~100 ms. Install and build
units get the same treatment: their keys are tree OIDs of the package
directories, so the whole plan is a tree walk.

- Hit job: removes 1–3 s from idea 1 (ledger download + select); enables idea 4.
- Build: ~1 week (ledger schema, `quod job plan`, Cloud endpoint).
- Never-wrong: equal, provided the OID is of the committed content and the
  working tree is clean (`git status --porcelain` empty); otherwise blake3.
- Drop-in: yes.

### 3. Content-addressed node_modules and dist (for partial hits)

A full hit needs no node_modules at all: nothing executes. Partial hits do,
and today's restore is a tar of the whole tree (convex node_modules ≈ 300 MB:
10–20 s restore and extract). Replace it with Quod's CAS served lazily:
`quod-fs` (the FUSE observer; `sudo` is passwordless on hosted runners,
fuse3 is installed) mounts node_modules and dist from the store and fetches
blobs on first read. Cost becomes O(files touched), ~1 s for a test run that
reaches a few hundred packages. The same mount records what the install and
build *read*, which is the proof the build units need, with no
over-approximation.

- Hit job: unchanged (full hits do not touch it). Partial hits: −10 to −20 s.
- Build: 3–4 weeks (lazy fetch in quod-fs, blob store traffic, fallback to tar).
- Never-wrong: content-addressed; neutral or better (exact build inputs).
- Drop-in: yes.

### 4. Speculative verdict at push time (GitHub App, optional)

With idea 2, Quod Cloud can compute the verdict from the GitHub tree API the
moment the push webhook arrives, before any runner exists, and post a check
run ("Quod: 120 files proven unchanged, render ready"). The action's GET then
returns a prepared answer. For speed it buys ~0.1 s over idea 2 computed in
the job. Its value is product: the verdict is visible in seconds on the
commit, and it is the only road **below the floor** (idea 5).

- Build: 1–2 weeks (App, webhook, tree walk server-side). Infra on Railway:
  ~$5–20/month at this scale.
- Never-wrong: same proofs; the posted check is informational unless idea 5.
- Drop-in: the App install is one click per org; still one action line.

### 5. Below the floor: skip the job (opt-in, not drop-in)

GitHub offers no drop-in way for an outside party to make a job succeed
without running it: an App can cancel a run (cancelled ≠ success), cannot
complete another workflow's job, and rulesets cannot waive a required check
on Quod's say-so. The only mechanism is the customer's own
`if: needs.quod.outputs.hit != 'true'` on the job (plus a 3-line `quod` job
that asks the App). With idea 4 the verdict exists before the runner is
needed, so the gated job never boots.

- Hit commit: **~3 s wall, 0 runner-seconds, 0 billed minutes** for the
  gated job (the 3-line verdict job still bills 1 minute on hosted runners;
  on a per-second runner ≈ 5 s).
- Build: on top of 4, ~1 week.
- Never-wrong: identical proofs; the check run must carry the render and the
  proof id, as the job's log would.
- Drop-in: **no** (workflow edit). Offer it as "zero-runner mode" with a price;
  never make it the default path.

### 6. Snapshot or restore of a warmed job state

Hosted runners expose no VM or filesystem snapshot. Firecracker-class runners
(Blacksmith, Depot, Namespace) do: boot from a snapshot in 2–5 s with
node_modules and caches in place. That needs a `runs-on` change (not
drop-in) and a paid runner ($0.004–0.008/min). Quod's hit path on such a
runner is the same binary; the floor drops to ~5–7 s. Partner, do not build.

### 7. Not worth it

Faster tar/zstd for the ledger (idea 2 makes the ledger tiny); a Node
compile cache for the action (idea 1 removes Node); test-level granularity
(tests are 10–20% of the vitest step and unsound across files); shaving
`checkout` or `setup-node` (customer steps; `setup-node` at 4 s is a Node
download for `.nvmrc` 24.18 and would need an image change).

## 3. Ranking and the call

| rank | idea | convex hit wall | runner-s (ratio) | billed min | build | never-wrong | drop-in |
|---:|---|---:|---:|---:|---|---|---|
| 1 | near-zero action, hit saves nothing | 12–14 s | 12 (0.07x) | 1 (0.36x) | 1 wk | unchanged | yes |
| 2 | git-OID keys, verdict by one GET | −1 to −3 s | −2 | 1 | 1 wk | unchanged | yes |
| 3 | CAS-served node_modules/dist | partial hits −10–20 s | | | 3–4 wk | better | yes |
| 4 | push-time verdict (App) | ≈ 2 | | | 1–2 wk + infra | unchanged | one click |
| 5 | skip the job (`if:`) | ~3 s | 0 | 0 | 1 wk on 4 | unchanged | **no** |
| 6 | snapshot runners | 5–7 s | | | partner | unchanged | no |

**If the current approach stalls above ~30 s, Opus builds 1 then 2.** The
measurements say the stall is not in what is memoized but in what the action
itself costs around it: 15–80 s of Node start-ups, downloads, restores and
saves on every job, hit or miss. Idea 1 removes them; idea 2 removes the
last second and the ledger download, and makes every later idea possible.
Together they reach the floor plus ~2 s: 12–14 s on convex, which is 12–14x
on wall and 0.36x on the invoice, with the 0.5x runner-seconds target
depending on the hit rate (≥ 56%) and on misses costing no more than plain.
Below the floor exists (idea 5), costs the customer three lines, and should
be sold, not defaulted.

Measure every step as M0 does: from the jobs API, hit and miss commits both,
wall, runner-seconds and billed minutes.
