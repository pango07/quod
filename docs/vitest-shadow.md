# vitest replay with shadow recording (`quod vitest --no-record`)

2026-11-02, branch exp/vitest-shadow (built on exp/vitest-fast; see
docs/vitest.md for selection, replay, the recorder and audits). The idea:
recording costs time (coverage per test file, hooks, the dump), so the
developer's CI job never records. It replays what the most recent completed
recording proves unchanged and runs the rest with plain vitest. A second,
non-blocking job of the same commit records.

## The two jobs

**Critical path** (the developer's check): `quod vitest --no-record --dir L`.
- Selection is exp/vitest-fast's, unchanged (keys, shared inputs, tool facts,
  quarantine, function-level narrowing). Nothing is forked.
- Files that must run go through vitest with no setup file, no
  main-process hooks, no coverage and no audits. Replays are injected into
  the run's reporters as usual, so the summary, the exit code and the
  JSON/JUnit output are those of a full run.
- The ledger is read, never written.
- First run ever (no ledger): every file runs, so the job is plain vitest
  under the driver.

**Shadow** (a separate job, `continue-on-error`, never a required check):
`quod vitest --dir L --audits 4 --expect-report <the critical path's vitest JSON report>`.
- It restores the same way, re-selects against its ledger, and runs the
  affected files with full recording. On a first run that means all of
  them, and the observer backstop (`observe.sh`) can run there too.
- It audits up to 4 replayable files. A mismatch quarantines the file in
  the ledger, so the next critical path runs it.
- **Recording must not change behaviour.** A file records only if its
  per-test results in the shadow match the critical path's (uninstrumented)
  results for the same commit (`--expect-report`). On a mismatch the shadow
  logs "recording changed behaviour" and quarantines the file. A file the
  report lacks is not recorded. This is how the bench caught an
  instrumentation that changed test names and outcomes on vuejs/router: the
  recorded results would otherwise have been replayed as wrong results.
- It saves the ledger for this commit. Its exit code and report are
  diagnostics only: the developer's verdict is the critical path's.

**Why any completed ledger is safe to read.** Every record carries the
states of its inputs, and selection recomputes them on the current tree.
So a ledger from an older commit, a sibling branch or a re-run only changes
*how much* replays, never *whether a replay is right*. This is what makes
the fallback trivial: when the shadow of commit N-1 has not finished, the
critical path of commit N reads the newest ledger that exists (N-2, N-3, ...)
and the files whose inputs changed since then run.

## Per-run variables while recording (QUOD_VITEST_HIDE)

A recording worker lists the environment without the per-run variables:
the commit, the run id, the event file, the runner's per-step paths, and
Quod's own. Code that copies the whole environment therefore never holds
one, and the file can record. Without this, an SDK that copies
`process.env` when it loads made 88 of class-management's backend files
unrecordable. A read of such a variable by name still sees it, and that
read makes the file unrecordable.

**Residual.** Take a per-run variable that is copied by a listing and later
read through the copy. While recording, the copy has no such variable. In
a plain run, it has one.
- If the outcome depends on whether the variable is present (a test
  expects it, or code branches on it), the shadow's comparison with the
  critical path catches the difference. The file is quarantined and runs.
  Test: `a_hidden_variable_read_through_an_env_copy_quarantines_the_file`.
- If the outcome depends only on the value, and only when one is present,
  the comparison does not catch it. Plain runs compute with this run's
  value; the recording computed with none, and a replay reports that. Such
  a result changes from run to run by construction (the value is per-run),
  so it was never repeatable. The residual case is a result that a per-run
  value changes only in a way the test's pass/fail does not show, such as
  logged output or a snapshot of the copied environment. Those differ in a
  replay. Pass/fail and test names do not.

## The hand-off: actions/cache, keyed by branch and commit

The ledger is one directory (`ledger.json`, `results/<taskId>.json`,
`blobs/<blake3>`). Each completed shadow saves it under an immutable key
`quod-vitest-<branch>-<sha>`. A job restores with that exact key, falling
back to the prefixes `quod-vitest-<branch>-` and then
`quod-vitest-<default branch>-`. For a prefix, actions/cache returns the
**most recently created** match, which is exactly "the most recent completed
recording". A shadow that is still running has saved nothing, so it is never
read half-written. Re-running a commit finds its own entry; saving the same
key twice is a no-op.

```yaml
jobs:
  test:                       # the critical path: the required check
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - run: pnpm install --frozen-lockfile        # the project's own setup
      - uses: actions/cache/restore@v4
        with:
          path: .quod/vitest
          key: quod-vitest-${{ github.ref_name }}-${{ github.sha }}
          restore-keys: |
            quod-vitest-${{ github.ref_name }}-
            quod-vitest-${{ github.event.repository.default_branch }}-
      - run: quod vitest --no-record --dir .quod/vitest -- run --reporter=default --reporter=json --outputFile.json=report.json
      - uses: actions/upload-artifact@v4
        if: always()
        with: { name: vitest-report, path: report.json }

  quod-shadow:                # recording, off the critical path
    needs: test               # for its report; the check has already reported
    if: always() && needs.test.result != 'cancelled'
    runs-on: ubuntu-latest
    continue-on-error: true
    # Fork PRs never write a ledger (see Trust below).
    if: github.event.pull_request.head.repo.fork != true
    steps:
      - uses: actions/checkout@v4
      - run: pnpm install --frozen-lockfile
      - uses: actions/cache/restore@v4
        id: restore
        with:
          path: .quod/vitest
          key: quod-vitest-${{ github.ref_name }}-${{ github.sha }}
          restore-keys: |
            quod-vitest-${{ github.ref_name }}-
            quod-vitest-${{ github.event.repository.default_branch }}-
      - uses: actions/download-artifact@v4
        with: { name: vitest-report }
      - run: quod vitest --dir .quod/vitest --audits 4 --expect-report report.json -- run --reporter=default --reporter=json --outputFile.json=report.json || true
      - uses: actions/cache/save@v4
        if: steps.restore.outputs.cache-hit != 'true'
        with:
          path: .quod/vitest
          key: quod-vitest-${{ github.ref_name }}-${{ github.sha }}
```

Notes:
- The shadow `needs` the critical job only for its report (the same
  vitest arguments, so the same keys). The developer's check has reported
  by then; the shadow costs no wall time on it. The workflow's overall
  status waits for the shadow, but the required check is the `test` job,
  and the shadow cannot fail anything. (Variant: start the shadow in
  parallel and fetch the report only at the end of its run; the report is
  read after the tests ran.)
- Cache scope: a branch reads its own entries and the default branch's.
  A PR's entries live in the PR's merge-ref scope, so `github.ref_name`
  there is `<n>/merge` and the prefix falls back to the default branch.
- Size: a few MB per commit (the bench reports it). Every save is a new
  entry, so old entries need pruning (the repo-wide 10 GB LRU does it
  eventually; a scheduled `gh cache delete` of all but the newest few per
  branch keeps it small).
- **Trust.** The ledger is an input that can make a test report "pass"
  without running. Whoever can write a ledger the critical path reads can
  hide a failure, so only jobs on trusted refs save one. Fork PRs are
  excluded by the `if:` above: they read the base branch's ledger and run
  everything else.
- An artifact works too (upload per commit, look up the newest successful
  shadow run's artifact for the branch through the API), but it needs that
  lookup code. The cache prefix rule gives "newest completed" for free.
- **Quod Cloud, later:** the same directory, stored per (repo, branch, sha).
  The lookup there can do better than "newest created": it can return the
  ledger of the **nearest completed ancestor** of the commit, so fewer files
  re-run after a rebase or a branch switch. The cloud can also keep a
  merged ledger (the newest record per test file across recent shadows)
  instead of one snapshot per commit, and it can accept ledgers only from
  trusted runners.

## Measured

Bench mode `shadow` (`scripts/bench-vitest/shadow.sh`, `report-shadow.mjs`).
It runs in this repo's `bench-vitest.yml` and, for the numbers below, in
the public pango07/ci-bench, which downloads a `bin-<sha>` release built
from this branch. Per commit, in sequence: plain, the critical path, then
that commit's shadow (with `--expect-report` of the critical path's
report). It reports the critical path against plain (the headline), with
shadow compute as its own column. Every critical-path report is compared
with plain's per file and per test, and four commits get a mutation check.
Times come from a monotonic clock: the hosted runner's wall clock is stepped
during the job. `SHADOW_LAG=1` makes every critical path read the recording
from two commits back, which simulates a shadow that has not finished.

20 first-parent commits each, GitHub-hosted public runners (4 CPUs);
headline binary bin-486f82b (2026-11-03). Plain's speed differs between
runners, so compare within a row:

| tier | project | plain total | critical path total | speedup (total / median) | shadow compute | wrong | mutations |
|---|---|---:|---:|---:|---:|---:|---|
| 1 | unjs/ufo (13 files, ~1.5 s) | 31.6 s | 23.2 s | 1.36x / 1.0x | 32.3 s | 0 | 4/4 |
| 2 | vuejs/router (81 files, ~9.3 s) | 185.3 s | 38.1 s | 4.9x / 6.9x | 49.4 s | 0 | 4/4 |
| 3 | vuejs/core (184 files, ~41 s) | 820.0 s | 285.5 s | 2.87x / 4.6x | 352.7 s | 0 | 4/4 |

Runs (pango07/ci-bench): ufo 41566245303, router 41566249277, core
41566252339. Earlier binary bin-29742b8: ufo 1.39x, router 5.0x, core 2.50x
(41561706644, 41561710020, 41561713091).

- When every file must run (the first run, or a shared input changed), the
  critical path costs the same as plain: tier 1 ±0.1 s, tier 3 median
  -0.2%, worst +0.8%. Quod's own work outside vitest is 3-10 ms, and the
  driver's vitest start-up replaces the CLI's.
- Lag 1 on router (bin-a074f76): 2.74x against 3.9x at lag 0 on the same
  binary, 0 wrong.
- The safety net, validated: bin-a5e5bf0 had an instrumentation that
  changed test names and outcomes in 3 router files. The shadow flagged
  those files "recording changed behaviour" and quarantined them, and the
  run had 0 wrong replays. Without the net, the same binary replayed 2 files
  wrongly on 18 commits.
- Runs: lag-1
  41560601953, safety net 41561416787 (pango07/ci-bench).

## Future: a sound persistent transform cache (designed, not built)

Measured same-job on router (ci-bench run 41568734381): vitest's own
`experimental.fsModuleCache`, populated by the shadow and read by the
critical path, cut the critical path 7.7% in total and 7% at the median
(2.49 → 2.26 s on commits that run 1-14 files). It is not shipped because
its key (module id and content, NODE_ENV, the environment's config, plugin
*names*, config-file contents, the lockfile hash) does not cover everything
a transform can depend on. A stale hit runs old code and nothing notices.
The gaps:

- **Other module files read during a transform.** An SFC's imported prop
  types (`defineProps<Props>()` from `./types.ts`), a macro or codegen plugin
  that reads sibling files, a plugin that inlines another file. Quod's
  shared inputs deliberately exclude `f:` reads of files that are themselves
  transformed modules (each test file's module graph accounts for those), so
  a change to `types.ts` changes neither the cache key of the SFC nor a
  shared input.
- **Plugin behaviour.** Options and code beyond the plugin's name; covered
  only indirectly, through the config-file contents and the lockfile.
- **Resolution.** Import specifiers are rewritten to resolved ids. Vite's
  JS resolver probes are main-process reads, which are shared inputs.
  A native resolver's probes are invisible to the hooks.

A sound version, keyed by Quod and never by vitest's key alone:

1. **Per-transform read attribution (record runs).** Wrap every
   environment's `transformRequest` in `AsyncLocalStorage.run({ file })`.
   In the main-process fs hooks, a read inside that context of any path
   other than the module's own file is a *foreign read* of that module. A
   read of a transformed module's file outside any transform context
   (buildStart, configResolved, a plugin's own cache warm-up) is a foreign
   read of *every* module. The union of foreign-read files is F. Because
   vite's and plugins' in-process caches mean a later transform may reuse a
   read without repeating it, F is global, not per module.
2. **Key.** The entry for module X in environment E is blake3 of:
   - X's id and content;
   - the current states of every shared (global) input;
   - the contents of every file in F;
   - E's name, the vite and vitest versions, and quod's driver hash.

   The Rust side already hashes all of these during selection and passes
   one `transformSig` to the driver, so a key costs nothing extra. Any
   change to a shared input or to F changes every key: the cache starts a
   new generation, never serves stale output.
3. **Plugin allow-list.** Serve hits only when every plugin with a
   `transform` or `load` hook in the environment is on a list whose
   behaviour on a module depends only on that module's id, content and the
   config: vite's core plugins, vitest's own, `@vitejs/plugin-vue` for
   non-SFC modules (and for SFCs only with F covering their type imports),
   `@vitejs/plugin-react`. Any other plugin disables the cache for that
   environment. Also bail on `import.meta.glob`, as vitest does, and on any
   module whose transform reached a native resolver or child process the
   hooks cannot see.
4. **Where.** Entries are written only by the shadow, from transforms it
   ran fresh. The critical path reads them through a wrapped
   `transformRequest`, returning the stored `{ code, map }` with the module
   graph updated as vite would. Entries travel in the hand-off; size is
   bounded by evicting entries not hit in the last N recordings.
5. **Verification.** The shadow re-transforms every module it reaches (it
   must anyway, to record). Where an entry exists under the same key it
   compares bytes. A mismatch is a design bug: it disables the cache for
   the repository, records why, and the bench fails loudly. It is not a
   sampling rate.

This is recording-side work (the recorder and the ledger), not run-phase
work.
