# Whole-job replay: a hit commit costs fixed overhead only

2026-11-04, branch `exp/whole-job-replay`. Architecture for an implementer
with no other context. Code citations are at `exp/supported-v1` (32b630c,
the `bin-32b630c` the 14-repo bench ran) and `pango07/quod-action`
`exp/supported-v1` (76297ed). Build on exp/supported-v1, not main.

## 1. Goal, success, kill

Today Quod memoizes vitest test files only. On the 14-repo drop-in bench
(ci-bench run 41916046652) that is 5–30% of a job's runner-seconds; the rest
is runner boot and checkout (~15 s), install (1.5–36 s), build (vercel-ai
141 s, shiki 28 s per commit) and vitest's own start-up per invocation
(vercel-ai: 120 invocations, ~170 s wall with 96% of files replayed). A
perfect test memoizer caps at 1.5–4x per job. The goal is that a commit whose
recorded inputs are all unchanged (a **hit**) does no install, no build and
starts no vitest: the job is checkout plus the action's verdict.

Measured as real GitHub jobs (section 6), from the jobs API:

- **Success:** hit jobs ≤ 30 s wall and ≤ 0.3x plain runner-seconds; corpus
  cost (every commit, hits and misses, fixed overhead included) ≤ 0.6x plain
  on the two larger repos.
- **Kill:** hit jobs still > 60 s after milestone 5. Then the per-job floor
  eats the gain, and 15x/50% needs the runner not to boot (a GitHub App
  verdict), which needs a workflow `if:` and is not drop-in.

## 2. Invariants

1. **Never wrong.** Install, build and tests are skipped only on Quod's own
   proof: every input the step read is byte-identical to a record that two
   independent runs agreed on (the second-witness policy,
   `crates/quod-cli/src/vitest.rs:284-298`). turbo's and Nx's hashes are a
   delivery channel, never a proof (section 3.4). A failure never replays.
   Doubt of any kind means the step runs.
2. **Drop-in.** One action line after `setup-node` plus
   `permissions: id-token: write`. No `if:`, no cache steps, no changes to
   scripts. Everything happens inside the action's main step, its post step
   and the `node` shim it puts first on PATH (`quod-action/src/main.js:256-277`).
3. **Never slower.** A miss costs ≤ 5% over plain (section 4.2). Anything
   unsupported is detected, said in one line, and runs plain.

## 3. Design

### 3.1 Job start: the verdict before install

Today `main.js:216` (`run()`) restores dependencies (`main.js:86-170`), points
turbo/Nx at Quod Cloud (`main.js:172-212`), installs quod, restores the ledger
from the Actions cache (`main.js:335`) and starts Cloud restores *in the
background* (`main.js:382-418`), then writes `config.json` for the shim. There
is no plan step; `quod vitest --plan` (`vitest.rs:1419`) exists but nothing
calls it, and it is pnpm-only (`install_key`, `vitest.rs:1736-1739`).

New: after the ledger is restored, the action runs **`quod job plan`** (new
subcommand, `crates/quod-cli/src/job.rs`), with a deadline of 10 s on the
Cloud restore (past it: verdict `run`). It computes, from the checkout, the
environment and the restored ledger:

1. the **install key** (section 3.2) and whether a trusted node_modules tree
   exists for it;
2. for every recorded **build unit** (section 3.3), its key and whether a
   trusted record exists;
3. for every recorded **vitest suite**, `--plan`'s selection with
   node_modules keys taken as recorded (`vitest.rs:1419-1524`): `done` only if
   every file is witnessed (`vitest.rs:1485-1491`) and a render exists
   (`vitest.rs:1153-1166`).

It writes `plan.json` next to `config.json`: `{install: hit|run,
builds: {unit: hit|run}, suites: {id: done|run}}`, and starts prefetching
(in the background, while checkout-time steps finish) what the hits need: the
node_modules tree, build outputs, renders. The shim then acts on it per
invocation (3.5). Nothing in the customer's workflow changes: `pnpm install`,
`turbo run build`, `vitest run` are still invoked by their steps; each
returns at once when its unit is a hit, runs plain otherwise.

A hit job is therefore: boot, checkout, setup-node, the action (ledger
restore + plan + prefetch, target ≤ 5 s), install (returns at once), build
(returns at once per unit), test (render emitted, ≤ 50 ms per invocation).

### 3.2 Install: proven and restored

Proof: `install_key` (`vitest.rs:1736-1771`): node, platform, package
manager version, quod's code hash, every manifest and lockfile in the
workspace, `~/.npmrc`, `npm_config_*`/`pnpm_*`/`corepack_*` variables. Today
pnpm only. Generalize using the action's `deps.cjs:142` (`key()`, which
already handles npm, pnpm, yarn classic/berry and bun manifests,
`deps.cjs:61-129`): port that detection to Rust so action and quod compute
one key (test: the two agree on fixtures for all four managers).

Restore: today the action restores `node_modules` for npm only
(`main.js:102-128`) and only the *store* for pnpm, yarn, bun
(`main.js:130-170`), so their install still runs (zustand 1.5 s, shiki 36 s).
Change: save and restore the **workspace node_modules tree** for every
manager (pnpm's and yarn-berry's trees are relocatable within the same
absolute workspace path; the key includes `GITHUB_WORKSPACE`), with the
`.quod-deps.json` marker the npm path writes (`hook.cjs:215`). The shim's
`npmCi` (`hook.cjs:171-223`) generalizes to `pnpm install --frozen-lockfile`,
`yarn install --immutable|--frozen-lockfile`, `bun install --frozen-lockfile`:
when the marker's key equals the job's install key and the tree print
(`deps.cjs:239`, `fullPrint`) is unchanged, print one line and exit 0;
otherwise run the real install. pnpm's own `node` shim path: pnpm is a node
script, so the shim sees it (yarn's execPath case is handled at
`hook.cjs:152-167`; bun is native and is intercepted by a `bun` shim beside
the `node` shim, same logic).

Source of the tree: Actions cache first (existing `cache.restoreCache`,
`main.js:108`, `post.js:69-80`), Quod Cloud's blob store as the second tier
(`crates/quod-server/src/blobs.rs`, `s3.rs`) when the tree exceeds the cache
budget. Save only from trusted refs (`maySave`, `main.js:76`).

### 3.3 Build units: identified by invocation, proven by Quod

Without touching the workflow, the build is whatever runs between install and
tests. The shim sees every `node` process. A **build unit** is a one-shot node
process whose entry is a known build tool (tsc, tsup, tsdown, unbuild, rollup,
vite build, esbuild's JS entry, swc, babel, next build, rolldown) or a
package script whose name starts with `build`, with `cwd` = the package
directory (its nearest `package.json`). Under turbo/Nx the orchestrator spawns
`pnpm run build` per package and the shim sees one unit per package
(`TURBO_HASH`/`NX_TASK_HASH` present, cf. `alone()`, `vitest.rs:2384-2393`).

**Key (over-approximate on purpose, quod-fs's rule "over-record, never
under-record", `crates/quod-fs/src/lib.rs:5-7`):** install key, tool facts
(node, quod), the command line, the environment minus volatile names
(`quod-key/src/lib.rs:333-342`), and the content tree of the unit's package
directory minus its own outputs, plus the content trees of every workspace
package in its `dependencies`/`devDependencies`/`peerDependencies` closure
(and their outputs, since a package builds against its dependencies' dist).
A unit whose package has no workspace dependencies and no `package.json` is
keyed on the whole workspace tree. Native children (esbuild, swc, oxc) read
nothing outside this set by construction; a read outside the workspace and
node_modules (a build reading `/etc`, the network, the clock) is not proven
and shows up as **non-identical outputs**, below.

**Outputs:** the paths the unit created or changed, by a before/after walk of
the workspace with the stat identity used for the ledger's stat cache
(`vitest.rs:2462`, `ident`) plus content hash on change. A unit that writes
outside the workspace (except `$TMPDIR`, `.turbo`, `node_modules/.cache`) is
unrecordable. Only exit code 0 is recorded.

**Witnesses:** the record is trusted when two independent runs with the same
key produced byte-identical outputs (same `Witness` shape as tests,
`vitest.rs:265-282`). Builds with timestamps in sourcemaps or banners never
agree and therefore never replay: safe, and reported in one line
("build of packages/ai is not deterministic (dist/index.js.map)").

**Replay:** the shim restores the outputs into place (prefetched by the plan),
prints the recorded stdout/stderr, exits 0. Outputs live in the Cloud blob
store, Actions cache as fallback; the record (key, witness, output manifest)
in the ledger directory beside the vitest suites, saved by the post step
(`post.js:119`).

### 3.4 turbo and Nx: our proof gates their cache

The action already points turbo and Nx at Quod Cloud when the repo has no
remote cache of its own (`main.js:172-212`; server routes
`crates/quod-server/src/taskcache.rs:551-563`). Today the server answers on
the orchestrator's hash alone. Change: a build unit records the orchestrator
hash it ran under (`TURBO_HASH`, `NX_TASK_HASH`). The server stores the
artifact *and* the Quod build-unit key it was produced under. It serves an
artifact for hash H only when the requesting job's plan (3.1) has posted, for
this job id, the set of build-unit keys that are hits; H must map to one of
them. turbo's hash therefore selects, Quod's key authorizes. If the plan
could not run (no ledger, deadline), the server answers "miss" and turbo
builds. A repo with its own remote cache keeps it (we never override), and
whole-job replay is then limited to install and tests for that repo.

### 3.5 Tests: render, no vitest start

`quod vitest` already renders a full replay after the run that makes every
record trusted (`vitest.rs:1153-1166`, `render` at `vitest.rs:1894`) and emits
it without starting vitest (`emit_render`, `vitest.rs:1968`; the hook's path
`hook.cjs:125-128`). On the bench it was declined everywhere:

- `driver.mjs:384` compares `p.config.dir` (a relative string such as
  `tests`) with the absolute cwd: "test dir tests is outside …". Resolve
  against `p.config.root` first. Same for `p.config.root` at `driver.mjs:383`.
- `driver.mjs:127-128` and `:386`: coverage disables render. Keep (coverage
  output must be real; `--coverage` repos get install+build replay only).
- The render covers only known reporters (`driver.mjs:380`); a custom
  reporter means a vitest start with everything replayed (today's 1.7 s
  floor). Acceptable.

`--plan`'s suite verdict (3.1) reuses `plan()` as is, with `install_key`
generalized.

### 3.6 Partial hits and the fallback

Every unit decides alone. Install hit + build miss: the install returns at
once, the builds run and record, the suites run with the files whose inputs
changed (today's behaviour). Any doubt (no plan, deadline, marker mismatch,
an output path missing from the store, a prefetch still running when the
step arrives: wait ≤ 20 s then run) means the real command runs, with
recording. Exit codes of replayed steps are the recorded ones (always 0).
`QUOD_DISABLE=1` turns everything off (`main.js:219`).

## 4. Prerequisites folded in

### 4.1 Timeout margin

The one real wrong replay: `packages/mistral/src/mistral-transcription-stream.test.ts`
passed at ~2.3 s against a 5 s timeout, later timed out under load, and Quod
replayed "passed". Rule: a test whose duration exceeds **half** its effective
timeout is **near-timeout**; its file is recorded but never trusted while any
witness saw a near-timeout test. The driver records per test
`duration` and the timeout in force (`ctx.config.testTimeout`, the project's,
and `hookTimeout`; `testStates`, `driver.mjs:518`, gains both), the ledger's
`Rec` (`vitest.rs:236-262`) gains `near_timeout: bool`, `witnessed()`
(`vitest.rs:298`) returns false when set, and the run report says why. A later
witness with every test ≤ 0.5x clears it.

### 4.2 Recording overhead ≤ 5% on misses

Measured on all-run commits: +17% typical (zustand, convex), +46–67% mcp-sdk,
+105% outline (vitest "setup" 23.5 → 83.5 s under hooks), +27–68% vercel-ai.
Half the commits miss, so this alone decides whether cost < 1. Suspects, in
order, each measured on bench-dropin's all-run commits before and after:

1. **V8 precise coverage in every worker** (`coverage.mjs`, started from
   `setup.mjs`) feeds function-level narrowing. A first witness cannot replay,
   so take coverage only on the second witness run. Expected to remove most
   of mcp-sdk's overhead.
2. **Stack capture per read** for attribution ("read by at …"): capture only
   for env and package.json reads, and once per (path, caller) pair.
3. **fs hooks on setup-heavy files** (`record.mjs:36` originals, `:576` env
   Proxy, `:629`/`:687` `registerHooks`): collect keys in a Set, hash at
   teardown; no per-call string building.
4. The failed-file plain re-run (`confirm_failures`, `vitest.rs:1773`) only
   on failures: fine.

Acceptance: `quod_ms/plain_ms ≤ 1.05` on zustand, mcp-sdk, outline, vercel-ai
all-run commits, 0 recording differences.

## 5. Milestones

Each: files, tests, acceptance. In order; the first proves the most for the
least.

**M0. Measure real jobs (no product code).** ci-bench `bench-job.yml`
(section 6) runs plain vs the action as shipped (`dependencies`, `task-cache`,
`cloud` on). Acceptance: real fixed overhead per job measured; the 15 s
assumption in `scripts/vitest/ci-metrics.mjs:23` replaced by the measured
value. Kill early if boot+checkout+setup-node alone exceed 40 s.

**M1. Render on every full-hit invocation.** `driver.mjs:383-386`;
`crates/quod-cli/tests/vitest_e2e.rs`: a project with `dir: 'tests'`,
one with `root` set, renders; one with coverage does not. Acceptance:
bench-dropin zustand full-hit commits ≤ 0.2 s test step (was 1.7 s);
vercel-ai hit commits test step ≤ 90 s (was 170 s); 0 wrong.

**M2. Timeout margin and recording overhead.** `driver.mjs:518`,
`vitest.rs:236-298`, `setup.mjs`, `coverage.mjs`, `record.mjs`;
`tests/vitest_recorder.rs`: a test at 0.6x its timeout is recorded near-timeout
and never replays; one at 0.4x does; a second witness at 0.4x clears it.
Acceptance: 4.2's numbers.

**M3. Install for every manager.** `vitest.rs:1736` (`install_key` → shared
`crates/quod-key/src/install.rs`), `quod-action/deps.cjs`, `hook.cjs:171-223`
(generalized `npmCi`), `main.js:86-170`, `post.js:44-81`; `tests/install.rs`:
key equality across Rust and `deps.cjs` fixtures (npm, pnpm, yarn classic,
yarn berry node-modules linker, bun); shim answers each manager's frozen
install only with a matching marker. Acceptance: bench-dropin with
`dependencies: true`: install step ≤ 2 s on hit commits for zustand (pnpm),
convex-helpers (npm), yup (yarn berry), resolvers (bun).

**M4. Build units.** New `crates/quod-build` (key, output walk, witness,
restore), `hook.cjs` (unit detection and replay), `job.rs` (plan entries),
`quod-server/src/taskcache.rs` (gated serving), `blobs.rs`;
`tests/build_unit.rs`: deterministic tsup fixture replays after two witnesses;
a fixture with a timestamp never does; a change in a workspace dependency's
source misses; turbo artifact served only with a posted hit key.
Acceptance: shiki (pnpm+turbo) and vercel-ai build step ≤ 10 s on hit commits
with outputs byte-identical to a plain build (bench compares `dist/` trees).

**M5. The plan at job start and prefetch.** `job.rs` (`quod job plan`),
`main.js` (await Cloud restore ≤ 10 s, run plan, prefetch), `vitest.rs:1419`
(`plan` generalized). `tests/job_plan.rs`: plan verdicts for hit, install
miss, build miss, no ledger, deadline. Acceptance: section 6 success
criteria on bench-job.yml.

## 6. The bench-job.yml experiment (public pango07/ci-bench)

One real GitHub job per (repo, commit, variant); timings from
`GET /repos/{o}/{r}/actions/runs/{id}/jobs` (`started_at` → `completed_at`
for wall and runner-seconds, `run_started_at` for queue), never in-job timers.
A driver job runs commits in order per repo (dispatching a reusable
`bench-job-one.yml` with `GITHUB_TOKEN`, waiting for each), so the ledger of
commit i is what commit i+1 restores, as in production.

- **Repos and commits:** 6 consecutive first-parent commits ending at
  `scripts/dropin/corpus.json`'s sha for zustand (`382d29c48`; pnpm, small
  control), convex-helpers (npm, 110 s tests), mcp-sdk (pnpm `-r`, 13
  invocations), vercel-ai (`add01abd2`; pnpm+turbo, 141 s build). shiki is
  out (coverage on: no render).
- **Variants:** `plain` (the repo's own steps: setup, install, build, test);
  `quod-shipped` (M0: the action at defaults); `quod-whole` (after M5).
- **Per job, as the repo's CI has it:** checkout, pnpm/bun/corepack,
  setup-node, the action line, install, build, test. The commit's own
  `package.json` scripts, untouched.
- **Report:** per commit wall and runner-seconds for each variant; hit
  commits flagged from Quod's log; corpus cost = sum of runner-seconds.
  Success and kill as in section 1. The job verifies the test step's JSON
  report against `plain`'s per file and per test (compare.mjs), and `dist/`
  trees against `plain`'s.

## 7. Risks and decisions for the founder

1. **Build-unit scope.** Package closure (more hits; a relative import across
   packages outside declared dependencies is an under-record) vs whole
   workspace tree (fewer hits, no such risk). Proposal: closure, and the
   whole tree when a package has no `package.json` dependencies at all; the
   witness rule catches the rest only if outputs differ. Decide.
2. **Storage and egress.** node_modules trees (vercel-ai ~1–2 GB) and build
   outputs through Quod Cloud on every hit; the Actions cache's 10 GB budget
   is the customer's. Pricing follows.
3. **Repos with their own turbo/Nx remote cache** get install and tests only.
4. **Coverage repos** get install and build only (render is off).
5. **PR events:** a fork PR records nothing (`main.js:287-295`); hits come
   from the base branch's ledger.
6. **Kill decision at M0:** if measured fixed overhead is > 40 s, in-job
   whole-job replay cannot reach ≤ 30 s and the bet moves to a GitHub App
   verdict, which is not drop-in.
