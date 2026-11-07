### Real jobs: where a job's runner-seconds go (jobs API, seconds)

Medians per project and variant. **fixed** = runner-seconds minus install, build and test steps. **boot+setup** = "Set up job" + checkout + package-manager/node set-up (the kill line is 40 s). Queue is not billed and not in runner.

| project | variant | jobs | wall | queue | runner | boot | setup | action | install | build | test | post | **fixed** | **boot+setup** | corpus wall-s | corpus runner-s |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| convex-helpers | floor | 2 | 12 | 1 | 11 | 2 | 6 | 0 | 0 | 0 | 0 | 0 | **11** | **7** | 23 | 21 |
| convex-helpers | plain | 2 | 157 | 2 | 156 | 1 | 5 | 0 | 52 | 2 | 94 | 1 | **9** | **6** | 314 | 311 |
| convex-helpers | quod-post | 2 | 135 | 1 | 134 | 3 | 6 | 2 | 31 | 2 | 82 | 7 | **20** | **8** | 269 | 267 |
| convex-helpers | quod | 2 | 171 | 2 | 169 | 4 | 8 | 2 | 34 | 3 | 113 | 4 | **20** | **11** | 342 | 338 |
| **all** | floor | 2 | 12 | 1 | 11 | 2 | 6 | 0 | 0 | 0 | 0 | 0 | **11** | **7** | | |
| **all** | plain | 2 | 157 | 2 | 156 | 1 | 5 | 0 | 52 | 2 | 94 | 1 | **9** | **6** | | |
| **all** | quod-post | 2 | 135 | 1 | 134 | 3 | 6 | 2 | 31 | 2 | 82 | 7 | **20** | **8** | | |
| **all** | quod | 2 | 171 | 2 | 169 | 4 | 8 | 2 | 34 | 3 | 113 | 4 | **20** | **11** | | |

| project | variant | jobs | median wall | runner-s | x plain | billed min | x plain | wrong | unchecked |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|
| convex-helpers | floor | 2 | 12 | 21 | 0.07 | 2 | 0.33 | 0 | 0 |
| convex-helpers | plain | 2 | 157 | 311 | 1.00 | 6 | 1.00 |  |  |
| convex-helpers | quod-post | 2 | 135 | 267 | 0.86 | 5 | 0.83 | 0 | 0 |
| convex-helpers | quod | 2 | 171 | 338 | 1.09 | 6 | 1.00 | 0 | 0 |

| variant | after-first-job runner-s | x plain | billed min | x plain | best hit above floor | median hit above floor | lineage audits | disagreed |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| quod-post | 95 | 0.53 | 2 | 0.67 | no full hits | no full hits | 0 | 0 |
| quod | 161 | 0.90 | 3 | 1.00 | no full hits | no full hits | 0 | 0 |

<details><summary>per job</summary>

| project | variant | # | commit | run | image | result | Quod | wrong | wall | queue | runner | billed | boot | setup | action | install | build | test | post | fixed |
|---|---|---:|---|---|---|---|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| convex-helpers | floor | 0 | 6f830b66f | [42352075324](https://github.com/pango07/ci-bench/actions/runs/42352075324) | ubuntu-26.04/20261101.186.1 | success |  | ? | 13 | 1 | 12 | 1 | 2 | 6 | 0 | 0 | 0 | 0 | 0 | 12 |
| convex-helpers | floor | 1 | 5eaa7e516 | [42352076750](https://github.com/pango07/ci-bench/actions/runs/42352076750) | ubuntu-26.04/20261101.186.1 | success |  | ? | 10 | 1 | 9 | 1 | 1 | 5 | 0 | 0 | 0 | 0 | 0 | 9 |
| convex-helpers | plain | 0 | 6f830b66f | [42352074278](https://github.com/pango07/ci-bench/actions/runs/42352074278) | ubuntu-26.04/20261101.186.1 | success |  | 128 lines | 133 | 1 | 132 | 3 | 1 | 4 | 0 | 46 | 1 | 76 | 1 | 9 |
| convex-helpers | plain | 1 | 5eaa7e516 | [42352075684](https://github.com/pango07/ci-bench/actions/runs/42352075684) | ubuntu-26.04/20261101.186.1 | success |  | 128 lines | 181 | 2 | 179 | 3 | 1 | 5 | 0 | 58 | 2 | 111 | 0 | 8 |
| convex-helpers | quod | 0 | 6f830b66f | [42352075154](https://github.com/pango07/ci-bench/actions/runs/42352075154) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 179 | 2 | 177 | 3 | 4 | 9 | 1 | 48 | 3 | 104 | 4 | 22 |
| convex-helpers | quod | 1 | 5eaa7e516 | [42352293766](https://github.com/pango07/ci-bench/actions/runs/42352293766) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 163 | 2 | 161 | 3 | 3 | 6 | 3 | 19 | 2 | 122 | 4 | 18 |
| convex-helpers | quod-post | 0 | 6f830b66f | [42352075191](https://github.com/pango07/ci-bench/actions/runs/42352075191) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 173 | 1 | 172 | 3 | 2 | 5 | 1 | 51 | 2 | 102 | 6 | 17 |
| convex-helpers | quod-post | 1 | 5eaa7e516 | [42352294203](https://github.com/pango07/ci-bench/actions/runs/42352294203) | ubuntu-26.04/20261101.186.1 | success | miss | 0 | 96 | 1 | 95 | 2 | 3 | 6 | 2 | 11 | 1 | 61 | 8 | 22 |

</details>


Experimental Quod variants: Cloud ledger and dashboard reporting requested: true.
