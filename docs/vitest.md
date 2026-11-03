# Test-file-level replay for vitest (`quod vitest`)

2026-11-02, branch exp/vitest-fast. `quod vitest -- run --project unit`
runs a vitest suite, replaying every test file whose recorded inputs are
unchanged and running the rest, with the summary, exit code and JSON/JUnit
output a full run gives. Code: `crates/quod-cli/src/vitest.rs` (selection,
ledger, ingest, observer backstop) and `crates/quod-cli/src/vitest/*.mjs`
(the recorder, the driver in vitest's main process, the setup file in each
worker). Benchmark: `.github/workflows/bench-vitest.yml`,
`scripts/bench-vitest/`.

Why files, not steps: on a small repo every commit changes code, so a
whole-step key misses on every commit, and Quod's fixed cost per step made
step-level replay 1.06-1.09x *slower* than plain CI. At file level a commit
re-runs the files its change can reach.

## The unit, and why attribution is exact per file

vitest (4.x) runs each test file in a fresh worker (thread or fork) when the
project isolates (the default): `isolate: true` gives every file its own
runner, terminated after it. So a worker's whole life belongs to one test
file. But a worker does not read its source itself: it asks vitest's main
process over RPC, and vite transforms the file there. So **process-tree
attribution is not exact** (the main process reads every file's source, and
under `pool: threads` all workers share the main pid). What is exact is the
worker's own view:

- **Module graph.** The worker's module runner keeps every module it
  fetched (`__vitest_worker__.evaluatedModules`); with isolation, that is
  exactly the modules this file imported, transitively, including the
  setup files.
- **Hooks in the worker** (`record.mjs`, installed by Quod's setup file,
  first in every recorded project's `setupFiles`, loaded natively from
  `node_modules/.quod-vitest`): every `fs` read, stat, existence check,
  readlink/realpath and directory listing (recursive ones as recursive
  listings); `process.env` reads (a Proxy; enumerating the environment
  keys all of it); every module node's own loader loads (ESM and CJS, via
  `module.registerHooks`), `require.cache`, `process.dlopen` (addons). The
  record is written at the worker's teardown, after the file's last hook.
- **What the hooks cannot follow makes the file unrecordable** (it always
  runs): a child process, the network (net, tls, http(s), dgram, dns,
  fetch), a worker thread, `fs.watch`, a write inside the workspace, a
  read of a per-run CI variable (`GITHUB_SHA`, ...), a missing hook
  (no `registerHooks`, no worker teardown hook), a non-isolated, vm-pool,
  browser or typecheck project.
- **Shared inputs** (every file's): everything vitest's main process read
  (the hooks are installed before vitest is imported): config and its
  bundled dependencies (`configFileDependencies`), tsconfig, plugins, the
  tools' own modules and packages, resolution probes, env reads,
  everything read during start-up, and during the run every read that is
  not a module some worker imported (those are attributed by the graph).
  vitest's glob for test files is the one thing not recorded: Quod lets
  vitest find the files and partitions *its* list, so a new or deleted
  test file is handled exactly.
- **Packages, not lockfiles.** A file under `node_modules` is keyed by its
  package (`p:<root>`: the root's path, which under pnpm names the version
  and patch, and its package.json). Plus, for every file, the packages
  vitest and the test environment (jsdom) load before Quod's setup file
  can watch: their dependency closure. So a lockfile change that bumps
  eslint re-runs nothing; one that bumps vite or jsdom re-runs everything.
  The package manager's own files under node_modules (dot entries: `.bin`
  shims, `.modules.yaml`, `.pnpm/lock.yaml`, workspace state, caches) are
  not inputs: the packages loaded are.
- **package.json, field-level only for the resolvers.** A workspace
  `package.json` read by vite's or vitest's own code (the first frame
  outside Quod is in vite, vitest, @vitest or rolldown) is keyed on the
  resolution fields only (`pj:`: name, type, exports, imports, main,
  module, browser, workspaces, vite, vitest); a
  release's `version` bump counts for nobody. Any other reader (the config,
  a plugin that injects `version`, a test's read or import) keeps the whole
  file (`f:`). Test: `tests/vitest_recorder.rs`.
- **fs.watch** keys the watched path's whole recursive content at the end
  of the file (`rc:`), except under a directory the file itself created
  (its own output, below).
- **A directory the file made outside the workspace is its own output.**
  When the file's worker creates a directory outside the workspace (the
  repository: the nearest `.git` up from the project root), by `mkdtemp`,
  `mkdir` or a recursive `mkdir` (the first directory it actually made),
  sync, promise or callback, the reads, listings, existence checks and
  watches under it from then on are not keys: nothing under it existed
  before it was made, so they can only see what this run put there (a
  `mkdtemp` path is new every run and keyed it ran every time). A check
  made before the directory existed, or after the file removed or moved
  it, stays keyed; a directory that already existed (a recursive `mkdir`
  that made nothing) or that another process made is not the file's; and
  a directory made inside the workspace is not exempt (a write there still
  makes the file unrecordable). Test: `tests/vitest_recorder.rs`.
- **vitest's arguments** are keyed, but not the output-only ones
  (`--reporter`, `--outputFile`, `--silent`, `--color`, ...); replays render
  through whatever reporters a run asks for.

## Function-level narrowing

A test file whose only changed inputs are workspace source files it reached
as modules may still replay. Recording: the worker takes V8 precise
coverage (function granularity, binary counts; never best effort, which can
drop an executed function) and maps each executed function of a workspace
module back to the original source: the module runner evaluates
`'use strict';async (...)=>{{<code>\n}}`, so a range minus that prefix is an
offset in the transformed code, and its inline source map (the last one:
vitest's module cache appends metadata after it; several sources are fine
when exactly one is the file) gives the original line and column. A file
with an executed function that maps nowhere gets no function-level data
(vite's generated SSR export getters excepted). Nothing is instrumented: an
earlier version instrumented the transformed source and changed function
source text, test names and results (wrong replays on vuejs/router); the
bench now fails on any recorded run whose results differ from plain.

Selection: Quod keeps every workspace source version by content
(`<dir>/blobs`), so `narrow.mjs` diffs the version a record saw against
the current one (Myers, by line), parses the old version with the
project's `typescript`, and checks the new one compiles (transpile, then
V8). The file replays only if every changed region lies strictly inside
the body of a function the test file never executed (an executed function
that strictly contains it is an ancestor; equal or overlapping ranges run).
Everything else runs: a change outside every body (top-level, imports,
exports, class fields, signatures), a function it executed, a parse or
compile failure, no typescript. A replay so narrowed keeps the version it
was recorded on (`pinned`) and is audited first.

A test that reads function source (`Function.prototype.toString`, watched
by `preload.cjs`, loaded into every recorded worker with `--require` before
any library can cache the original) gets no function-level data, nor does
one whose results quote text of a source file it reached.

Per test: tests of one file share module state, so skipping one test while
running another is not provably sound; Quod narrows whole files only.

**The observer backstop** (`scripts/bench-vitest/observe.sh`, `quod vitest
--absorb`): the same suite runs once under Quod's FUSE observer (`quod
run`), which sees every process's reads whatever the hooks missed (native
code, `fs` bindings, wasm, children). Every path it saw read that no
recorded key accounts for (no key names it and it is in no recorded
package) becomes a shared input of every file: over-record. Directory
listings are left out (the only one the hooks skip on purpose is vitest's
glob). It costs one observed full run, on record runs only.

## Selection

`quod vitest` loads the ledger (`<dir>/ledger.json`: interned keys, their
recorded states and stat identities, the shared keys, and per file its
dependency key ids, result path, state and duration), and computes the
current state of every live key once, in parallel threads: file content
(blake3), listings, existence and link targets, package.json, env values.
A stat cache keyed on (dev, inode, size, mtime_ns, ctime_ns, mode), kept
only for files whose times are older than 2 s when hashed (git's racy
rule, as quod-cas's import cache), skips unchanged files. A file replays
when the shared keys, vitest's arguments and the tool facts (node,
platform, vitest, recorder versions; checked again by the driver) are
unchanged, it is not quarantined, it passed (or was skipped), and none of
its keys changed. A failure never replays.

## Replay

The driver (`driver.mjs`) starts vitest through its Node API
(`createVitest` + `start`, as `startVitest` does), wraps
`specifications.getRelevantTestSpecifications` so vitest's own file list is
split into run and replay, and wraps `_testRun.end`: before the run ends,
each replayed file's recorded `File` task is reported through
`_reportFileTask` (the path `--merge-reports` uses) and its specification
added to the run's, so every reporter (default, JSON, JUnit) and the exit
code see a full run. After the run, each ran file's task is stored (back
references stripped and rebuilt) with its record.

## Audits

When the files to run leave workers idle, a random sample of replayable
files, none slower than the slowest file that runs anyway, runs instead of
replaying, in the same invocation (`--audits auto`: workers minus files to
run; none when nothing runs). Their results are compared test by test with
the recorded ones; a mismatch quarantines the file (it always runs). The
audited file's reported result is the fresh one, so an audit never makes
the job wrong and costs no wall time.

The benchmark verifies independently: every commit also runs plain vitest
and the two JSON reports are compared per file and per test; a difference
in a file Quod replayed is a wrong replay and fails the job.

## Supported versions

vitest 3.2 and later (the driver uses `createVitest`, `_testRun`,
`_reportFileTask` and the worker's `onCleanup`, all in 3.2). The worker's
module graph is vitest 4's `evaluatedModules`, or on vitest 3 vite-node's
`moduleCache` (module id -> the transformed code it evaluated, under the
same `'use strict';async (...)=>{{` wrapper, so function-level coverage maps
the same way). vitest 3 hands a forks-pool worker each transformed module
as a file in its project's temp directory (`project.tmpDir`, a new one each
run); reads there are not inputs (the driver's `QUOD_VITEST_SKIP`): the
module's own source is keyed through the graph. Before 3.2 (no
`_reportFileTask`; vitest 3.1 projects were not recordable on ufo's older
commits) the action's hook leaves vitest alone and prints one line, and
`quod vitest` itself runs plain vitest with the same arguments, nothing
recorded or hashed. Node with `module.registerHooks` (22.15+, 23.5+).

Yarn (classic and berry, node-modules linker) runs scripts with a `node`
of its own first on PATH (a wrapper in /tmp/yarn--* or /tmp/xfs-* that runs
yarn's `process.execPath`), so the action's shim makes `process.execPath`
the shim inside yarn's own process (quod-action hook.cjs `yarnNode`). Yarn
Plug'n'Play stays plain vitest (dependencies in zip archives).

### What always runs, and why

A file that starts a child process or touches the network (net, tls,
http(s), http2, dgram, dns, fetch, WebSocket) is unrecordable: what it
read lives in another process or on another machine, and nothing in the
worker can key it. It always runs, on every commit, at full cost (axios:
16 of its unit files start a local HTTP server or a child process and take
the whole ~65 s of the run; the rest replay in no time). A worker thread,
`fs.watch`, and a write inside the workspace are the same. This is by
design: replaying such a file would trust inputs Quod never saw.

### Timing-sensitive tests

Recording costs a test that reads `process.env` in a hot loop about 220 ns
a read through the recorder's Proxy (node's own `process.env` read is
~100 ns; measured on node 25, 1M reads: 104 ms plain, 324 ms through a bare
Proxy, 350 ms with the recorder's bookkeeping). reselect's dev-mode check
reads `process.env.NODE_ENV` once per recomputation; its two perf tests
recompute 1M times against a 2000 ms budget and go over it only while
recording (2.1-2.7 s). The shadow compares every file it records with the
critical path's report of the same commit; such a file ran differently, so
it is quarantined and always runs (never replayed, never wrong). The
critical path itself runs plain (`--no-record`), so the job's result is
the plain one. Precise V8 coverage (function granularity, binary) was
measured not to slow reselect's selector loop (240 ms either way).

## npm workspaces and the CI environment (exp/vitest-npm)

These rules came from the first customer, a turbo monorepo on npm workspaces
(pango07/class-management). There, each package runs vitest from its own
directory, and node_modules is hoisted to the repository root.

- **Workspace links.** A path under node_modules whose package root is a
  link to a directory outside every node_modules is a workspace package. A
  read through the link is keyed at the real path, plus the link (`e:`).
  Before, such a read was keyed as `p:`, which hashes only that package's
  package.json, so a change to the source behind the link went unseen. This
  was true for pnpm workspaces too.
- **npm's layout.** `node_modules/<name>` names no version. An npm package's
  `p:` state adds its entry in the workspace's package-lock.json (version,
  resolved, integrity), so a git or tarball dependency that moves without a
  version bump counts.
  - The driver goes in the nearest node_modules up to the repository, and is
    written atomically, because the packages run at once.
  - A write anywhere in the repository (a sibling package) makes a file
    unrecordable.
- **CI-provider variables.** `GITHUB_*`, `RUNNER_*`, `ACTIONS_*` and turbo's
  `TURBO_HASH` count only where code other than the tools reads them.
  - Tool code is vite, vitest, rolldown and the helpers they read the
    environment through: std-env, obug, debug, tinyexec and the colour
    helpers. A read counts as the tools' only when every frame on the stack
    outside Quod and node is in one of those. A test reading
    `import.meta.env.X` goes through vitest's getter, and its own frame keeps
    the read keyed.
  - The tools' copy of the whole environment to the workers (`envtool:`) is
    keyed on what node and the tools behave on: `NODE_*`, `VITE*`, `CI`,
    `TEST`, `TZ`, `LANG`, `LC_*`, `DEBUG*`, colour and terminal variables,
    `PATH`, `HOME` and `TMPDIR`. Without this, the runner image's own
    variables (`ImageVersion`, tool paths) made every VM a full run.
  - So a push's recording replays on a pull request. Test:
    `a_branch_variable_counts_only_for_the_test_that_reads_it`.
  - `SYSTEMD_EXEC_PID` and `JOURNAL_STREAM` (the runner service's) are not
    inputs.
- **`--report FILE`** writes each test's state in the shape of vitest's JSON
  report, whatever the run's reporters are. The critical path can then hand
  the shadow its `--expect-report` without adding a reporter to the user's
  run.
  - A shadow's audit is verified against its own record. Before, an audited
    file that matched was dropped from the ledger, because the report has it
    only as a replay.
- **Every CPU for large runs** is off under a task runner (`TURBO_HASH`,
  `NX_TASK_*`, `LERNA_PACKAGE_NAME`) and with `QUOD_VITEST_ALL_CPUS=0`.
  turbo's five concurrent suites oversubscribed the runner, and a test with
  a 5 s timeout timed out.

The customer-facing setup is pango07/quod-action. Its README is the "Add
Quod to your CI" page.

## Residual risks

1. Inputs read before Quod's setup file runs in a worker that are neither
   modules of the graph nor in vitest's or the environment's package
   closure (the environment's own native reads). The observer backstop
   covers these when it ran.
2. pnpm's store paths name versions, and npm's lockfile entry is part of a
   package's state. A package patched in place without changing either is
   not seen.
3. Environment reads in a worker before Quod's setup file (vitest's own)
   are not per-file. The main process's copy of the environment keys what
   node and the tools behave on for every file. A variable outside that set
   which changes a pool's behaviour before the setup file is not seen.
4. A test that depends on another test file's side effects (a shared temp
   file outside the workspace): isolation makes in-memory sharing
   impossible; files are caught by audits.

## Measured

ci-bench (pango07/ci-bench, GitHub-hosted runners), shadow recording
(the developer's job is `quod vitest --no-record`; recording runs as a
separate job), bin-b9b9d4b (exp/vitest-fast and exp/vitest-shadow at
b9b9d4b), 20 first-parent commits per project, every commit's critical-path
JSON report compared with plain vitest's file by file and test by test, and
a mutation check on 4 commits each:

| project | plain median | Quod median | median speedup | total | wrong | audits (mismatches) |
|---|---:|---:|---:|---:|---:|---:|
| vuejs/core (68f25a4) | 42.6 s | 4.1 s | 16.1x | 851 s to 223 s (3.8x) | 0 | 74 (0) |
| vuejs/router (b810a7e) | 9.1 s | 1.5 s | 6.0x | 183 s to 39 s (4.7x) | 0 | 80 (0) |
| unjs/ufo (f06c800) | 1.4 s | 1.0 s | 1.4x | 31 s to 18 s (1.8x) | 0 | 47 (0) |

Where every file had to run (the first run; vue's vite/rolldown bump) the
critical path cost -2.7% to +0.5% against plain. Shadow compute (machine
time off the critical path): vue median 7.0 s per commit, router 2.4 s, ufo
1.8 s. Whole job (install plus tests, `--plan` first, WHOLE_JOB=1) on
router: median 16.1 s to 3.3 s (4.9x); vue: median 42.4 s to 11.6 s (3.6x), 4 of
20 commits skipped the install.

Runs: vue https://github.com/pango07/ci-bench/actions/runs/41573419257,
router https://github.com/pango07/ci-bench/actions/runs/41573428966,
ufo https://github.com/pango07/ci-bench/actions/runs/41573438808,
router whole job https://github.com/pango07/ci-bench/actions/runs/41573456977,
vue whole job https://github.com/pango07/ci-bench/actions/runs/41573448188.

Earlier, inline recording (recording inside the developer's run; retired
as a shipping mode), bin-b09f2a4: router 6.3x median, vue 2.0x, 0 wrong,
0 recording differences.
