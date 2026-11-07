# Native-origin engine: full 12-commit window

This historical engine has been superseded by the
[corrected engine's Cloud-enabled window](convex-helpers-current-cloud-window.md).
Use that report for the current measured result; its storage configuration
and default metadata policy differ from this experiment.

The target is **not met**. Engine `081f20a`, Action `1aa08ca`, and harness
`62a5c2d` completed the same cold 12-commit window at
[run 42167641326](https://github.com/pango07/ci-bench/actions/runs/42167641326).
All 48 expected jobs finished successfully. All 24 Quod/plain comparisons
agree on the per-test names/states fingerprint, normalized output-line
multiplicity, and job conclusion. This observed agreement is not a general
correctness sign-off: a separate [unit metadata regression](unit-metadata-audit.md)
reproduces eight stale results in this shipped engine.

| Variant | Window runner-s | Ratio to plain | Rounded minutes | c1–c11 runner-s | Ratio to plain | Rounded minutes c1–c11 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Plain | 1,862 | 1.000 | 36 | 1,695 | 1.000 | 33 |
| Quod, timing off | 1,561 | 0.838 | 32 | 1,381 | 0.815 | 29 |
| Quod, timing replay | 1,424 | 0.765 | 29 | 1,259 | 0.743 | 26 |

The whole-window reductions are 16.2% runner-seconds / 11.1% rounded
minutes for the default variant, and 23.5% / 19.4% for the timing variant.
Rounded minutes mean the sum of `ceil(job runner-seconds / 60)`, a
paid-runner cost model. Actual GitHub charges for these public jobs are $0.
Queue time and the benchmark coordinator are excluded from measured job
runner time. These are observed comparisons, not promised savings.

There are **no full hits**, so neither best nor median hit can be reported
as `floor + N s` for this window. Median job runner times are 177.5 s plain,
127.5 s default Quod, and 116.5 s timing Quod. Each Quod variant runs all
1,440 file/project instances and replays zero Vitest files. Each replays
five units. Both have zero lineage promotions, audits, disagreements, and
revocations; this window does not exercise lineage-audit correctness.

Every job used `ubuntu-26.04/20261101.186.1`. The exact image and run URL for
each of the 48 jobs are in the [per-job report](convex-helpers-native-origin-report.md).
The [verification JSON](convex-helpers-native-origin-window.json) lists
wrong=0 separately for each variant at every commit c0–c11 and records the
source artifact digests. [Raw job evidence](convex-helpers-native-origin-jobs.jsonl)
preserves the timestamps, signatures, fingerprints and Quod summaries.
An independent calculation checked all 48 unique expected job identities,
paired commits, successful conclusions, sums, rounding, and multiplicity.

Fresh-checkout metadata changes invalidate the shared Vitest inputs on
every later commit. As a result, E1 never receives a plain-trusted
predecessor to promote. The main improvement in this window comes from
dependency installation caching. The difference between timing variants
does not establish a replay-policy benefit: neither actually replays tests,
and plain test-step times themselves vary from 51 to 112 seconds.

The next performance work remains a sound way to validate preparation
outputs at the worker boundary, or a complete proof of the metadata fields
that preparation consumes. Ignoring timestamps would restore misleading
numbers and is not an acceptable fix. Units must also pass the new
metadata regressions before a replacement action is packaged.
