# Current packaged recorder: full-suite CPU profile

[Gate 42350116176](https://github.com/pango07/quod/actions/runs/42350116176)
passed using engine `f9d8153` from Action
`e7098a6b5e5b8d460d0f63af782b25aaa23d4934`, with its embedded JavaScript
and a fresh disposable ledger. Harness `450c7bc` ran convex-helpers c11
(`5eaa7e5160736054d53f8b71e2c522835eb427e0`) on one GitHub runner.
This profiles the shipped recorder, not the separate worker-boundary
experiment. No observers or input checks were disabled.

| Phase | Seconds | File/project instances | Test states |
| --- | ---: | ---: | ---: |
| Plain before | 67.356 | 120 | 4,131 |
| Recorded, CPU profiler active | 76.153 | 120 | 4,131 |
| Plain after | 67.411 | 120 | 4,131 |

All phases produced the same file/test names and states: 4,130 passed and
one skipped. Independent comparison of the downloaded raw reports agrees.
There were zero replays. The recorded phase was 8.770 seconds, or 13.0%,
above the mean of its plain neighbors. This includes profiler cost only
on the recorded side; one fixed-order triplet does not establish a stable
overhead estimate or any new job savings.

Across 116 collected CPU profiles, environment caller attribution accounts
for 7.487 sampled inclusive CPU-seconds. Its cache-key construction accounts
for 4.702 seconds and its uncached path for 2.325 seconds. These are
overlapping inclusive costs, not additive wall-time savings. Most of this
work is spread across short-lived processes. The largest per-profile
caller-attribution total is 0.126 seconds. Optimizing it could reduce miss
cost, but this profile does not support treating it as the missing replay
breakthrough.

The default policy retained 395 `sm:`, 257 `sl:` and zero `st:` keys.
The earlier driver diagnostic with optional proofs enabled reported a
different key set; its 39 successful shared size-query inputs must not be
described as the only metadata dependencies of the default shipped path.
The fresh-runner window remains the authority for actual invalidations.

Node was 24.18.0, on `ubuntu26/20261101.186.1`. The artifact archive matches
GitHub's SHA-256. All 125 file digests, the three report digests, 116 profile
digests, log digest and aggregate samples are retained in the
[evidence record](current-recorder-profile.json). Three negative sample
deltas totaling nine microseconds were preserved as signed values.
Collected profile counts do not prove complete process coverage.

The corrected engine's Cloud-enabled 12-commit window completed separately
at [42349931075](https://github.com/pango07/ci-bench/actions/runs/42349931075).
It uses harness `8131a61`, the same Action pin, the original 12 commits,
`floor,plain,quod,quod-ts`, and no repeated warm-up commit. Cloud reporting
and storage costs are included. The [verified window result](convex-helpers-current-cloud-window.md)
is 11.4% fewer runner-seconds and 11.1% fewer modeled billed minutes for
default Quod, with zero observed mismatches and no full hits. This profile
does not establish additional savings.
