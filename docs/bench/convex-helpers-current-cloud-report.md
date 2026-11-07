### Real jobs: where a job's runner-seconds go (jobs API, seconds)

Medians per project and variant. **fixed** = runner-seconds minus install, build and test steps. **boot+setup** = "Set up job" + checkout + package-manager/node set-up (the kill line is 40 s). Queue is not billed and not in runner.

| project | variant | jobs | wall | queue | runner | boot | setup | action | install | build | test | post | **fixed** | **boot+setup** | corpus wall-s | corpus runner-s |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| convex-helpers | floor | 12 | 13 | 2 | 10 | 1 | 6 | 0 | 0 | 0 | 0 | 0 | **10** | **7** | 162 | 118 |
| convex-helpers | plain | 12 | 142 | 2 | 136 | 1 | 5 | 0 | 46 | 2 | 77 | 0 | **10** | **7** | 1771 | 1735 |
| convex-helpers | quod-ts | 12 | 128 | 2 | 126 | 2 | 6 | 3 | 6 | 2 | 93 | 2 | **16** | **8** | 1520 | 1497 |
| convex-helpers | quod | 12 | 130 | 2 | 128 | 2 | 6 | 3 | 5 | 2 | 99 | 3 | **16** | **8** | 1560 | 1538 |
| **all** | floor | 12 | 13 | 2 | 10 | 1 | 6 | 0 | 0 | 0 | 0 | 0 | **10** | **7** | | |
| **all** | plain | 12 | 142 | 2 | 136 | 1 | 5 | 0 | 46 | 2 | 77 | 0 | **10** | **7** | | |
| **all** | quod-ts | 12 | 128 | 2 | 126 | 2 | 6 | 3 | 6 | 2 | 93 | 2 | **16** | **8** | | |
| **all** | quod | 12 | 130 | 2 | 128 | 2 | 6 | 3 | 5 | 2 | 99 | 3 | **16** | **8** | | |

| project | variant | jobs | median wall | runner-s | x plain | billed min | x plain | wrong | unchecked |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|
| convex-helpers | floor | 12 | 13 | 118 | 0.07 | 12 | 0.33 | 0 | 0 |
| convex-helpers | plain | 12 | 142 | 1735 | 1.00 | 36 | 1.00 |  |  |
| convex-helpers | quod-ts | 12 | 128 | 1497 | 0.86 | 32 | 0.89 | 0 | 0 |
| convex-helpers | quod | 12 | 130 | 1538 | 0.89 | 32 | 0.89 | 0 | 0 |

| variant | after-first-job runner-s | x plain | billed min | x plain | best hit above floor | median hit above floor | lineage audits | disagreed |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| quod-ts | 1311 | 0.81 | 28 | 0.85 | no full hits | no full hits | 0 | 0 |
| quod | 1353 | 0.84 | 28 | 0.85 | no full hits | no full hits | 0 | 0 |

<details><summary>per job</summary>

| project | variant | # | commit | run | image | result | Quod | wrong | wall | queue | runner | billed | boot | setup | action | install | build | test | post | fixed |
|---|---|---:|---|---|---|---|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| convex-helpers | floor | 0 | 1d7c57881 | [42349940051](https://github.com/pango07/ci-bench/actions/runs/42349940051) | ubuntu-26.04/20261101.186.1 | success |  | ? | 10 | 2 | 8 | 1 | 1 | 5 | 0 | 0 | 0 | 0 | 1 | 8 |
| convex-helpers | floor | 1 | 290b146e0 | [42349941714](https://github.com/pango07/ci-bench/actions/runs/42349941714) | ubuntu-26.04/20261101.186.1 | success |  | ? | 12 | 1 | 11 | 1 | 1 | 6 | 0 | 0 | 0 | 0 | 0 | 11 |
| convex-helpers | floor | 2 | 1815d4a72 | [42349943199](https://github.com/pango07/ci-bench/actions/runs/42349943199) | ubuntu-26.04/20261101.186.1 | success |  | ? | 13 | 2 | 11 | 1 | 1 | 6 | 0 | 0 | 0 | 0 | 0 | 11 |
| convex-helpers | floor | 3 | 03ffe84ef | [42349944567](https://github.com/pango07/ci-bench/actions/runs/42349944567) | ubuntu-26.04/20261101.186.1 | success |  | ? | 14 | 2 | 12 | 1 | 2 | 6 | 0 | 0 | 0 | 0 | 0 | 12 |
| convex-helpers | floor | 4 | 6b17e1514 | [42349946246](https://github.com/pango07/ci-bench/actions/runs/42349946246) | ubuntu-26.04/20261101.186.1 | success |  | ? | 10 | 1 | 9 | 1 | 1 | 5 | 0 | 0 | 0 | 0 | 0 | 9 |
| convex-helpers | floor | 5 | 49a82e544 | [42349947819](https://github.com/pango07/ci-bench/actions/runs/42349947819) | ubuntu-26.04/20261101.186.1 | success |  | ? | 11 | 2 | 9 | 1 | 1 | 6 | 0 | 0 | 0 | 0 | 0 | 9 |
| convex-helpers | floor | 6 | 51f7bf848 | [42349949394](https://github.com/pango07/ci-bench/actions/runs/42349949394) | ubuntu-26.04/20261101.186.1 | success |  | ? | 10 | 2 | 8 | 1 | 1 | 5 | 0 | 0 | 0 | 0 | 0 | 8 |
| convex-helpers | floor | 7 | f5c2b9e9c | [42349951272](https://github.com/pango07/ci-bench/actions/runs/42349951272) | ubuntu-26.04/20261101.186.1 | success |  | ? | 10 | 2 | 8 | 1 | 1 | 5 | 0 | 0 | 0 | 0 | 1 | 8 |
| convex-helpers | floor | 8 | d244cf22d | [42349953456](https://github.com/pango07/ci-bench/actions/runs/42349953456) | ubuntu-26.04/20261101.186.1 | success |  | ? | 17 | 5 | 12 | 1 | 2 | 6 | 0 | 0 | 0 | 0 | 0 | 12 |
| convex-helpers | floor | 9 | c976cabdf | [42349955043](https://github.com/pango07/ci-bench/actions/runs/42349955043) | ubuntu-26.04/20261101.186.1 | success |  | ? | 15 | 6 | 9 | 1 | 1 | 5 | 0 | 0 | 0 | 0 | 0 | 9 |
| convex-helpers | floor | 10 | 6f830b66f | [42349956864](https://github.com/pango07/ci-bench/actions/runs/42349956864) | ubuntu-26.04/20261101.186.1 | success |  | ? | 17 | 6 | 11 | 1 | 1 | 7 | 0 | 0 | 0 | 0 | 0 | 11 |
| convex-helpers | floor | 11 | 5eaa7e516 | [42349958581](https://github.com/pango07/ci-bench/actions/runs/42349958581) | ubuntu-26.04/20261101.186.1 | success |  | ? | 23 | 13 | 10 | 1 | 1 | 6 | 0 | 0 | 0 | 0 | 1 | 10 |
| convex-helpers | plain | 0 | 1d7c57881 | [42349939209](https://github.com/pango07/ci-bench/actions/runs/42349939209) | ubuntu-26.04/20261101.186.1 | success |  | 122 lines | 128 | 2 | 126 | 3 | 1 | 6 | 0 | 46 | 2 | 68 | 1 | 10 |
| convex-helpers | plain | 1 | 290b146e0 | [42349940877](https://github.com/pango07/ci-bench/actions/runs/42349940877) | ubuntu-26.04/20261101.186.1 | success |  | 122 lines | 149 | 1 | 148 | 3 | 1 | 6 | 0 | 56 | 2 | 79 | 1 | 11 |
| convex-helpers | plain | 2 | 1815d4a72 | [42349942567](https://github.com/pango07/ci-bench/actions/runs/42349942567) | ubuntu-26.04/20261101.186.1 | success |  | 128 lines | 184 | 2 | 182 | 4 | 2 | 5 | 0 | 57 | 2 | 111 | 1 | 12 |
| convex-helpers | plain | 3 | 03ffe84ef | [42349944084](https://github.com/pango07/ci-bench/actions/runs/42349944084) | ubuntu-26.04/20261101.186.1 | success |  | 128 lines | 181 | 1 | 180 | 3 | 1 | 6 | 0 | 57 | 2 | 111 | 0 | 10 |
| convex-helpers | plain | 4 | 6b17e1514 | [42349945449](https://github.com/pango07/ci-bench/actions/runs/42349945449) | ubuntu-26.04/20261101.186.1 | success |  | 128 lines | 157 | 2 | 155 | 3 | 2 | 6 | 0 | 55 | 2 | 88 | 0 | 10 |
| convex-helpers | plain | 5 | 49a82e544 | [42349947033](https://github.com/pango07/ci-bench/actions/runs/42349947033) | ubuntu-26.04/20261101.186.1 | success |  | 128 lines | 99 | 2 | 97 | 2 | 1 | 5 | 0 | 34 | 1 | 54 | 0 | 8 |
| convex-helpers | plain | 6 | 51f7bf848 | [42349948850](https://github.com/pango07/ci-bench/actions/runs/42349948850) | ubuntu-26.04/20261101.186.1 | success |  | 128 lines | 179 | 1 | 178 | 3 | 1 | 4 | 0 | 57 | 2 | 111 | 0 | 8 |
| convex-helpers | plain | 7 | f5c2b9e9c | [42349950419](https://github.com/pango07/ci-bench/actions/runs/42349950419) | ubuntu-26.04/20261101.186.1 | success |  | 128 lines | 136 | 2 | 134 | 3 | 1 | 5 | 0 | 46 | 1 | 76 | 1 | 11 |
| convex-helpers | plain | 8 | d244cf22d | [42349952210](https://github.com/pango07/ci-bench/actions/runs/42349952210) | ubuntu-26.04/20261101.186.1 | success |  | 128 lines | 140 | 5 | 135 | 3 | 2 | 6 | 0 | 46 | 2 | 75 | 0 | 12 |
| convex-helpers | plain | 9 | c976cabdf | [42349953902](https://github.com/pango07/ci-bench/actions/runs/42349953902) | ubuntu-26.04/20261101.186.1 | success |  | 128 lines | 136 | 5 | 131 | 3 | 0 | 5 | 0 | 45 | 2 | 76 | 0 | 8 |
| convex-helpers | plain | 10 | 6f830b66f | [42349955749](https://github.com/pango07/ci-bench/actions/runs/42349955749) | ubuntu-26.04/20261101.186.1 | success |  | 128 lines | 139 | 6 | 133 | 3 | 1 | 4 | 0 | 46 | 2 | 77 | 0 | 8 |
| convex-helpers | plain | 11 | 5eaa7e516 | [42349957466](https://github.com/pango07/ci-bench/actions/runs/42349957466) | ubuntu-26.04/20261101.186.1 | success |  | 128 lines | 143 | 7 | 136 | 3 | 1 | 5 | 0 | 46 | 2 | 77 | 1 | 11 |
| convex-helpers | quod | 0 | 1d7c57881 | [42349940099](https://github.com/pango07/ci-bench/actions/runs/42349940099) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 187 | 2 | 185 | 4 | 2 | 5 | 1 | 57 | 3 | 111 | 4 | 14 |
| convex-helpers | quod | 1 | 290b146e0 | [42350172286](https://github.com/pango07/ci-bench/actions/runs/42350172286) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 103 | 2 | 101 | 2 | 2 | 6 | 2 | 0 | 2 | 84 | 1 | 15 |
| convex-helpers | quod | 2 | 1815d4a72 | [42350333564](https://github.com/pango07/ci-bench/actions/runs/42350333564) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 105 | 1 | 104 | 2 | 2 | 5 | 3 | 9 | 1 | 78 | 3 | 16 |
| convex-helpers | quod | 3 | 03ffe84ef | [42350502234](https://github.com/pango07/ci-bench/actions/runs/42350502234) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 140 | 1 | 139 | 3 | 1 | 6 | 3 | 0 | 2 | 123 | 1 | 14 |
| convex-helpers | quod | 4 | 6b17e1514 | [42350694432](https://github.com/pango07/ci-bench/actions/runs/42350694432) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 124 | 1 | 123 | 3 | 2 | 6 | 3 | 12 | 2 | 90 | 4 | 19 |
| convex-helpers | quod | 5 | 49a82e544 | [42350853758](https://github.com/pango07/ci-bench/actions/runs/42350853758) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 143 | 2 | 141 | 3 | 1 | 4 | 2 | 14 | 2 | 112 | 3 | 13 |
| convex-helpers | quod | 6 | 51f7bf848 | [42351051709](https://github.com/pango07/ci-bench/actions/runs/42351051709) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 140 | 2 | 138 | 3 | 1 | 7 | 3 | 0 | 2 | 121 | 1 | 15 |
| convex-helpers | quod | 7 | f5c2b9e9c | [42351238994](https://github.com/pango07/ci-bench/actions/runs/42351238994) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 110 | 2 | 108 | 2 | 3 | 5 | 3 | 0 | 2 | 90 | 2 | 16 |
| convex-helpers | quod | 8 | d244cf22d | [42351388078](https://github.com/pango07/ci-bench/actions/runs/42351388078) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 113 | 3 | 110 | 2 | 2 | 6 | 3 | 0 | 2 | 92 | 2 | 16 |
| convex-helpers | quod | 9 | c976cabdf | [42351536161](https://github.com/pango07/ci-bench/actions/runs/42351536161) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 135 | 2 | 133 | 3 | 3 | 7 | 2 | 0 | 2 | 106 | 1 | 25 |
| convex-helpers | quod | 10 | 6f830b66f | [42351736606](https://github.com/pango07/ci-bench/actions/runs/42351736606) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 96 | 2 | 94 | 2 | 2 | 7 | 2 | 9 | 1 | 66 | 4 | 18 |
| convex-helpers | quod | 11 | 5eaa7e516 | [42351857182](https://github.com/pango07/ci-bench/actions/runs/42351857182) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 164 | 2 | 162 | 3 | 2 | 8 | 2 | 19 | 2 | 123 | 4 | 18 |
| convex-helpers | quod-ts | 0 | 1d7c57881 | [42349939963](https://github.com/pango07/ci-bench/actions/runs/42349939963) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 188 | 2 | 186 | 4 | 3 | 5 | 1 | 56 | 3 | 112 | 4 | 15 |
| convex-helpers | quod-ts | 1 | 290b146e0 | [42350172022](https://github.com/pango07/ci-bench/actions/runs/42350172022) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 129 | 2 | 127 | 3 | 2 | 6 | 3 | 0 | 2 | 110 | 1 | 15 |
| convex-helpers | quod-ts | 2 | 1815d4a72 | [42350332900](https://github.com/pango07/ci-bench/actions/runs/42350332900) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 160 | 2 | 158 | 3 | 3 | 6 | 2 | 17 | 2 | 121 | 4 | 18 |
| convex-helpers | quod-ts | 3 | 03ffe84ef | [42350537652](https://github.com/pango07/ci-bench/actions/runs/42350537652) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 111 | 1 | 110 | 2 | 3 | 6 | 3 | 0 | 2 | 90 | 1 | 18 |
| convex-helpers | quod-ts | 4 | 6b17e1514 | [42350694051](https://github.com/pango07/ci-bench/actions/runs/42350694051) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 127 | 2 | 125 | 3 | 2 | 8 | 2 | 13 | 2 | 92 | 3 | 18 |
| convex-helpers | quod-ts | 5 | 49a82e544 | [42350853639](https://github.com/pango07/ci-bench/actions/runs/42350853639) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 156 | 2 | 154 | 3 | 2 | 5 | 3 | 15 | 2 | 121 | 3 | 16 |
| convex-helpers | quod-ts | 6 | 51f7bf848 | [42351051646](https://github.com/pango07/ci-bench/actions/runs/42351051646) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 96 | 2 | 94 | 2 | 2 | 6 | 2 | 0 | 1 | 79 | 1 | 14 |
| convex-helpers | quod-ts | 7 | f5c2b9e9c | [42351170785](https://github.com/pango07/ci-bench/actions/runs/42351170785) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 77 | 1 | 76 | 2 | 2 | 4 | 3 | 0 | 1 | 62 | 1 | 13 |
| convex-helpers | quod-ts | 8 | d244cf22d | [42351288217](https://github.com/pango07/ci-bench/actions/runs/42351288217) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 103 | 2 | 101 | 2 | 2 | 7 | 2 | 0 | 2 | 86 | 0 | 13 |
| convex-helpers | quod-ts | 9 | c976cabdf | [42351436749](https://github.com/pango07/ci-bench/actions/runs/42351436749) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 83 | 2 | 81 | 2 | 2 | 6 | 2 | 0 | 2 | 66 | 1 | 13 |
| convex-helpers | quod-ts | 10 | 6f830b66f | [42351554903](https://github.com/pango07/ci-bench/actions/runs/42351554903) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 161 | 3 | 158 | 3 | 2 | 5 | 4 | 19 | 2 | 117 | 5 | 20 |
| convex-helpers | quod-ts | 11 | 5eaa7e516 | [42351752006](https://github.com/pango07/ci-bench/actions/runs/42351752006) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 129 | 2 | 127 | 3 | 2 | 6 | 3 | 12 | 2 | 93 | 5 | 20 |

</details>


Experimental Quod variants: Cloud ledger and dashboard reporting requested: true.
