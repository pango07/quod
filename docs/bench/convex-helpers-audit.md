# convex-helpers benchmark audit — 2026-11-05

Audited baseline: Quod `c0f55ad` (fresh branch `exp/whole-job-e-codex`),
quod-action `27d36d3`, ci-bench `bench/whole-job`.

## What the evidence establishes

The [12-commit run](https://github.com/pango07/ci-bench/actions/runs/42048941374)
contains these totals, recomputed from its downloaded `bench-job` artifact:

| scope | plain runner-s / rounded minutes | quod | quod-ts |
|---|---:|---:|---:|
| c0–c11 | 1777 / 34 | 1328 / 27 (0.75× seconds) | 1103 / 23 (0.62× seconds) |
| c1–c11 | 1606 / 31 | 1136 / 23 (0.71× seconds) | 910 / 19 (0.57× seconds) |

Minutes are the harness's sum of per-job `ceil(runner_seconds/60)`, not a
charge: these public runners are free. This window does not meet the
0.50× runner-seconds bar. The faster-hit evidence in the handoff comes
from earlier runs; the 12-commit artifact classifies every observed Quod
job as a miss (some files still ran/audited), so it cannot establish a new
best or median full hit.

The original artifact lacked the default-quod c3 signature. Independently
retrieved [c3 job logs](https://github.com/pango07/ci-bench/actions/runs/42050560208)
contain 128 normalized lines, matching plain's signature exactly. With
that recovery, both variants have zero signature/conclusion mismatches
at each of c0–c11. The observed images were
`ubuntu-26.04/20261025.179.1` and `ubuntu-26.04/20261101.186.1`.

This is empirical evidence, not a proof that all observable behavior is
identical. The harness compares normalized per-file test lines, totals,
and job conclusions. It does not independently compare every passing
test's name/state or generated build artifacts.

## Correctness fixes

1. `pj_fields` discarded dependency version values even when configuration
   code read them. A config can choose a suite based on an unloaded
   package's declared version. Nested accessors now record dependency
   values that are actually read; names-only reads remain narrow. Spread
   and structuredClone read values and therefore key them conservatively.
2. Missing traces became empty text, and empty traces parsed successfully.
   Missing/invalid UTF-8/empty traces and unfinished/unmatched resumed
   syscalls now refuse recording rather than claiming an empty read set.
3. Missing stdout/stderr blobs became empty text in replay and witness
   comparison. Unit candidates now verify content-addressed text and
   output blobs. Missing/corrupt evidence forces execution.
4. A reproduced permission-only change (`0644` to `0755`) changed an
   `fs.accessSync(path, X_OK)` outcome while the recorded `e:` key/state
   stayed identical. Access queries now retain their mode and key the
   actual permission/ACL result, including inside installed packages.
   Native faccessat queries retain their effective-ID flag; unsupported
   flags fail closed.
5. Output snapshot/read/save failures could fabricate empty output content.
   These errors now prevent recording/promotion. Existing corrupt blobs
   are replaced when their genuine content is recorded again.

Unit VERSION is now 4; vitest LEDGER_VERSION is now 3. Old records must
re-prove rather than retain trust under corrected key semantics. These
fixes have not yet been measured in a new real-job window.

## Benchmark fixes

On ci-bench branch `bench/whole-job-audit`, the report fails for wrong,
unchecked, or omitted expected jobs; comparison retains duplicate-line
counts; drive.sh propagates that failure, preserves all Quod diagnostic
lines, and retries transient log-download failures. Previously drive.sh
unconditionally exited 0. Five lightweight report regressions pass.

## Limits and next work

The accepted witness model is an empirical check of recorder coverage.
D2 and E1 cannot mathematically establish that all unseen inputs varied.
Source review still warrants adversarial tests of general stat-metadata
dependence: regular-file `stat` inputs are content keys, although metadata
can affect programs. The reproduced `access` case was fixed separately. This
review does not certify the recorder as complete.

E1's sampled audit detects a recorder fault after promotion; it cannot
prevent the first bad replay. Its projected savings also need a consecutive
miss test: promotion sets `plain_n=0`, whereas the next promotion requires
a plain-trusted predecessor. Do not assume both c4 and c5 immediately
promote without testing that exact sequence.

Validation so far: the [snapshot/manifest gate](https://github.com/pango07/quod/actions/runs/42062771178)
passed 102 tests (88 CLI units, one real strace/bubblewrap end-to-end test,
13 recorder tests) and clippy. The [permission-query gate](https://github.com/pango07/quod/actions/runs/42064185771)
passed 105 tests (90 CLI units, one real strace/bubblewrap end-to-end test,
14 recorder tests) and clippy, including the permission-state, native-query
and recorder regressions. The remote rustfmt corrections were applied to
the final source; no cargo command ran on the Mac.

No E1/E2 implementation, action input, action binary, or fresh 12-commit
bench was shipped as part of this audit. No production deployment, tag
movement, PR, or main-branch push occurred. Action post already skips a
wholly replayed job; reducing miss uploads needs a delta/semantic-change
implementation, not another unconditional skip or an upload abandoned at
runner teardown.
