# Convex-helpers: names-proof 12-commit window

[Driver 42103710173](https://github.com/pango07/ci-bench/actions/runs/42103710173)
completed all 48 jobs in 24 minutes 43 seconds. This repeats exactly c0–c11
ending at `5eaa7e5160736054d53f8b71e2c522835eb427e0`, with a cold cache per
Quod chain and no warm-up job inside or before the measured window.

Measured versions: binary `b9e34fc4c12a27cbb9c219121b2d29a7bf0684f3`, action
`0af7cd7d9330577124ba93dc3ed30bfcb38d2ca5`, harness
`344be8d83ea6769d8a175ec49b52aa88aaa28839`. Binary archive SHA-256:
`caca43ddfeefd7bf05f771865ac69ac1b80a4bfad03bca7f667fa775400c2ac4`.
Both experimental variants enable lineage witness and the bounded names
proof. `quod-proof` uses the default timing policy (timing replay off);
`quod-ts-proof` enables timing replay. These are not new product defaults.

## Cost and correctness

| Scope | Plain runner-s / rounded min | Timing off | Timing replay |
|---|---:|---:|---:|
| c0–c11 | 1766 / 36 | 968 / 21 (0.548× s; 0.583× min) | 1062 / 24 (0.601× s; 0.667× min) |
| c1–c11 | 1636 / 33 | 790 / 18 (0.483× s; 0.545× min) | 894 / 21 (0.546× s; 0.636× min) |

The best policy saves **798 runner-seconds (45.2%)** over the entire window,
and **846 seconds (51.7%)** after the first job. It avoids 15 rounded minutes
(41.7%) across the entire window. The whole-window 0.50× runner-seconds bar
is **not met**: the budget is 883 seconds, leaving **85 seconds to remove**.
The after-cold-start runner-seconds bar is met. Minutes are summed after
rounding each job upward; these public jobs actually cost $0. Time and cost
percentages must not be added.

Wrong = **0 on every c0–c11 for both variants**. All 24 comparisons matched
their paired plain job's per-test names/states, normalized file/count
signatures (including multiplicity), and conclusion. c0–c6 had 4,129 test
states, c7 had 4,130, and c8–c11 had 4,131, always across 120 files. There
were no missing or unchecked comparisons. Every job's independently fetched
timestamps and image also matched the driver artifact and expected manifest.
These observations do not certify universal recorder completeness; the
stat-metadata audit limitation remains open.

Each policy promoted 113 records and ran **one lineage plain audit**, at
c10. Both audits agreed; no lineage disagreement or batch revocation was
observed. These are actual plain lineage audits, distinct from ordinary
recorded audits. The c4 promotions were replaced by c5's changed inputs;
they do not manufacture a plain-trusted predecessor for consecutive misses.

## Separate full-hit measurements

No changing-commit job was a full hit. After the window finished, its default-
timing branch and exact binary were reused at unchanged c11. A 26-second
confirmation job completed remaining witnesses; then three jobs replayed
all 120 test files and both npm units. Their per-test fingerprints and
normalized signatures independently matched plain c11. No probe is included
in the 12-commit cost totals.

Best hit: **floor + 3 s** (14 s absolute against an 11 s floor).
Median hit: **20 s absolute**, median matched-floor overhead **floor + 8 s**.
The best-hit ≤ floor + 8 s bar is met on the measured binary with timing
replay off. All probes and floors used `ubuntu-26.04/20261101.186.1`.

| Probe | Run | Runner-s | Matched floor | Above floor |
|---|---|---:|---|---:|
| Confirmation | [42107581215](https://github.com/pango07/ci-bench/actions/runs/42107581215) | 26 | — | — |
| Hit 1 | [42107679334](https://github.com/pango07/ci-bench/actions/runs/42107679334) | 21 | [13 s](https://github.com/pango07/ci-bench/actions/runs/42107684705) | 8 s |
| Hit 2 | [42107755227](https://github.com/pango07/ci-bench/actions/runs/42107755227) | 14 | [11 s](https://github.com/pango07/ci-bench/actions/runs/42107759200) | 3 s |
| Hit 3 | [42107835613](https://github.com/pango07/ci-bench/actions/runs/42107835613) | 20 | [12 s](https://github.com/pango07/ci-bench/actions/runs/42107840149) | 8 s |

## What still costs time

Default timing spent 591 seconds in test steps, 125 installing, 44 building,
and 70 in action/post steps. Eliminating every action/post second would
still leave 15 seconds of the gap. The dominant test steps are c0 (103 s),
c4 (94 s), c5 (52 s), c6 (121 s), and c7 (99 s). c4's exports change and
c5's removed workspace manifest invalidate shared inputs; c6 needs plain
confirmation. Timing-sensitive replay also varied substantially with runner
performance; this single window does not establish one policy as always
faster. The [paired dependency-upgrade diagnostic](config-names-proof-results.md)
is the controlled evidence for the names-proof effect.

A [native-stack attribution experiment](https://github.com/pango07/quod/actions/runs/42105950948)
on the same runner measured 51.995 s current versus 52.523 s native, with all
4,129 states matching and all prototype records marked unrecordable. It
showed no speed gain and was not adopted (probe source `93fe69a`).

A [resolver-field diagnostic](https://github.com/pango07/quod/actions/runs/42107187070)
at `cfc47b8` varied the actual c3/c4/c5 manifests with one fixed installed
dependency tree. Configuration plus selection took 128–145 ms; all cases
selected the same 9 projects/120 files and matched the controlled config
hash. Vite did read `exports` and `name` from both manifests. Therefore
ignoring those fields is not justified. The diagnostic kept the broad keys
and produced no replay ledger. Configuration equality alone still does not
prove transformed modules, resolution, closures or setup effects equivalent.

Next investigation: determine which nested export values and resolver
outputs actually influence the loaded module graph. A refinement must cover
every reader and fall back on unknown effects; npm units keep whole-file
version-sensitive keying. Neither failed experiments nor estimated savings
count toward the remaining 85 seconds.

## Validation and complete job inventory

The exact binary passed [Rust/recorder/clippy gates](https://github.com/pango07/quod/actions/runs/42098123727),
[actual loaded-configuration checks](https://github.com/pango07/quod/actions/runs/42098119329),
and the [packaged-action replay gate](https://github.com/pango07/quod/actions/runs/42098902044).
The benchmark artifact holds `expected.jsonl`, `jobs.jsonl`, `job-metrics.json`
and its report. The report below preserves all 48 run URLs, images and per-
commit wrong counts. Floor rows have no test comparison. No running bench
was cancelled, no default switch was changed, and no benchmark code was
merged or deployed to production.

### Real jobs: where a job's runner-seconds go (jobs API, seconds)

Medians per project and variant. **fixed** = runner-seconds minus install, build and test steps. **boot+setup** = "Set up job" + checkout + package-manager/node set-up (the kill line is 40 s). Queue is not billed and not in runner.

| project | variant | jobs | wall | queue | runner | boot | setup | action | install | build | test | post | **fixed** | **boot+setup** | corpus wall-s | corpus runner-s |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| convex-helpers | floor | 12 | 14 | 2 | 11 | 3 | 5 | 0 | 0 | 0 | 0 | 0 | **11** | **8** | 170 | 137 |
| convex-helpers | plain | 12 | 141 | 2 | 139 | 2 | 6 | 0 | 47 | 2 | 79 | 0 | **12** | **9** | 1797 | 1766 |
| convex-helpers | quod-proof | 12 | 67 | 2 | 65 | 3 | 5 | 3 | 6 | 3 | 38 | 3 | **17** | **8** | 993 | 968 |
| convex-helpers | quod-ts-proof | 12 | 72 | 2 | 70 | 2 | 6 | 3 | 8 | 3 | 31 | 3 | **18** | **8** | 1096 | 1062 |
| **all** | floor | 12 | 14 | 2 | 11 | 3 | 5 | 0 | 0 | 0 | 0 | 0 | **11** | **8** | | |
| **all** | plain | 12 | 141 | 2 | 139 | 2 | 6 | 0 | 47 | 2 | 79 | 0 | **12** | **9** | | |
| **all** | quod-proof | 12 | 67 | 2 | 65 | 3 | 5 | 3 | 6 | 3 | 38 | 3 | **17** | **8** | | |
| **all** | quod-ts-proof | 12 | 72 | 2 | 70 | 2 | 6 | 3 | 8 | 3 | 31 | 3 | **18** | **8** | | |

| project | variant | jobs | median wall | runner-s | x plain | billed min | x plain | wrong | unchecked |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|
| convex-helpers | floor | 12 | 14 | 137 | 0.08 | 12 | 0.33 | 0 | 0 |
| convex-helpers | plain | 12 | 141 | 1766 | 1.00 | 36 | 1.00 |  |  |
| convex-helpers | quod-proof | 12 | 67 | 968 | 0.55 | 21 | 0.58 | 0 | 0 |
| convex-helpers | quod-ts-proof | 12 | 72 | 1062 | 0.60 | 24 | 0.67 | 0 | 0 |

| variant | after-first-job runner-s | x plain | billed min | x plain | best hit above floor | median hit above floor | lineage audits | disagreed |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| quod-proof | 790 | 0.48 | 18 | 0.55 | no full hits | no full hits | 1 | 0 |
| quod-ts-proof | 894 | 0.55 | 21 | 0.64 | no full hits | no full hits | 1 | 0 |

<details><summary>per job</summary>

| project | variant | # | commit | run | image | result | Quod | wrong | wall | queue | runner | billed | boot | setup | action | install | build | test | post | fixed |
|---|---|---:|---|---|---|---|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| convex-helpers | floor | 0 | 1d7c57881 | [42103736549](https://github.com/pango07/ci-bench/actions/runs/42103736549) | ubuntu-26.04/20261101.186.1 | success |  | ? | 14 | 2 | 12 | 1 | 3 | 5 | 0 | 0 | 0 | 0 | 0 | 12 |
| convex-helpers | floor | 1 | 290b146e0 | [42103740153](https://github.com/pango07/ci-bench/actions/runs/42103740153) | ubuntu-26.04/20261101.186.1 | success |  | ? | 15 | 2 | 13 | 1 | 3 | 6 | 0 | 0 | 0 | 0 | 0 | 13 |
| convex-helpers | floor | 2 | 1815d4a72 | [42103744349](https://github.com/pango07/ci-bench/actions/runs/42103744349) | ubuntu-26.04/20261101.186.1 | success |  | ? | 13 | 2 | 11 | 1 | 3 | 5 | 0 | 0 | 0 | 0 | 0 | 11 |
| convex-helpers | floor | 3 | 03ffe84ef | [42103748184](https://github.com/pango07/ci-bench/actions/runs/42103748184) | ubuntu-26.04/20261101.186.1 | success |  | ? | 13 | 2 | 11 | 1 | 2 | 5 | 0 | 0 | 0 | 0 | 1 | 11 |
| convex-helpers | floor | 4 | 6b17e1514 | [42103752032](https://github.com/pango07/ci-bench/actions/runs/42103752032) | ubuntu-26.04/20261101.186.1 | success |  | ? | 13 | 2 | 11 | 1 | 3 | 5 | 0 | 0 | 0 | 0 | 1 | 11 |
| convex-helpers | floor | 5 | 49a82e544 | [42103756355](https://github.com/pango07/ci-bench/actions/runs/42103756355) | ubuntu-26.04/20261101.186.1 | success |  | ? | 12 | 1 | 11 | 1 | 2 | 5 | 0 | 0 | 0 | 0 | 1 | 11 |
| convex-helpers | floor | 6 | 51f7bf848 | [42103760941](https://github.com/pango07/ci-bench/actions/runs/42103760941) | ubuntu-26.04/20261101.186.1 | success |  | ? | 15 | 2 | 13 | 1 | 3 | 6 | 0 | 0 | 0 | 0 | 0 | 13 |
| convex-helpers | floor | 7 | f5c2b9e9c | [42103764271](https://github.com/pango07/ci-bench/actions/runs/42103764271) | ubuntu-26.04/20261101.186.1 | success |  | ? | 12 | 2 | 10 | 1 | 2 | 5 | 0 | 0 | 0 | 0 | 1 | 10 |
| convex-helpers | floor | 8 | d244cf22d | [42103768061](https://github.com/pango07/ci-bench/actions/runs/42103768061) | ubuntu-26.04/20261101.186.1 | success |  | ? | 12 | 2 | 10 | 1 | 1 | 6 | 0 | 0 | 0 | 0 | 0 | 10 |
| convex-helpers | floor | 9 | c976cabdf | [42103771581](https://github.com/pango07/ci-bench/actions/runs/42103771581) | ubuntu-26.04/20261101.186.1 | success |  | ? | 15 | 5 | 10 | 1 | 2 | 5 | 0 | 0 | 0 | 0 | 0 | 10 |
| convex-helpers | floor | 10 | 6f830b66f | [42103775655](https://github.com/pango07/ci-bench/actions/runs/42103775655) | ubuntu-26.04/20261101.186.1 | success |  | ? | 19 | 5 | 14 | 1 | 3 | 6 | 0 | 0 | 0 | 0 | 0 | 14 |
| convex-helpers | floor | 11 | 5eaa7e516 | [42103779624](https://github.com/pango07/ci-bench/actions/runs/42103779624) | ubuntu-26.04/20261101.186.1 | success |  | ? | 17 | 6 | 11 | 1 | 2 | 5 | 0 | 0 | 0 | 0 | 1 | 11 |
| convex-helpers | plain | 0 | 1d7c57881 | [42103733525](https://github.com/pango07/ci-bench/actions/runs/42103733525) | ubuntu-26.04/20261101.186.1 | success |  | 122 lines | 132 | 2 | 130 | 3 | 4 | 5 | 0 | 46 | 2 | 68 | 1 | 14 |
| convex-helpers | plain | 1 | 290b146e0 | [42103738287](https://github.com/pango07/ci-bench/actions/runs/42103738287) | ubuntu-26.04/20261101.186.1 | success |  | 122 lines | 134 | 3 | 131 | 3 | 5 | 7 | 0 | 47 | 1 | 68 | 0 | 15 |
| convex-helpers | plain | 2 | 1815d4a72 | [42103742552](https://github.com/pango07/ci-bench/actions/runs/42103742552) | ubuntu-26.04/20261025.179.1 | success |  | 128 lines | 101 | 1 | 100 | 2 | 3 | 4 | 0 | 34 | 2 | 54 | 0 | 10 |
| convex-helpers | plain | 3 | 03ffe84ef | [42103746730](https://github.com/pango07/ci-bench/actions/runs/42103746730) | ubuntu-26.04/20261101.186.1 | success |  | 128 lines | 135 | 2 | 133 | 3 | 2 | 6 | 0 | 47 | 1 | 74 | 0 | 11 |
| convex-helpers | plain | 4 | 6b17e1514 | [42103750264](https://github.com/pango07/ci-bench/actions/runs/42103750264) | ubuntu-26.04/20261101.186.1 | success |  | 128 lines | 143 | 2 | 141 | 3 | 4 | 5 | 0 | 46 | 2 | 79 | 0 | 14 |
| convex-helpers | plain | 5 | 49a82e544 | [42103754055](https://github.com/pango07/ci-bench/actions/runs/42103754055) | ubuntu-26.04/20261101.186.1 | success |  | 128 lines | 172 | 2 | 170 | 3 | 2 | 7 | 0 | 54 | 2 | 103 | 0 | 11 |
| convex-helpers | plain | 6 | 51f7bf848 | [42103757414](https://github.com/pango07/ci-bench/actions/runs/42103757414) | ubuntu-26.04/20261101.186.1 | success |  | 128 lines | 181 | 2 | 179 | 3 | 1 | 5 | 0 | 58 | 2 | 110 | 0 | 9 |
| convex-helpers | plain | 7 | f5c2b9e9c | [42103761185](https://github.com/pango07/ci-bench/actions/runs/42103761185) | ubuntu-26.04/20261101.186.1 | success |  | 128 lines | 184 | 2 | 182 | 4 | 2 | 6 | 0 | 56 | 2 | 110 | 1 | 14 |
| convex-helpers | plain | 8 | d244cf22d | [42103764623](https://github.com/pango07/ci-bench/actions/runs/42103764623) | ubuntu-26.04/20261101.186.1 | success |  | 128 lines | 139 | 2 | 137 | 3 | 3 | 6 | 0 | 46 | 1 | 78 | 0 | 12 |
| convex-helpers | plain | 9 | c976cabdf | [42103768729](https://github.com/pango07/ci-bench/actions/runs/42103768729) | ubuntu-26.04/20261101.186.1 | success |  | 128 lines | 106 | 5 | 101 | 2 | 2 | 5 | 0 | 35 | 1 | 55 | 0 | 10 |
| convex-helpers | plain | 10 | 6f830b66f | [42103772371](https://github.com/pango07/ci-bench/actions/runs/42103772371) | ubuntu-26.04/20261101.186.1 | success |  | 128 lines | 183 | 3 | 180 | 3 | 2 | 7 | 0 | 57 | 2 | 109 | 0 | 12 |
| convex-helpers | plain | 11 | 5eaa7e516 | [42103776328](https://github.com/pango07/ci-bench/actions/runs/42103776328) | ubuntu-26.04/20261025.179.1 | success |  | 128 lines | 187 | 5 | 182 | 4 | 2 | 6 | 0 | 57 | 2 | 113 | 0 | 10 |
| convex-helpers | quod-proof | 0 | 1d7c57881 | [42103735766](https://github.com/pango07/ci-bench/actions/runs/42103735766) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 180 | 2 | 178 | 3 | 3 | 5 | 0 | 58 | 3 | 103 | 5 | 14 |
| convex-helpers | quod-proof | 1 | 290b146e0 | [42104245752](https://github.com/pango07/ci-bench/actions/runs/42104245752) | ubuntu-26.04/20261025.179.1 | success | miss | 0 | 72 | 3 | 69 | 2 | 3 | 5 | 3 | 0 | 2 | 50 | 2 | 17 |
| convex-helpers | quod-proof | 2 | 1815d4a72 | [42104518479](https://github.com/pango07/ci-bench/actions/runs/42104518479) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 62 | 2 | 60 | 1 | 2 | 5 | 3 | 13 | 2 | 26 | 4 | 19 |
| convex-helpers | quod-proof | 3 | 03ffe84ef | [42104710469](https://github.com/pango07/ci-bench/actions/runs/42104710469) | ubuntu-26.04/20261025.179.1 | success | miss | 0 | 43 | 2 | 41 | 1 | 1 | 5 | 2 | 0 | 3 | 26 | 1 | 12 |
| convex-helpers | quod-proof | 4 | 6b17e1514 | [42104903744](https://github.com/pango07/ci-bench/actions/runs/42104903744) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 143 | 2 | 141 | 3 | 2 | 7 | 3 | 12 | 14 | 94 | 4 | 21 |
| convex-helpers | quod-proof | 5 | 49a82e544 | [42105324214](https://github.com/pango07/ci-bench/actions/runs/42105324214) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 85 | 2 | 83 | 2 | 3 | 5 | 2 | 12 | 3 | 52 | 3 | 16 |
| convex-helpers | quod-proof | 6 | 51f7bf848 | [42105671277](https://github.com/pango07/ci-bench/actions/runs/42105671277) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 137 | 1 | 136 | 3 | 2 | 5 | 2 | 0 | 2 | 121 | 1 | 13 |
| convex-helpers | quod-proof | 7 | f5c2b9e9c | [42106094629](https://github.com/pango07/ci-bench/actions/runs/42106094629) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 121 | 2 | 119 | 2 | 3 | 6 | 3 | 0 | 3 | 99 | 2 | 17 |
| convex-helpers | quod-proof | 8 | d244cf22d | [42106561388](https://github.com/pango07/ci-bench/actions/runs/42106561388) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 32 | 2 | 30 | 1 | 6 | 8 | 3 | 0 | 3 | 5 | 2 | 22 |
| convex-helpers | quod-proof | 9 | c976cabdf | [42106674625](https://github.com/pango07/ci-bench/actions/runs/42106674625) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 24 | 2 | 22 | 1 | 3 | 5 | 3 | 0 | 3 | 3 | 2 | 16 |
| convex-helpers | quod-proof | 10 | 6f830b66f | [42106784329](https://github.com/pango07/ci-bench/actions/runs/42106784329) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 52 | 3 | 49 | 1 | 2 | 6 | 8 | 15 | 3 | 6 | 5 | 25 |
| convex-helpers | quod-proof | 11 | 5eaa7e516 | [42106972069](https://github.com/pango07/ci-bench/actions/runs/42106972069) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 42 | 2 | 40 | 1 | 1 | 5 | 2 | 15 | 3 | 6 | 5 | 16 |
| convex-helpers | quod-ts-proof | 0 | 1d7c57881 | [42103735376](https://github.com/pango07/ci-bench/actions/runs/42103735376) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 170 | 2 | 168 | 3 | 3 | 5 | 0 | 59 | 3 | 91 | 4 | 15 |
| convex-helpers | quod-ts-proof | 1 | 290b146e0 | [42104244554](https://github.com/pango07/ci-bench/actions/runs/42104244554) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 125 | 2 | 123 | 3 | 2 | 5 | 4 | 0 | 3 | 102 | 2 | 18 |
| convex-helpers | quod-ts-proof | 2 | 1815d4a72 | [42104596889](https://github.com/pango07/ci-bench/actions/runs/42104596889) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 70 | 2 | 68 | 2 | 4 | 6 | 4 | 16 | 3 | 27 | 4 | 22 |
| convex-helpers | quod-ts-proof | 3 | 03ffe84ef | [42104869438](https://github.com/pango07/ci-bench/actions/runs/42104869438) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 48 | 2 | 46 | 1 | 2 | 6 | 4 | 0 | 2 | 27 | 2 | 17 |
| convex-helpers | quod-ts-proof | 4 | 6b17e1514 | [42105060664](https://github.com/pango07/ci-bench/actions/runs/42105060664) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 174 | 2 | 172 | 3 | 3 | 6 | 3 | 15 | 4 | 133 | 5 | 20 |
| convex-helpers | quod-ts-proof | 5 | 49a82e544 | [42105554142](https://github.com/pango07/ci-bench/actions/runs/42105554142) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 142 | 2 | 140 | 3 | 2 | 5 | 2 | 16 | 4 | 104 | 4 | 16 |
| convex-helpers | quod-ts-proof | 6 | 51f7bf848 | [42105973367](https://github.com/pango07/ci-bench/actions/runs/42105973367) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 140 | 1 | 139 | 3 | 2 | 5 | 2 | 0 | 3 | 122 | 1 | 14 |
| convex-helpers | quod-ts-proof | 7 | f5c2b9e9c | [42106403270](https://github.com/pango07/ci-bench/actions/runs/42106403270) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 47 | 13 | 34 | 1 | 4 | 9 | 3 | 0 | 2 | 12 | 1 | 20 |
| convex-helpers | quod-ts-proof | 8 | d244cf22d | [42106599569](https://github.com/pango07/ci-bench/actions/runs/42106599569) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 40 | 2 | 38 | 1 | 4 | 14 | 4 | 0 | 3 | 8 | 1 | 27 |
| convex-helpers | quod-ts-proof | 9 | c976cabdf | [42106786048](https://github.com/pango07/ci-bench/actions/runs/42106786048) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 22 | 2 | 20 | 1 | 1 | 5 | 2 | 0 | 4 | 4 | 1 | 12 |
| convex-helpers | quod-ts-proof | 10 | 6f830b66f | [42106897925](https://github.com/pango07/ci-bench/actions/runs/42106897925) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 45 | 2 | 43 | 1 | 1 | 6 | 2 | 19 | 4 | 5 | 4 | 15 |
| convex-helpers | quod-ts-proof | 11 | 5eaa7e516 | [42107084609](https://github.com/pango07/ci-bench/actions/runs/42107084609) | ubuntu-26.04/20261025.179.1 | success | miss | 0 | 73 | 2 | 71 | 2 | 2 | 6 | 2 | 15 | 3 | 35 | 5 | 18 |

</details>

