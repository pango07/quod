# Convex-helpers: measured Addendum E window

Run: [42087297690](https://github.com/pango07/ci-bench/actions/runs/42087297690), 27 minutes 19 seconds.

Immutable measured versions: Quod binary `a026fea` (source gate/report harness `b01a116`), action `98ebac2bb98158c0026bcc591645e9dec48062de`, ci-bench `1aec15fe48a07e7309747d37cab5a7ea5816d2bc`. The later harness commit `615262c` prevents failed downloads from retaining an older job's log; it was not used by the running driver. All 48 job IDs, timestamps, images, signatures, and per-test fingerprints were independently downloaded and checked against the driver artifact.

## Verdict

| scope | plain s / rounded min | default Quod | timing replay |
|---|---:|---:|---:|
| c0–c11 | 1905 / 36 | 1292 / 28 (0.678× s; 0.778× min) | 1173 / 25 (0.616× s; 0.694× min) |
| c1–c11 | 1772 / 33 | 1157 / 25 (0.653× s; 0.758× min) | 1015 / 22 (0.573× s; 0.667× min) |

The 0.50× whole-window cost target is **not met**. Timing replay needs at least 221 more runner-seconds removed from this measured window. Rounded minutes are the sum of per-job ceiling(seconds / 60); the public runs incur no runner charge. Cold jobs are included.

Wrong = 0 at every c0–c11 for both variants: all 24 Quod jobs matched their corresponding plain job's individual test names/states, normalized file/count signatures, and conclusion. All expected comparisons were present. This is observed agreement, not a proof that the recorder covers every possible program behavior; the stat-metadata limitation from the audit remains open.

Lineage promoted 224 records in default Quod and 223 in timing replay. **Zero lineage audits ran in the window, and zero audit disagreements were observed.** Subsequent shared-input changes replaced those promoted records before an eligible replay/audit. The packaged-action regression independently exercised one agreeing plain audit; unit regressions exercised batch revocation on disagreement. E1's production default remains off.

There were no full hits within the changing-commit window. Timing replay's fastest partial jobs were 23 s; the c8 job was floor + 8 s. Those are not classified as full hits.

## Separate unchanged-commit hit probes

After the final timing-replay job, one 93 s confirmation job established the remaining witnesses. Three subsequent jobs replayed every test file and unit. Each matched plain c11's individual test states. These additional measurements are **excluded from the 12-commit cost window**. Best hit: **floor + 4 s**; median matched-floor overhead: **floor + 6 s**. Absolute best/median hit: 19 / 20 s. All three probes and their floors used `ubuntu-26.04/20261101.186.1`.

| probe | run | runner-s | matched floor-s | above floor-s |
|---|---|---:|---:|---:|
| confirmation | [42091438656](https://github.com/pango07/ci-bench/actions/runs/42091438656) | 93 | — | — |
| hit-0 | [42091736671](https://github.com/pango07/ci-bench/actions/runs/42091736671) | 20 | [14](https://github.com/pango07/ci-bench/actions/runs/42091728373) | 6 |
| hit-1 | [42091854522](https://github.com/pango07/ci-bench/actions/runs/42091854522) | 19 | [15](https://github.com/pango07/ci-bench/actions/runs/42091848489) | 4 |
| hit-2 | [42091972608](https://github.com/pango07/ci-bench/actions/runs/42091972608) | 22 | [13](https://github.com/pango07/ci-bench/actions/runs/42091966630) | 9 |

## Remaining cost

The default policy repeatedly executes `stream.property.test.ts` because one case approaches its timeout. Timing replay removes that repeated cost after plain agreement, but remains opt-in. Whole-suite invalidations at package-structure and dependency changes then dominate both policies. The config's `Object.keys({...dependencies, ...devDependencies})` copies version values before discarding them, so the observer conservatively keys those reads. Ignoring all such reads would reintroduce the audited correctness bug.

Timing replay spent 788 s in test steps, 131 s installing, 36 s building, and 83 s in action/post steps. Even eliminating action/post entirely cannot close its 221 s gap. E2 cross-record agreement worked in real jobs, including image changes; changed npm version banners correctly refused agreement.

The next investigation is a sound configuration/transform boundary: rerun changed producers, retain downstream execution only when the full observable output is validated unchanged. A serialized config hash alone is insufficient because Vitest strips functions; plugin closures, transformed code, setup, provided context and side effects require coverage or fallback. A measurement-only config probe is an appropriate first step.

## Driver report and every job's runner image

### Real jobs: where a job's runner-seconds go (jobs API, seconds)

Medians per project and variant. **fixed** = runner-seconds minus install, build and test steps. **boot+setup** = "Set up job" + checkout + package-manager/node set-up (the kill line is 40 s). Queue is not billed and not in runner.

| project | variant | jobs | wall | queue | runner | boot | setup | action | install | build | test | post | **fixed** | **boot+setup** | corpus wall-s | corpus runner-s |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| convex-helpers | floor | 12 | 16 | 2 | 13 | 3 | 6 | 0 | 0 | 0 | 0 | 0 | **13** | **9** | 193 | 149 |
| convex-helpers | plain | 12 | 174 | 2 | 172 | 3 | 6 | 0 | 58 | 2 | 99 | 1 | **12** | **8** | 1934 | 1905 |
| convex-helpers | quod-ts | 12 | 106 | 2 | 105 | 2 | 6 | 3 | 6 | 3 | 72 | 3 | **18** | **8** | 1195 | 1173 |
| convex-helpers | quod | 12 | 117 | 2 | 115 | 2 | 5 | 3 | 6 | 3 | 80 | 3 | **17** | **7** | 1316 | 1292 |
| **all** | floor | 12 | 16 | 2 | 13 | 3 | 6 | 0 | 0 | 0 | 0 | 0 | **13** | **9** | | |
| **all** | plain | 12 | 174 | 2 | 172 | 3 | 6 | 0 | 58 | 2 | 99 | 1 | **12** | **8** | | |
| **all** | quod-ts | 12 | 106 | 2 | 105 | 2 | 6 | 3 | 6 | 3 | 72 | 3 | **18** | **8** | | |
| **all** | quod | 12 | 117 | 2 | 115 | 2 | 5 | 3 | 6 | 3 | 80 | 3 | **17** | **7** | | |

| project | variant | jobs | median wall | runner-s | x plain | billed min | x plain | wrong | unchecked |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|
| convex-helpers | floor | 12 | 16 | 149 | 0.08 | 12 | 0.33 | 0 | 0 |
| convex-helpers | plain | 12 | 174 | 1905 | 1.00 | 36 | 1.00 |  |  |
| convex-helpers | quod-ts | 12 | 106 | 1173 | 0.62 | 25 | 0.69 | 0 | 0 |
| convex-helpers | quod | 12 | 117 | 1292 | 0.68 | 28 | 0.78 | 0 | 0 |

| variant | c1–c11 runner-s | x plain | billed min | x plain | best hit above floor | median hit above floor | lineage audits | disagreed |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| quod-ts | 1015 | 0.57 | 22 | 0.67 | no full hits | no full hits | 0 | 0 |
| quod | 1157 | 0.65 | 25 | 0.76 | no full hits | no full hits | 0 | 0 |

<details><summary>per job</summary>

| project | variant | # | commit | run | image | result | Quod | wrong | wall | queue | runner | billed | boot | setup | action | install | build | test | post | fixed |
|---|---|---:|---|---|---|---|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| convex-helpers | floor | 0 | 1d7c57881 | [42087325752](https://github.com/pango07/ci-bench/actions/runs/42087325752) | ubuntu-26.04/20261101.186.1 | success |  | ? | 13 | 1 | 12 | 1 | 3 | 5 | 0 | 0 | 0 | 0 | 0 | 12 |
| convex-helpers | floor | 1 | 290b146e0 | [42087330282](https://github.com/pango07/ci-bench/actions/runs/42087330282) | ubuntu-26.04/20261101.186.1 | success |  | ? | 15 | 2 | 13 | 1 | 3 | 6 | 0 | 0 | 0 | 0 | 0 | 13 |
| convex-helpers | floor | 2 | 1815d4a72 | [42087335040](https://github.com/pango07/ci-bench/actions/runs/42087335040) | ubuntu-26.04/20261101.186.1 | success |  | ? | 15 | 2 | 13 | 1 | 3 | 6 | 0 | 0 | 0 | 0 | 0 | 13 |
| convex-helpers | floor | 3 | 03ffe84ef | [42087338774](https://github.com/pango07/ci-bench/actions/runs/42087338774) | ubuntu-26.04/20261025.179.1 | success |  | ? | 15 | 2 | 13 | 1 | 3 | 6 | 0 | 0 | 0 | 0 | 1 | 13 |
| convex-helpers | floor | 4 | 6b17e1514 | [42087343055](https://github.com/pango07/ci-bench/actions/runs/42087343055) | ubuntu-26.04/20261101.186.1 | success |  | ? | 13 | 2 | 11 | 1 | 2 | 6 | 0 | 0 | 0 | 0 | 0 | 11 |
| convex-helpers | floor | 5 | 49a82e544 | [42087346903](https://github.com/pango07/ci-bench/actions/runs/42087346903) | ubuntu-26.04/20261101.186.1 | success |  | ? | 17 | 3 | 14 | 1 | 3 | 5 | 0 | 0 | 0 | 0 | 1 | 14 |
| convex-helpers | floor | 6 | 51f7bf848 | [42087350987](https://github.com/pango07/ci-bench/actions/runs/42087350987) | ubuntu-26.04/20261101.186.1 | success |  | ? | 20 | 6 | 14 | 1 | 4 | 6 | 0 | 0 | 0 | 0 | 0 | 14 |
| convex-helpers | floor | 7 | f5c2b9e9c | [42087357055](https://github.com/pango07/ci-bench/actions/runs/42087357055) | ubuntu-26.04/20261025.179.1 | success |  | ? | 14 | 2 | 12 | 1 | 2 | 7 | 0 | 0 | 0 | 0 | 1 | 12 |
| convex-helpers | floor | 8 | d244cf22d | [42087361182](https://github.com/pango07/ci-bench/actions/runs/42087361182) | ubuntu-26.04/20261101.186.1 | success |  | ? | 17 | 2 | 15 | 1 | 4 | 7 | 0 | 0 | 0 | 0 | 0 | 15 |
| convex-helpers | floor | 9 | c976cabdf | [42087365031](https://github.com/pango07/ci-bench/actions/runs/42087365031) | ubuntu-26.04/20261101.186.1 | success |  | ? | 16 | 6 | 10 | 1 | 1 | 5 | 0 | 0 | 0 | 0 | 1 | 10 |
| convex-helpers | floor | 10 | 6f830b66f | [42087369796](https://github.com/pango07/ci-bench/actions/runs/42087369796) | ubuntu-26.04/20261101.186.1 | success |  | ? | 16 | 6 | 10 | 1 | 1 | 5 | 0 | 0 | 0 | 0 | 1 | 10 |
| convex-helpers | floor | 11 | 5eaa7e516 | [42087374309](https://github.com/pango07/ci-bench/actions/runs/42087374309) | ubuntu-26.04/20261025.179.1 | success |  | ? | 22 | 10 | 12 | 1 | 3 | 5 | 0 | 0 | 0 | 0 | 0 | 12 |
| convex-helpers | plain | 0 | 1d7c57881 | [42087322333](https://github.com/pango07/ci-bench/actions/runs/42087322333) | ubuntu-26.04/20261101.186.1 | success |  | 122 lines | 135 | 2 | 133 | 3 | 4 | 7 | 0 | 48 | 1 | 69 | 0 | 15 |
| convex-helpers | plain | 1 | 290b146e0 | [42087327415](https://github.com/pango07/ci-bench/actions/runs/42087327415) | ubuntu-26.04/20261025.179.1 | success |  | 122 lines | 178 | 2 | 176 | 3 | 4 | 7 | 0 | 66 | 2 | 92 | 1 | 16 |
| convex-helpers | plain | 2 | 1815d4a72 | [42087331860](https://github.com/pango07/ci-bench/actions/runs/42087331860) | ubuntu-26.04/20261025.179.1 | success |  | 128 lines | 175 | 2 | 173 | 3 | 2 | 5 | 0 | 60 | 2 | 101 | 0 | 10 |
| convex-helpers | plain | 3 | 03ffe84ef | [42087336390](https://github.com/pango07/ci-bench/actions/runs/42087336390) | ubuntu-26.04/20261101.186.1 | success |  | 128 lines | 173 | 2 | 171 | 3 | 2 | 6 | 0 | 59 | 2 | 99 | 0 | 11 |
| convex-helpers | plain | 4 | 6b17e1514 | [42087340967](https://github.com/pango07/ci-bench/actions/runs/42087340967) | ubuntu-26.04/20261025.179.1 | success |  | 128 lines | 178 | 2 | 176 | 3 | 3 | 5 | 0 | 59 | 2 | 103 | 0 | 12 |
| convex-helpers | plain | 5 | 49a82e544 | [42087345600](https://github.com/pango07/ci-bench/actions/runs/42087345600) | ubuntu-26.04/20261025.179.1 | success |  | 128 lines | 115 | 2 | 113 | 2 | 3 | 5 | 0 | 39 | 1 | 63 | 0 | 10 |
| convex-helpers | plain | 6 | 51f7bf848 | [42087349966](https://github.com/pango07/ci-bench/actions/runs/42087349966) | ubuntu-26.04/20261101.186.1 | success |  | 128 lines | 181 | 1 | 180 | 3 | 1 | 5 | 0 | 57 | 2 | 111 | 1 | 10 |
| convex-helpers | plain | 7 | f5c2b9e9c | [42087355211](https://github.com/pango07/ci-bench/actions/runs/42087355211) | ubuntu-26.04/20261101.186.1 | success |  | 128 lines | 173 | 2 | 171 | 3 | 2 | 5 | 0 | 59 | 2 | 99 | 0 | 11 |
| convex-helpers | plain | 8 | d244cf22d | [42087360004](https://github.com/pango07/ci-bench/actions/runs/42087360004) | ubuntu-26.04/20261101.186.1 | success |  | 128 lines | 107 | 2 | 105 | 2 | 2 | 5 | 0 | 36 | 2 | 56 | 1 | 11 |
| convex-helpers | plain | 9 | c976cabdf | [42087364810](https://github.com/pango07/ci-bench/actions/runs/42087364810) | ubuntu-26.04/20261025.179.1 | success |  | 128 lines | 146 | 2 | 144 | 3 | 4 | 8 | 0 | 47 | 1 | 79 | 1 | 17 |
| convex-helpers | plain | 10 | 6f830b66f | [42087370796](https://github.com/pango07/ci-bench/actions/runs/42087370796) | ubuntu-26.04/20261101.186.1 | success |  | 128 lines | 184 | 3 | 181 | 4 | 3 | 6 | 0 | 56 | 2 | 109 | 1 | 14 |
| convex-helpers | plain | 11 | 5eaa7e516 | [42087376102](https://github.com/pango07/ci-bench/actions/runs/42087376102) | ubuntu-26.04/20261101.186.1 | success |  | 128 lines | 189 | 7 | 182 | 4 | 2 | 6 | 0 | 58 | 1 | 111 | 1 | 12 |
| convex-helpers | quod | 0 | 1d7c57881 | [42087325294](https://github.com/pango07/ci-bench/actions/runs/42087325294) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 137 | 2 | 135 | 3 | 3 | 7 | 1 | 47 | 2 | 69 | 3 | 17 |
| convex-helpers | quod | 1 | 290b146e0 | [42087766718](https://github.com/pango07/ci-bench/actions/runs/42087766718) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 69 | 1 | 68 | 2 | 3 | 5 | 2 | 0 | 2 | 51 | 2 | 15 |
| convex-helpers | quod | 2 | 1815d4a72 | [42087966807](https://github.com/pango07/ci-bench/actions/runs/42087966807) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 73 | 2 | 71 | 2 | 2 | 5 | 5 | 15 | 3 | 34 | 4 | 19 |
| convex-helpers | quod | 3 | 03ffe84ef | [42088243795](https://github.com/pango07/ci-bench/actions/runs/42088243795) | ubuntu-26.04/20261025.179.1 | success | miss | 0 | 57 | 2 | 55 | 1 | 2 | 7 | 4 | 0 | 3 | 34 | 1 | 18 |
| convex-helpers | quod | 4 | 6b17e1514 | [42088441064](https://github.com/pango07/ci-bench/actions/runs/42088441064) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 134 | 2 | 132 | 3 | 2 | 5 | 8 | 12 | 3 | 94 | 4 | 23 |
| convex-helpers | quod | 5 | 49a82e544 | [42088880875](https://github.com/pango07/ci-bench/actions/runs/42088880875) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 108 | 2 | 106 | 2 | 2 | 5 | 3 | 13 | 3 | 72 | 4 | 18 |
| convex-helpers | quod | 6 | 51f7bf848 | [42089236379](https://github.com/pango07/ci-bench/actions/runs/42089236379) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 157 | 4 | 153 | 3 | 3 | 6 | 4 | 0 | 3 | 132 | 2 | 18 |
| convex-helpers | quod | 7 | f5c2b9e9c | [42089669816](https://github.com/pango07/ci-bench/actions/runs/42089669816) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 122 | 1 | 121 | 3 | 3 | 5 | 3 | 0 | 4 | 101 | 2 | 16 |
| convex-helpers | quod | 8 | d244cf22d | [42090042829](https://github.com/pango07/ci-bench/actions/runs/42090042829) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 111 | 2 | 109 | 2 | 2 | 5 | 4 | 0 | 4 | 88 | 2 | 17 |
| convex-helpers | quod | 9 | c976cabdf | [42090406948](https://github.com/pango07/ci-bench/actions/runs/42090406948) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 57 | 2 | 55 | 1 | 2 | 5 | 2 | 0 | 2 | 41 | 1 | 12 |
| convex-helpers | quod | 10 | 6f830b66f | [42090612861](https://github.com/pango07/ci-bench/actions/runs/42090612861) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 144 | 2 | 142 | 3 | 2 | 5 | 2 | 20 | 3 | 103 | 6 | 16 |
| convex-helpers | quod | 11 | 5eaa7e516 | [42091057503](https://github.com/pango07/ci-bench/actions/runs/42091057503) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 147 | 2 | 145 | 3 | 2 | 5 | 2 | 19 | 3 | 106 | 5 | 17 |
| convex-helpers | quod-ts | 0 | 1d7c57881 | [42087325238](https://github.com/pango07/ci-bench/actions/runs/42087325238) | ubuntu-26.04/20261025.179.1 | success | miss | 0 | 159 | 1 | 158 | 3 | 3 | 6 | 1 | 56 | 3 | 81 | 5 | 18 |
| convex-helpers | quod-ts | 1 | 290b146e0 | [42087767339](https://github.com/pango07/ci-bench/actions/runs/42087767339) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 126 | 2 | 124 | 3 | 3 | 6 | 3 | 0 | 3 | 103 | 2 | 18 |
| convex-helpers | quod-ts | 2 | 1815d4a72 | [42088132130](https://github.com/pango07/ci-bench/actions/runs/42088132130) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 91 | 3 | 88 | 2 | 2 | 6 | 4 | 19 | 3 | 34 | 17 | 32 |
| convex-helpers | quod-ts | 3 | 03ffe84ef | [42088410656](https://github.com/pango07/ci-bench/actions/runs/42088410656) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 56 | 2 | 54 | 1 | 2 | 6 | 4 | 0 | 3 | 34 | 2 | 17 |
| convex-helpers | quod-ts | 4 | 6b17e1514 | [42088607931](https://github.com/pango07/ci-bench/actions/runs/42088607931) | ubuntu-26.04/20261025.179.1 | success | miss | 0 | 105 | 1 | 104 | 2 | 2 | 6 | 2 | 13 | 2 | 72 | 3 | 17 |
| convex-helpers | quod-ts | 5 | 49a82e544 | [42088964699](https://github.com/pango07/ci-bench/actions/runs/42088964699) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 169 | 2 | 167 | 3 | 1 | 6 | 2 | 15 | 4 | 133 | 4 | 15 |
| convex-helpers | quod-ts | 6 | 51f7bf848 | [42089475762](https://github.com/pango07/ci-bench/actions/runs/42089475762) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 143 | 2 | 141 | 3 | 3 | 5 | 4 | 0 | 3 | 120 | 2 | 18 |
| convex-helpers | quod-ts | 7 | f5c2b9e9c | [42089931552](https://github.com/pango07/ci-bench/actions/runs/42089931552) | ubuntu-26.04/20261025.179.1 | success | miss | 0 | 51 | 2 | 49 | 1 | 3 | 6 | 3 | 0 | 3 | 28 | 2 | 18 |
| convex-helpers | quod-ts | 8 | d244cf22d | [42090126446](https://github.com/pango07/ci-bench/actions/runs/42090126446) | ubuntu-26.04/20261025.179.1 | success | miss | 0 | 25 | 2 | 23 | 1 | 2 | 5 | 3 | 0 | 3 | 6 | 1 | 14 |
| convex-helpers | quod-ts | 9 | c976cabdf | [42090244629](https://github.com/pango07/ci-bench/actions/runs/42090244629) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 24 | 1 | 23 | 1 | 1 | 6 | 3 | 0 | 3 | 6 | 1 | 14 |
| convex-helpers | quod-ts | 10 | 6f830b66f | [42090363164](https://github.com/pango07/ci-bench/actions/runs/42090363164) | ubuntu-26.04/20261025.179.1 | success | miss | 0 | 107 | 2 | 105 | 2 | 2 | 6 | 3 | 11 | 3 | 71 | 5 | 20 |
| convex-helpers | quod-ts | 11 | 5eaa7e516 | [42090733125](https://github.com/pango07/ci-bench/actions/runs/42090733125) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 139 | 2 | 137 | 3 | 3 | 4 | 2 | 17 | 3 | 100 | 5 | 17 |

</details>

