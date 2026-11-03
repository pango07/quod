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
  (its own output).
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

vitest 4.x (the driver uses `createVitest`, `_testRun`, `_reportFileTask`,
the module runner's `evaluatedModules` and the worker's `onCleanup`); vitest
3.1 projects were not recordable (ufo's older commits). Node with
`module.registerHooks` (22.15+, 23.5+).

## Residual risks

1. Inputs read before Quod's setup file runs in a worker that are neither
   modules of the graph nor in vitest's or the environment's package
   closure (the environment's own native reads). The observer backstop
   covers these when it ran.
2. pnpm's store paths name versions; a non-pnpm layout that patches a
   package in place without changing its package.json is not seen.
3. Environment reads in a worker before Quod's setup file (vitest's own)
   are not per-file; the main process's enumeration keys all of the
   environment (but per-run CI variables) for every file anyway.
4. A test that depends on another test file's side effects (a shared temp
   file outside the workspace): isolation makes in-memory sharing
   impossible; files are caught by audits.

## Measured

ci-bench (pango07/ci-bench, GitHub-hosted runners), inline recording,
bin-b09f2a4, 20 first-parent commits each, every commit's plain and Quod
JSON reports compared file by file and test by test:

| project | plain median | Quod median | median speedup | total | wrong | recording differences |
|---|---:|---:|---:|---:|---:|---:|
| vuejs/router (b810a7e) | 15.0 s | 2.4 s | 6.3x | 300 s to 66 s (4.5x) | 0 | 0 |
| vuejs/core (68f25a4) | 24.0 s | 12.4 s | 2.0x | 481 s to 227 s (2.1x) | 0 | 0 |

Runs: router https://github.com/pango07/ci-bench/actions/runs/41561553026,
vue https://github.com/pango07/ci-bench/actions/runs/41561573192. The
shadow-recording line (exp/vitest-shadow) takes recording off the critical
path; its numbers are in docs/vitest-shadow.md.
