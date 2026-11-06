### Real jobs: where a job's runner-seconds go (jobs API, seconds)

Medians per project and variant. **fixed** = runner-seconds minus install, build and test steps. **boot+setup** = "Set up job" + checkout + package-manager/node set-up (the kill line is 40 s). Queue is not billed and not in runner.

| project | variant | jobs | wall | queue | runner | boot | setup | action | install | build | test | post | **fixed** | **boot+setup** | corpus wall-s | corpus runner-s |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| convex-helpers | floor | 12 | 14 | 2 | 10 | 1 | 5 | 0 | 0 | 0 | 0 | 1 | **10** | **7** | 161 | 121 |
| convex-helpers | plain | 12 | 179 | 2 | 178 | 1 | 6 | 0 | 57 | 2 | 108 | 0 | **10** | **7** | 1893 | 1862 |
| convex-helpers | quod-proof | 12 | 130 | 2 | 128 | 2 | 6 | 3 | 5 | 3 | 106 | 3 | **17** | **8** | 1584 | 1561 |
| convex-helpers | quod-ts-proof | 12 | 119 | 2 | 117 | 2 | 6 | 2 | 6 | 3 | 97 | 3 | **15** | **8** | 1445 | 1424 |
| **all** | floor | 12 | 14 | 2 | 10 | 1 | 5 | 0 | 0 | 0 | 0 | 1 | **10** | **7** | | |
| **all** | plain | 12 | 179 | 2 | 178 | 1 | 6 | 0 | 57 | 2 | 108 | 0 | **10** | **7** | | |
| **all** | quod-proof | 12 | 130 | 2 | 128 | 2 | 6 | 3 | 5 | 3 | 106 | 3 | **17** | **8** | | |
| **all** | quod-ts-proof | 12 | 119 | 2 | 117 | 2 | 6 | 2 | 6 | 3 | 97 | 3 | **15** | **8** | | |

| project | variant | jobs | median wall | runner-s | x plain | billed min | x plain | wrong | unchecked |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|
| convex-helpers | floor | 12 | 14 | 121 | 0.06 | 12 | 0.33 | 0 | 0 |
| convex-helpers | plain | 12 | 179 | 1862 | 1.00 | 36 | 1.00 |  |  |
| convex-helpers | quod-proof | 12 | 130 | 1561 | 0.84 | 32 | 0.89 | 0 | 0 |
| convex-helpers | quod-ts-proof | 12 | 119 | 1424 | 0.76 | 29 | 0.81 | 0 | 0 |

| variant | after-first-job runner-s | x plain | billed min | x plain | best hit above floor | median hit above floor | lineage audits | disagreed |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| quod-proof | 1381 | 0.81 | 29 | 0.88 | no full hits | no full hits | 0 | 0 |
| quod-ts-proof | 1259 | 0.74 | 26 | 0.79 | no full hits | no full hits | 0 | 0 |

<details><summary>per job</summary>

| project | variant | # | commit | run | image | result | Quod | wrong | wall | queue | runner | billed | boot | setup | action | install | build | test | post | fixed |
|---|---|---:|---|---|---|---|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| convex-helpers | floor | 0 | 1d7c57881 | [42167654358](https://github.com/pango07/ci-bench/actions/runs/42167654358) | ubuntu-26.04/20261101.186.1 | success |  | ? | 14 | 2 | 12 | 1 | 2 | 6 | 0 | 0 | 0 | 0 | 1 | 12 |
| convex-helpers | floor | 1 | 290b146e0 | [42167656462](https://github.com/pango07/ci-bench/actions/runs/42167656462) | ubuntu-26.04/20261101.186.1 | success |  | ? | 10 | 2 | 8 | 1 | 1 | 4 | 0 | 0 | 0 | 0 | 0 | 8 |
| convex-helpers | floor | 2 | 1815d4a72 | [42167658792](https://github.com/pango07/ci-bench/actions/runs/42167658792) | ubuntu-26.04/20261101.186.1 | success |  | ? | 13 | 2 | 11 | 1 | 1 | 6 | 0 | 0 | 0 | 0 | 1 | 11 |
| convex-helpers | floor | 3 | 03ffe84ef | [42167661107](https://github.com/pango07/ci-bench/actions/runs/42167661107) | ubuntu-26.04/20261101.186.1 | success |  | ? | 12 | 2 | 10 | 1 | 1 | 5 | 0 | 0 | 0 | 0 | 1 | 10 |
| convex-helpers | floor | 4 | 6b17e1514 | [42167663094](https://github.com/pango07/ci-bench/actions/runs/42167663094) | ubuntu-26.04/20261101.186.1 | success |  | ? | 13 | 2 | 11 | 1 | 2 | 5 | 0 | 0 | 0 | 0 | 1 | 11 |
| convex-helpers | floor | 5 | 49a82e544 | [42167664853](https://github.com/pango07/ci-bench/actions/runs/42167664853) | ubuntu-26.04/20261101.186.1 | success |  | ? | 12 | 3 | 9 | 1 | 0 | 5 | 0 | 0 | 0 | 0 | 0 | 9 |
| convex-helpers | floor | 6 | 51f7bf848 | [42167666813](https://github.com/pango07/ci-bench/actions/runs/42167666813) | ubuntu-26.04/20261101.186.1 | success |  | ? | 14 | 2 | 12 | 1 | 1 | 7 | 0 | 0 | 0 | 0 | 1 | 12 |
| convex-helpers | floor | 7 | f5c2b9e9c | [42167668934](https://github.com/pango07/ci-bench/actions/runs/42167668934) | ubuntu-26.04/20261101.186.1 | success |  | ? | 12 | 2 | 10 | 1 | 1 | 5 | 0 | 0 | 0 | 0 | 1 | 10 |
| convex-helpers | floor | 8 | d244cf22d | [42167670854](https://github.com/pango07/ci-bench/actions/runs/42167670854) | ubuntu-26.04/20261101.186.1 | success |  | ? | 14 | 3 | 11 | 1 | 1 | 6 | 0 | 0 | 0 | 0 | 0 | 11 |
| convex-helpers | floor | 9 | c976cabdf | [42167672689](https://github.com/pango07/ci-bench/actions/runs/42167672689) | ubuntu-26.04/20261101.186.1 | success |  | ? | 14 | 5 | 9 | 1 | 1 | 5 | 0 | 0 | 0 | 0 | 0 | 9 |
| convex-helpers | floor | 10 | 6f830b66f | [42167674631](https://github.com/pango07/ci-bench/actions/runs/42167674631) | ubuntu-26.04/20261101.186.1 | success |  | ? | 17 | 7 | 10 | 1 | 1 | 7 | 0 | 0 | 0 | 0 | 0 | 10 |
| convex-helpers | floor | 11 | 5eaa7e516 | [42167676808](https://github.com/pango07/ci-bench/actions/runs/42167676808) | ubuntu-26.04/20261101.186.1 | success |  | ? | 16 | 8 | 8 | 1 | 1 | 4 | 0 | 0 | 0 | 0 | 1 | 8 |
| convex-helpers | plain | 0 | 1d7c57881 | [42167653272](https://github.com/pango07/ci-bench/actions/runs/42167653272) | ubuntu-26.04/20261101.186.1 | success |  | 122 lines | 169 | 2 | 167 | 3 | 2 | 6 | 0 | 55 | 2 | 98 | 1 | 12 |
| convex-helpers | plain | 1 | 290b146e0 | [42167655532](https://github.com/pango07/ci-bench/actions/runs/42167655532) | ubuntu-26.04/20261101.186.1 | success |  | 122 lines | 100 | 2 | 98 | 2 | 2 | 5 | 0 | 37 | 1 | 51 | 0 | 9 |
| convex-helpers | plain | 2 | 1815d4a72 | [42167657868](https://github.com/pango07/ci-bench/actions/runs/42167657868) | ubuntu-26.04/20261101.186.1 | success |  | 128 lines | 181 | 2 | 179 | 3 | 1 | 5 | 0 | 57 | 2 | 111 | 0 | 9 |
| convex-helpers | plain | 3 | 03ffe84ef | [42167660078](https://github.com/pango07/ci-bench/actions/runs/42167660078) | ubuntu-26.04/20261101.186.1 | success |  | 128 lines | 180 | 1 | 179 | 3 | 1 | 5 | 0 | 56 | 3 | 111 | 0 | 9 |
| convex-helpers | plain | 4 | 6b17e1514 | [42167662397](https://github.com/pango07/ci-bench/actions/runs/42167662397) | ubuntu-26.04/20261101.186.1 | success |  | 128 lines | 184 | 2 | 182 | 4 | 1 | 6 | 0 | 58 | 2 | 111 | 1 | 11 |
| convex-helpers | plain | 5 | 49a82e544 | [42167664258](https://github.com/pango07/ci-bench/actions/runs/42167664258) | ubuntu-26.04/20261101.186.1 | success |  | 128 lines | 178 | 2 | 176 | 3 | 2 | 6 | 0 | 57 | 2 | 105 | 0 | 12 |
| convex-helpers | plain | 6 | 51f7bf848 | [42167666212](https://github.com/pango07/ci-bench/actions/runs/42167666212) | ubuntu-26.04/20261101.186.1 | success |  | 128 lines | 182 | 2 | 180 | 3 | 1 | 5 | 0 | 57 | 2 | 112 | 0 | 9 |
| convex-helpers | plain | 7 | f5c2b9e9c | [42167668337](https://github.com/pango07/ci-bench/actions/runs/42167668337) | ubuntu-26.04/20261101.186.1 | success |  | 128 lines | 182 | 1 | 181 | 4 | 1 | 5 | 0 | 58 | 2 | 112 | 0 | 9 |
| convex-helpers | plain | 8 | d244cf22d | [42167670257](https://github.com/pango07/ci-bench/actions/runs/42167670257) | ubuntu-26.04/20261101.186.1 | success |  | 128 lines | 145 | 2 | 143 | 3 | 3 | 8 | 0 | 48 | 1 | 80 | 0 | 14 |
| convex-helpers | plain | 9 | c976cabdf | [42167672168](https://github.com/pango07/ci-bench/actions/runs/42167672168) | ubuntu-26.04/20261101.186.1 | success |  | 128 lines | 105 | 5 | 100 | 2 | 1 | 6 | 0 | 35 | 1 | 54 | 0 | 10 |
| convex-helpers | plain | 10 | 6f830b66f | [42167674093](https://github.com/pango07/ci-bench/actions/runs/42167674093) | ubuntu-26.04/20261101.186.1 | success |  | 128 lines | 186 | 5 | 181 | 4 | 1 | 6 | 0 | 57 | 2 | 110 | 0 | 12 |
| convex-helpers | plain | 11 | 5eaa7e516 | [42167676889](https://github.com/pango07/ci-bench/actions/runs/42167676889) | ubuntu-26.04/20261101.186.1 | success |  | 128 lines | 101 | 5 | 96 | 2 | 1 | 4 | 0 | 33 | 2 | 52 | 0 | 9 |
| convex-helpers | quod-proof | 0 | 1d7c57881 | [42167654653](https://github.com/pango07/ci-bench/actions/runs/42167654653) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 182 | 2 | 180 | 3 | 2 | 5 | 1 | 57 | 3 | 106 | 4 | 14 |
| convex-helpers | quod-proof | 1 | 290b146e0 | [42167979699](https://github.com/pango07/ci-bench/actions/runs/42167979699) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 129 | 3 | 126 | 3 | 2 | 5 | 3 | 1 | 3 | 106 | 2 | 16 |
| convex-helpers | quod-proof | 2 | 1815d4a72 | [42168200902](https://github.com/pango07/ci-bench/actions/runs/42168200902) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 103 | 2 | 101 | 2 | 3 | 5 | 2 | 9 | 3 | 74 | 3 | 15 |
| convex-helpers | quod-proof | 3 | 03ffe84ef | [42168421235](https://github.com/pango07/ci-bench/actions/runs/42168421235) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 118 | 2 | 116 | 2 | 2 | 6 | 4 | 0 | 3 | 95 | 2 | 18 |
| convex-helpers | quod-proof | 4 | 6b17e1514 | [42168627158](https://github.com/pango07/ci-bench/actions/runs/42168627158) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 167 | 2 | 165 | 3 | 3 | 6 | 2 | 15 | 3 | 128 | 4 | 19 |
| convex-helpers | quod-proof | 5 | 49a82e544 | [42168949728](https://github.com/pango07/ci-bench/actions/runs/42168949728) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 107 | 2 | 105 | 2 | 2 | 7 | 3 | 12 | 2 | 71 | 4 | 20 |
| convex-helpers | quod-proof | 6 | 51f7bf848 | [42169170693](https://github.com/pango07/ci-bench/actions/runs/42169170693) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 166 | 2 | 164 | 3 | 3 | 6 | 4 | 0 | 4 | 141 | 2 | 19 |
| convex-helpers | quod-proof | 7 | f5c2b9e9c | [42169501597](https://github.com/pango07/ci-bench/actions/runs/42169501597) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 73 | 1 | 72 | 2 | 1 | 6 | 2 | 0 | 2 | 57 | 1 | 13 |
| convex-helpers | quod-proof | 8 | d244cf22d | [42169665118](https://github.com/pango07/ci-bench/actions/runs/42169665118) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 129 | 2 | 127 | 3 | 2 | 7 | 3 | 0 | 3 | 109 | 1 | 15 |
| convex-helpers | quod-proof | 9 | c976cabdf | [42169880310](https://github.com/pango07/ci-bench/actions/runs/42169880310) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 130 | 2 | 128 | 3 | 2 | 5 | 4 | 0 | 4 | 107 | 2 | 17 |
| convex-helpers | quod-proof | 10 | 6f830b66f | [42170094544](https://github.com/pango07/ci-bench/actions/runs/42170094544) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 140 | 2 | 138 | 3 | 3 | 5 | 3 | 18 | 2 | 98 | 5 | 20 |
| convex-helpers | quod-proof | 11 | 5eaa7e516 | [42170368545](https://github.com/pango07/ci-bench/actions/runs/42170368545) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 140 | 1 | 139 | 3 | 1 | 5 | 2 | 15 | 3 | 105 | 4 | 16 |
| convex-helpers | quod-ts-proof | 0 | 1d7c57881 | [42167654863](https://github.com/pango07/ci-bench/actions/runs/42167654863) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 167 | 2 | 165 | 3 | 2 | 7 | 1 | 56 | 3 | 86 | 6 | 20 |
| convex-helpers | quod-ts-proof | 1 | 290b146e0 | [42167979760](https://github.com/pango07/ci-bench/actions/runs/42167979760) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 71 | 2 | 69 | 2 | 2 | 5 | 3 | 0 | 2 | 53 | 1 | 14 |
| convex-helpers | quod-ts-proof | 2 | 1815d4a72 | [42168152538](https://github.com/pango07/ci-bench/actions/runs/42168152538) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 170 | 1 | 169 | 3 | 1 | 5 | 2 | 19 | 3 | 133 | 3 | 14 |
| convex-helpers | quod-ts-proof | 3 | 03ffe84ef | [42168467914](https://github.com/pango07/ci-bench/actions/runs/42168467914) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 121 | 2 | 119 | 2 | 2 | 6 | 2 | 0 | 3 | 103 | 1 | 13 |
| convex-helpers | quod-ts-proof | 4 | 6b17e1514 | [42168673826](https://github.com/pango07/ci-bench/actions/runs/42168673826) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 93 | 1 | 92 | 2 | 2 | 5 | 2 | 11 | 1 | 66 | 3 | 14 |
| convex-helpers | quod-ts-proof | 5 | 49a82e544 | [42168844191](https://github.com/pango07/ci-bench/actions/runs/42168844191) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 107 | 2 | 105 | 2 | 2 | 6 | 3 | 12 | 2 | 72 | 4 | 19 |
| convex-helpers | quod-ts-proof | 6 | 51f7bf848 | [42169067560](https://github.com/pango07/ci-bench/actions/runs/42169067560) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 116 | 2 | 114 | 2 | 2 | 6 | 2 | 0 | 2 | 98 | 1 | 14 |
| convex-helpers | quod-ts-proof | 7 | f5c2b9e9c | [42169297232](https://github.com/pango07/ci-bench/actions/runs/42169297232) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 116 | 2 | 114 | 2 | 2 | 7 | 2 | 0 | 3 | 95 | 2 | 16 |
| convex-helpers | quod-ts-proof | 8 | d244cf22d | [42169520221](https://github.com/pango07/ci-bench/actions/runs/42169520221) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 128 | 2 | 126 | 3 | 1 | 6 | 3 | 0 | 3 | 107 | 2 | 16 |
| convex-helpers | quod-ts-proof | 9 | c976cabdf | [42169729433](https://github.com/pango07/ci-bench/actions/runs/42169729433) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 70 | 1 | 69 | 2 | 2 | 6 | 2 | 0 | 2 | 53 | 1 | 14 |
| convex-helpers | quod-ts-proof | 10 | 6f830b66f | [42169898045](https://github.com/pango07/ci-bench/actions/runs/42169898045) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 146 | 2 | 144 | 3 | 1 | 6 | 2 | 20 | 3 | 106 | 3 | 15 |
| convex-helpers | quod-ts-proof | 11 | 5eaa7e516 | [42170161226](https://github.com/pango07/ci-bench/actions/runs/42170161226) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 140 | 2 | 138 | 3 | 1 | 6 | 2 | 15 | 3 | 105 | 5 | 15 |

</details>

