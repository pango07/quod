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

`.github/workflows/bench-vitest.yml` mode `shadow`
(`scripts/bench-vitest/shadow.sh`). It runs on the same 20 vuejs/core
commits, and per commit, in sequence: plain, the critical path, then that
commit's shadow. It reports the critical path against plain (the headline),
with shadow compute as its own column. The full-plain per-file comparison
and the mutation check are kept. `SHADOW_LAG=1` makes every critical path
read the recording from two commits back, which simulates a shadow that
has not finished. Numbers: see the run linked in the branch's report.
