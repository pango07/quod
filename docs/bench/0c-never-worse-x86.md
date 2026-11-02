# 0c: the 0b history again, after exp/never-worse

The 40-commit cli/cli history of [0b](https://github.com/pango07/quod/blob/bench/x86-hetzner/docs/bench/0b-x86-hetzner.md)
(`go build`, `go vet`, `go test ./...` on each of the 39 first-parent commits
before cli/cli `9490d204a2ba`, and that one), on the same Hetzner CCX23 (4
dedicated vCPUs, Ubuntu 26.04, kernel 7.0, FUSE over io_uring), with the same
driver (`0c-never-worse-x86/history.py` is 0b's with only the binary and the
output directory changed; `COLD_EVERY=0`: the no-cache series is not part of
this comparison). Quod at exp/never-worse `b4e238d`, measured 2026-11-02
02:58-03:49 UTC under the box's heavy lock. Raw rows:
`0c-never-worse-x86/history-rows.jsonl`; every run's output:
`history-run-outputs.tgz`; tables: `compare.py` (`compare.txt`).

## Totals, commits 2-40

| | Plain, warm Go cache | Quod | Quod / plain |
| --- | --- | --- | --- |
| **Before** (main `5d296c4`, 0b) | 1,230 s (median job 18.3 s) | 2,341 s (median job 51.7 s) | **1.90x**, slower on 39 of 39 commits |
| **After** (exp/never-worse) | 1,237 s (median job 18.3 s) | 1,439 s (median job 21.1 s) | **1.16x**, slower on 34 of 39 commits |

The plain-warm series is re-measured in the same run and agrees with 0b's
within 0.6%.

**The target, Quod's total at or under plain-warm's, is not met.** Quod went
from 1,111 s slower to 202 s slower.

## Per step, after (before)

| Step | Plain-warm median / total | Quod median / total | Quod outcomes |
| --- | --- | --- | --- |
| step A, go mod download | 0.03 s / 2 s | 0.30 s / 13 s (0.70 / 29) | never memoized |
| go build ./... | 2.67 s / 171 s | 4.32 s / 244 s (5.40 / 282) | 39 misses (VCS stamping reads `.git`) |
| go vet ./... | 1.20 s / 109 s | 3.35 s / 191 s (4.32 / 226) | 31 miss, 6 proving, 2 replayed |
| go test ./... | 14.45 s / 949 s | **12.91 s** / 992 s (41.36 / 1,804) | 34 miss, 5 proving |

Median per-step phases of a Quod miss, after (before): setup 0.24 s (0.64),
import 0.35 s for vet and test, 0.60 s for build (0.44, 0.67), lookup 0.06-0.08 s
(0.56-0.62; at commit 40 0.08 s against 1.03 s), record 0.24 s (0.21). Quod's
peak RSS on `go test`: 77 MB at the first commit, 83 MB at the last (80 and
250 MB before).

## Where the remaining 202 s is

- **`go test` on the 9 heavy commits** (dependency bumps and changes under
  `api/`, `internal/config`, `git/`, which most packages import): Quod
  64-70 s against plain's 54-59 s, about +10 s each. Elsewhere Quod's `go test` is
  faster than plain (9.5 s against 12.7 s on a docs-only commit): go test's
  own cache is on under Quod's guard, and Quod's content-derived file times
  keep it valid where a fresh checkout's new mtimes make plain re-run ~40
  packages. The guard is not the heavy commits' cost: run outside Quod at
  commit 7 after commit 6, plain go test took 68.0 s and go test with the
  guard 68.3 s, with the same packages run (`guardcost.sh`). What is left is
  the observer's overhead on a run that compiles and tests most of the
  module.
- **`go vet`**: 2.27 s of execution against plain's 1.2 s. This is the
  observer's latency on the go command's loading phase (every source file
  opened through FUSE), as 0b's overhead section measured it.
- **Fixed cost per step**: about 1 s (setup, import, lookup, record, mount)
  on each of 156 step runs, against 0b's 1.5-3.5 s.

## Correctness

Every run in the history exited 0 on both sides, and every `go test` output
lists all 259 packages. Replays: 2 (`go vet`, commits 33 and 34, as in 0b). No
divergence: proofs compared identical. On GitHub's runners, on this branch,
acceptance-cli gives A7 0 false hits over 23,983 f + 5,336 d + 621 a, A7b
10/10, W1-W4 (W4: 29,907 entries with a full cache and with an empty one, 0
hidden), T1-T11 and P1. Hooks acceptance gives A7 0 false hits over 7,285 f +
1,690 d + 658 a.
