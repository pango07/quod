# Configuration names proof: paired changed-commit jobs

Latest follow-up: [the names-proof 12-commit window](config-names-proof-window.md) measured 968/1766 runner-seconds (0.548×), c1–c11 790/1636 (0.483×), with zero wrong comparisons; separate best hit floor + 3 s. The cold-inclusive 0.50× bar remains open.

[Driver 42099468869](https://github.com/pango07/ci-bench/actions/runs/42099468869)
completed all 16 jobs. This is a short diagnostic, separate from the
[original 12-commit window](convex-helpers-e-window.md). It does not establish
that the whole-window 0.5x cost target is met.

Both Quod variants used binary `b9e34fc4c12a27cbb9c219121b2d29a7bf0684f3`,
action `0af7cd7d9330577124ba93dc3ed30bfcb38d2ca5`, timing-sensitive replay and
lineage witness. Only `QUOD_VITEST_NAMES_PROOF` differed (0 control, 1 proof).
Harness `90123bccb8345151a246c110521b03d1d08ed1db` ran c9, repeated c9 for the
plain confirmation, then c10 (Chalk) and c11 (Commander). The repeated
position is matched to its own plain run. Cold recording and confirmation
costs remain in the all-jobs totals.

| Scope | Plain runner-s / rounded min | Control | Proof | Proof / plain runner-s / min |
|---|---:|---:|---:|---:|
| All four jobs | 651 / 14 | 558 / 11 | 379 / 8 | 0.582 / 0.571 |
| After first job, including confirmation | 514 / 11 | 343 / 7 | 216 / 5 | 0.420 / 0.455 |
| c10–c11 dependency upgrades | 330 / 7 | 240 / 5 | 78 / 2 | 0.236 / 0.286 |

The changed jobs took 47 s and 31 s with proof, versus 184 s and 146 s plain.
That is 252 runner-seconds (76%) and five rounded minutes (71%) avoided on
those two jobs. Against the identical-binary control, proof removed 162
runner-seconds (68%). Including cold recording and confirmation, it saved
272 seconds (42%) versus plain. These scopes are not interchangeable, and
speed/cost percentages must not be added. Rounded minutes are summed after
rounding each job upward to a minute. These public-runner jobs cost $0;
rounded minutes are a comparison, not a claim of an actual invoice saving.

Proof preserved 116 replays on each dependency upgrade; two affected files
ran, plus two ordinary recorded audits. Neither job was a whole-job hit.
There is therefore no new best/median full-hit measurement here. Historical
warm hits on the earlier binary remain best **floor + 4 s**, median
**floor + 6 s** (19 s best, 20 s median absolute), as documented in the full
window report. They are excluded from these totals.

All eight Quod jobs independently matched the corresponding plain job's
120 file names and 4,131 per-test names/states, plus the harness's normalized
summary and conclusions: wrong = 0 at each of the four positions for both
variants. The independently fetched GitHub job timestamps also reproduce
every reported runner duration. The per-test fingerprint was
`09ebb9e3a722220d3a21ddfe9f63992a8a074b0fcaa95764e54c37be90d469cd`.

Lineage: control promoted 117 records; proof promoted none because the
unaffected records stayed valid. **Zero lineage plain audits ran and zero
lineage disagreements were observed.** The four ordinary recorded audits
in the proof jobs must not be counted as lineage plain audits. The separate
packaged-action gate covers an agreeing lineage audit and the regression
suite covers disagreement revocation. No universal correctness claim is
made from these passing comparisons; the metadata gap in the audit report
remains open.

## Each measured job

All images were Ubuntu 26.04. Image version, run URL, seconds and per-job
rounded minutes are preserved below; cold control used an older image.

| Variant | Position | Commit | Image version | Run | Runner-s | Rounded min | Wrong |
|---|---:|---|---|---|---:|---:|---:|
| floor | 0 | c976cabdf | 20261101.186.1 | [42099495986](https://github.com/pango07/ci-bench/actions/runs/42099495986) | 15 | 1 | — |
| floor | 1 | c976cabdf | 20261101.186.1 | [42099501484](https://github.com/pango07/ci-bench/actions/runs/42099501484) | 12 | 1 | — |
| floor | 2 | 6f830b66f | 20261025.179.1 | [42099506307](https://github.com/pango07/ci-bench/actions/runs/42099506307) | 14 | 1 | — |
| floor | 3 | 5eaa7e516 | 20261025.179.1 | [42099510581](https://github.com/pango07/ci-bench/actions/runs/42099510581) | 12 | 1 | — |
| plain | 0 | c976cabdf | 20261101.186.1 | [42099493603](https://github.com/pango07/ci-bench/actions/runs/42099493603) | 137 | 3 | — |
| plain | 1 | c976cabdf | 20261101.186.1 | [42099499474](https://github.com/pango07/ci-bench/actions/runs/42099499474) | 184 | 4 | — |
| plain | 2 | 6f830b66f | 20261101.186.1 | [42099504497](https://github.com/pango07/ci-bench/actions/runs/42099504497) | 184 | 4 | — |
| plain | 3 | 5eaa7e516 | 20261101.186.1 | [42099509415](https://github.com/pango07/ci-bench/actions/runs/42099509415) | 146 | 3 | — |
| quod-ts-control | 0 | c976cabdf | 20261025.179.1 | [42099497002](https://github.com/pango07/ci-bench/actions/runs/42099497002) | 215 | 4 | 0 |
| quod-ts-control | 1 | c976cabdf | 20261101.186.1 | [42100106752](https://github.com/pango07/ci-bench/actions/runs/42100106752) | 103 | 2 | 0 |
| quod-ts-control | 2 | 6f830b66f | 20261101.186.1 | [42100466176](https://github.com/pango07/ci-bench/actions/runs/42100466176) | 150 | 3 | 0 |
| quod-ts-control | 3 | 5eaa7e516 | 20261101.186.1 | [42100908378](https://github.com/pango07/ci-bench/actions/runs/42100908378) | 90 | 2 | 0 |
| quod-ts-proof | 0 | c976cabdf | 20261101.186.1 | [42099496461](https://github.com/pango07/ci-bench/actions/runs/42099496461) | 163 | 3 | 0 |
| quod-ts-proof | 1 | c976cabdf | 20261101.186.1 | [42100021656](https://github.com/pango07/ci-bench/actions/runs/42100021656) | 138 | 3 | 0 |
| quod-ts-proof | 2 | 6f830b66f | 20261101.186.1 | [42100466338](https://github.com/pango07/ci-bench/actions/runs/42100466338) | 47 | 1 | 0 |
| quod-ts-proof | 3 | 5eaa7e516 | 20261101.186.1 | [42100661992](https://github.com/pango07/ci-bench/actions/runs/42100661992) | 31 | 1 | 0 |

## Reproducibility and validation

The shipped binary archive SHA-256 is
`caca43ddfeefd7bf05f771865ac69ac1b80a4bfad03bca7f667fa775400c2ac4`.
[Linux audit/binary gate](https://github.com/pango07/quod/actions/runs/42098123727),
[actual configuration gate](https://github.com/pango07/quod/actions/runs/42098119329)
and [packaged action gate](https://github.com/pango07/quod/actions/runs/42098902044)
passed before measurement. The packaged gate checks warm replay, harmless
version changes, disabling the flag, adding dependency names and an actual
version-dependent branch against plain test states.

The driver artifact contains `jobs.jsonl`, `job-metrics.json`,
`expected.jsonl` and the generated report. All 16 logs and job metadata were
also fetched independently for verification. No running bench was cancelled.
The bounded proof remains opt-in; it does not narrow npm script unit keys,
replace Vitest, or assume arbitrary configuration outputs prove equivalence.
