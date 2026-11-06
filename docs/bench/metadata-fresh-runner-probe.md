# Metadata-correct engine: fresh-runner probe

The corrected engine passes its adversarial regression and packaged Action
gate, but **does not yet recover Vitest reuse across fresh runners**. All
three Quod jobs executed all 120 files at the same source commit. Do not
apply the historical 45.2% / 41.7% savings or floor + 3 s hit to this engine.

This is a diagnostic at convex-helpers
`5eaa7e5160736054d53f8b71e2c522835eb427e0`, not the 12-commit window:

- Engine `15cb5ab317bde6ce1c067f8655a85ed670910d4b`.
- Action `9cf6d91874b38009acfde64c80b540f226415b40`, with the exact JavaScript
  bundle produced by [packaged gate 42137916242](https://github.com/pango07/quod/actions/runs/42137916242).
- Binary archive SHA-256 `eb75a5194e2cb97c443597299327b26bfe288b31730ff412b48237719d7d6db6`.
- Harness `f9fdd1033026e5a19c4402b9c2b70f069b78dfff`, diagnostic branch
  `bench/metadata-15cb5ab`. Plain/floor jobs do not prepare Quod actions.
- Names proof and lineage on; Boolean-manifest and timing replay off;
  Cloud and task cache off. Existing install/build/test commands unchanged.

The Quod jobs ran sequentially, each restoring the preceding job's actual
ledger. The first job had no dependency tree cached on this branch. It did
restore an older main-branch ledger, but the recorder-version boundary
rejected its test records: zero recorded files were available and all 120
were recorded afresh. Both npm units also recorded first witnesses.

## Real jobs

| Job | Run | Runner-s | Rounded min | Vitest replayed | Runner image |
|---|---|---:|---:|---:|---|
| Plain 1 | [42138265658](https://github.com/pango07/ci-bench/actions/runs/42138265658) | 178 | 3 | — | ubuntu-26.04/20261101.186.1 |
| Quod initial recording | [42138285985](https://github.com/pango07/ci-bench/actions/runs/42138285985) | 121 | 3 | 0 | ubuntu-26.04/20261101.186.1 |
| Quod reuse attempt 1 | [42138632952](https://github.com/pango07/ci-bench/actions/runs/42138632952) | 74 | 2 | 0 | ubuntu-26.04/20261101.186.1 |
| Quod reuse attempt 2 | [42138831356](https://github.com/pango07/ci-bench/actions/runs/42138831356) | 119 | 2 | 0 | ubuntu-26.04/20261025.179.1 |
| Plain 2 | [42138841991](https://github.com/pango07/ci-bench/actions/runs/42138841991) | 148 | 3 | — | ubuntu-26.04/20261101.186.1 |
| Floor | [42138852582](https://github.com/pango07/ci-bench/actions/runs/42138852582) | 12 | 1 | — | ubuntu-26.04/20261101.186.1 |

All five test jobs succeeded and their complete fingerprints matched:
120 file/project instances, 4,131 per-test names/states, SHA-256
`09ebb9e3a722220d3a21ddfe9f63992a8a074b0fcaa95764e54c37be90d469cd`.
Wrong = 0 for each of the three Quod/plain comparisons. There were **zero
lineage promotions and zero plain lineage audits**, so this probe supplies
no additional audit-agreement evidence. The packaged fixture separately ran
one plain lineage audit with zero disagreements.

There were no full hits; best/median hit and a hit expressed as floor + N s
are unavailable for this candidate. Public-job charges were $0. Rounded
minutes are per-job estimates before credits, not actual charges. There are
three Quod jobs and two plain jobs, so their raw totals cannot be compared
as a savings ratio. This small, repeated-commit probe does not establish
window or c1–c11 savings. The baseline varied by 30 seconds and the third
Quod job used an older image.

The [machine-readable evidence](metadata-fresh-runner-probe.json) includes
job IDs, step durations, fingerprints, images and the reported changed keys.

## What prevents reuse

The first reuse attempt restored dependencies and skipped `npm ci`, but
49 of 1,912 recorded input values changed. The second had 51 changes on the
older image. The first reported shared changes in both jobs were `sm:`
metadata for the checkout root, project directories and test files. Any
changed shared input currently reruns all 120 files. These were new recordings,
not plain confirmations; no Vitest records became trusted.

Unit agreement still works. Build confirmed across `d:node_modules` in the
second job. On the older image, both build and test:release confirmed across
changes to `envall:`, `/etc/ld.so.cache` and `host:` with identical input
names. Those executions are still charged; agreement makes future matching
records eligible, it does not erase the current miss cost.

The next full window should wait until an unchanged-input real job can
recover test reuse. Weakening metadata keys to restore the old speed would
reintroduce the demonstrated correctness bug.

## Runtime read-site diagnostic

[Gate 42139029246](https://github.com/pango07/quod/actions/runs/42139029246),
probe `7ddbc93`, executed the complete current Vitest suite with a main-process
read observer and names-only stat proof. It used the current manifests,
hooked before importing Vitest, produced no ledger and authorized no replay.
Its 120 files / 4,131 states independently matched the real-job fingerprint.
The 23 stat/Boolean proof regressions also passed.

The key new source is Vitest's `FilesStatsCache.updateStats`: 39 promise-stat
reads of test files. The executed library source awaits a stat and retains
only `stats.size` in its scheduling cache. A size-only proof could avoid
fresh-checkout timestamp/inode invalidation, but it must establish the
asynchronous result's provenance and all consumers; the filename and matching
outputs are not proof. No such async refinement is enabled here.

The remaining sites include direct synchronous stats for three project/root
directories, Node's async realpath lstat/stat calls, fdir callbacks and eight
environment-file probes. Node's realpath implementation uses inode/device
identity for symlinks, so treating every one of its reads as type-only would
be incorrect without a more specific proof.

The diagnostic also saw 234 temporary module-cache stat paths. It does not
model the product driver's complete scratch/cache exclusions, so these are
not evidence of 234 additional real replay blockers. It observes the main
process only and does not establish worker-input completeness. The 288 `sm:`,
12 `sl:` and 101 `st:` diagnostic paths must not be conflated with the real
job's changed shared-input counts.
