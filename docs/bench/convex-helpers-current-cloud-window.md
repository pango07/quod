# Corrected engine, Cloud enabled: original 12-commit window

The target is **not met**. [Run 42349931075](https://github.com/pango07/ci-bench/actions/runs/42349931075)
completed all 48 expected jobs successfully with engine `f9d8153`, Action
`e7098a6b5e5b8d460d0f63af782b25aaa23d4934`, and harness
`8131a61c3c614497d5b48a775e499010fee5f192`. This is the original cold
12-commit window ending at `5eaa7e5160736054d53f8b71e2c522835eb427e0`,
without a repeated warm-up commit. Cloud ledger storage and dashboard
reporting were enabled and their costs are included.

| Variant | Window runner-s | Ratio to plain | Rounded minutes | c1–c11 runner-s | Ratio to plain | Rounded minutes c1–c11 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Plain | 1,735 | 1.000 | 36 | 1,609 | 1.000 | 33 |
| Quod, timing off | 1,538 | 0.886 | 32 | 1,353 | 0.841 | 28 |
| Quod, timing replay | 1,497 | 0.863 | 32 | 1,311 | 0.815 | 28 |

Default Quod reduces window runner-seconds by **11.4%**, and modeled billed
minutes by **11.1%**. Excluding c0, these are **15.9%** and **15.2%**.
The timing variant reduces window runner-seconds by 13.7%, and the same
11.1% of modeled minutes; c1–c11 reductions are 18.5% and 15.2%.
The difference between variants is not evidence of a timing-policy benefit:
neither replays any tests. These are observed measurements, not guarantees.

Rounded minutes are the sum of `ceil(each job's runner-seconds / 60)`.
These public-repository jobs incur no GitHub runner charge; the minute
figures model paid usage. Queue time and the benchmark coordinator are
excluded from the measured product jobs. Time and cost savings are separate
measures and must not be added together.

There are **zero full hits**, so best and median hit are **not available**;
no result in this window can be called `floor + N s` for a hit. The floor
has a 9.5 s median and an 8–12 s range. Median job runner times are 135.5 s
plain, 128 s default Quod and 126 s timing Quod.

Each Quod variant executes all 1,440 file/project instances, replays zero
Vitest files and replays zero units. Both have zero lineage promotions,
audits, disagreements and revocations. This window therefore does not
exercise the lineage audit; the separate Action gate does.

| Commit | Default wrong | Timing replay wrong |
| --- | ---: | ---: |
| c0 | 0 | 0 |
| c1 | 0 | 0 |
| c2 | 0 | 0 |
| c3 | 0 | 0 |
| c4 | 0 | 0 |
| c5 | 0 | 0 |
| c6 | 0 | 0 |
| c7 | 0 | 0 |
| c8 | 0 | 0 |
| c9 | 0 | 0 |
| c10 | 0 | 0 |
| c11 | 0 | 0 |

All 24 comparisons agree with plain on the per-test names/states
fingerprint, normalized test-output line multiplicity and successful job
conclusion. This establishes observed agreement for this corpus, not a
universal correctness guarantee.

Every measured job used `ubuntu-26.04/20261101.186.1`. The
[per-job report](convex-helpers-current-cloud-report.md) and
[independent verification](convex-helpers-current-cloud-window.json) retain
each job's image, commit, run URL, timing and result. The
[raw job evidence](convex-helpers-current-cloud-jobs.jsonl) preserves the
timestamps, fingerprints, signatures and Quod log summaries. An independent
calculation verified all 48 unique expected identities, the exact original
commit sequence, terminal success states, harness SHA, per-job rounding,
comparison multiplicity and aggregate arithmetic.
The [artifact audit script](../../scripts/bench-vitest/audit-convex-cloud-window.py)
recomputes these totals and comparisons from the downloaded evidence.

Artifact `13566037281` is 29,269 bytes; its SHA-256 is
`771af4d43220cd23bd5a58c090a875502e35c6323d3bd59be4885dfdedad6661`.
The downloaded archive matched GitHub's digest. Individual evidence file
digests and the final GitHub run identities are retained in the JSON.

The improvement comes from dependency installation caching. Default
installation steps total 120 s versus 591 s plain, saving 471 s. Its test
steps cost 193 s more than plain, and its other work and overhead consume
another 81 s, leaving 197 s net savings. Inline Cloud ledger saves account
for 82.286 s of the default test steps, measured by the CLI; these times
are already included in the job totals.

Shared filesystem metadata changes invalidate every Vitest file on each
later commit. The corrected unit observer also rejects metadata on a file
descriptor without a stable pathname: `build` and `test:release` are marked
to run plain after that first observation. The implementation must not
ignore these observations to recover old benchmark numbers.

The default window must shed another 670.5 runner-seconds to reach 0.5x
plain. Even eliminating every measured Cloud-save second would leave it
well above that bar. [Deferred Cloud saves](cloud-post-save.md) can reduce
fixed waiting, but safe test reuse remains the central requirement.
