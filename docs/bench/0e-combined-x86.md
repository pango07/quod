# 0e: Quod at main against a warm Go cache, on x86 (history and realistic traffic)

Quod main `37e938a` (exp/never-worse: Go's test cache under Quod's guard and
the lookup index; trust v2: proofs 1, 2 shadow audits, decay audits, suspect
marking). cli/cli pinned at `9490d204a2ba`, plus the 39 first-parent
commits before it. Steps: `go build ./...`, `go vet ./...`, `go test ./...`.
The box was a fresh Hetzner CCX23, "quod-bench-3": 4 dedicated AMD EPYC
vCPUs, 16 GB, Ubuntu 26.04.1, kernel 7.0.0-31, Ashburn, FUSE over io_uring.
Nothing else ran on it. Measured 2026-11-02, 10:55 to 13:21 UTC. Plain-warm
and Quod ran on the same box, from the same driver, interleaved. Raw rows,
logs, ledger dumps and scripts are in `0e-combined-x86/`.

## Summary

**Short answer: not yet. Quod at main is still slower than plain CI with a
warm Go cache, in total, on both workloads. On realistic traffic the gap is
small (9%), and Quod wins a little over half the jobs.**

| | Plain, warm Go cache | Quod at main | Quod / plain | Jobs where Quod is faster |
| --- | --- | --- | --- | --- |
| **(1) History**: 40 commits, one job each (commits 2-40) | 1,250 s (median job 18.9 s) | 1,519 s (median job 23.2 s) | **1.22x** | 7 of 39 |
| **(2) Realistic traffic**: PR head, then merge, then a re-run 10% of the time (87 jobs) | 2,237 s (median job 16.9 s) | 2,443 s (median job 16.5 s) | **1.09x** | 45 of 87 |
| (2) merge jobs only, after commit 1 | 671 s | 673 s | 1.00x | 25 of 39 |
| (2) re-runs only | 114 s | 73.5 s | **0.65x** | 6 of 7 |
| (2) PR jobs only, after commit 1 | 1,311 s | 1,533 s | 1.17x | 13 of 39 |

For comparison, on the same history: 0b (main `5d296c4`) was **1.90x**,
slower on 39 of 39 commits. 0c (exp/never-worse alone, trust v1) was
**1.16x**, slower on 34 of 39. **Main is 1.22x, slower on 32 of 39. That is
slightly worse than 0c.** The plain-warm baseline agrees with 0b's (1,230 s),
0c's (1,237 s) and trust-v2's (1,265 s) to within 2%.

**On (1), the history, Quod loses by 269 s over 39 commits.** Where that goes:

- **+99 s: trust v2's audits.** A shadow or decay audit really re-executes
  the step, so `go test` runs with Go's test cache off: 40-41 s against
  plain's 10-13 s. That happened 3 times (commits 2, 5, 32). Under 0c's trust
  v1, the same commits were "proving" runs that used the test cache (5.6-9.5
  s). Trust v2 bought 2 `go test` replays and 4 `go vet` replays here (-25 s).
  On this history that is a net loss of ~75 s against 0c. (Plain's checkout,
  which Quod's job does not time, is the remaining -7 s.)
- **+152 s: the 11 heavy commits.** These are dependency bumps and changes
  under `api/`, `internal/config` and `git/`, which re-run most of the suite
  on both sides. On the 8 heaviest, Quod's `go test` takes 66-70 s against
  plain's 55-59 s.
  The +10-12 s is the observer's overhead on a step that compiles and tests
  most of the module. 0c measured the same, and showed the guard is not the
  cause.
- **+104 s: fixed costs on ordinary commits.** `go build` can never replay
  (it reads `.git` for VCS stamping) and costs about 1.9 s more per commit. A
  `go vet` miss costs about 2 s more. Step A costs 0.3 s against 0.03 s. Per keyed
  step, Quod's own time is about 0.7-0.9 s: setup 0.25 s, import 0.36-0.60 s,
  mount 0.12 s, record 0.24 s.
- **-55 s: `go test` on ordinary commits is faster under Quod.** A miss runs
  only the affected packages, through Go's own test cache. Quod's
  content-derived mtimes keep that cache valid where a fresh checkout's new
  mtimes make plain re-run ~40 packages: median 13.8 s of execution against
  plain's 14.5 s, and 6-9 s against 11-13 s on small commits.

**On (2), realistic traffic, Quod loses by 184 s over 85 jobs (commit 1
excluded): +9%.**

- **Replays: 56 of 73 unchanged-input step runs (77%).** That is 21% of all
  261 keyed step runs. A replay takes 0.7-0.9 s. `go test` replayed 12 times
  (3 PR, 3 merge, 6 re-runs), each in 0.7-0.9 s against plain's 10-16 s, for
  -142 s. `go vet` replayed 41 times and `go build` 3 times (re-runs only).
- **Audits: +172 s.** 17 audits (6 shadow, 11 decay). The 5 `go test`
  audits cost 149 s of that, at ~30 s each over plain.
- **Heavy commits: +178 s**, the same overhead as in (1), on both the PR and
  the merge job.
- **Everything else is even (-1 s):** `go test` misses on ordinary commits
  are faster than plain by 195 s, and that pays for `go build`, `go vet` and
  step A's fixed costs.
- **Why the merge job rarely replays `go test` (3 of 40):** `go test` reads
  `.git/HEAD`. The PR job runs on branch `pr` and the merge job on `main`, so
  68 of `go test`'s 70 traffic misses name `.git/HEAD` as their first changed
  input (trust-v2 saw the same: 66 of 69). `go vet` does not read it, and
  replays on 28 of 34 merge jobs whose inputs are unchanged.

**Correctness: no false hit, no divergence, no suspect mark.** All 24 audits
(7 in the history, 17 in traffic) compared identical: byte-identical for vet
and build, identical up to normalized durations for test. Every key is
`proven`, 0 divergences, 0 quarantined, 0 suspect runs. Every one of the 62
replays was re-run plainly on a fresh checkout (`go test` as `-count=1`), and
all 62 agreed (exit code and normalized stdout; for the 14 `go test` replays every one of the 319-321 result lines matched). Every run in both series exited 0.

**Where Quod still loses, in order of size:**

1. **Audits that re-run the full `go test` suite** (+30 s each against a
   warm plain run). This cost is new with trust v2. It is the price of an
   audit that really executes. Options: audit less often per key once it has
   a clean record; or let an audit use Go's test cache for packages whose
   cached result was itself executed and recorded by Quod, then compare. The
   second needs a soundness argument first.
2. **Observer overhead on heavy commits** (~+10 s on a 56 s `go test`, +15
   s per job), unchanged since 0c.
3. **Fixed costs on steps that cannot replay**: `go build` (reads `.git`)
   and `go vet` misses, about 1-2 s each, every job. `-buildvcs=false` would
   make `go build` replayable.
4. **`.git/HEAD` in `go test`'s inputs** keeps the PR-then-merge pair from
   replaying `go test`, the case trust v2 was meant to unlock.

Where Quod wins today: re-runs (0.65x, 6 of 7 faster). Merge jobs whose
tree matches the PR head, for `go vet`. Docs- and CI-only commits (commits
23, 33, 34, 36 in the history: 5.9-10.9 s against 16 s). Ordinary
small-change commits, where Quod's `go test` miss beats plain's warm cache
(4, 35, 37).

## Main against 0c, per step (history, commits 2-40)

`compare.py` (0c's) on 0c's rows and these (`compare.txt`):

| Step | Plain-warm total, 0c / 0e | Quod total, 0c / 0e | Quod outcomes, 0c | Quod outcomes, 0e |
| --- | --- | --- | --- | --- |
| step A | 2 / 2 s | 13 / 14 s | not memoized | not memoized |
| `go build ./...` | 171 / 174 s | 244 / 248 s | 39 miss | 39 miss |
| `go vet ./...` | 109 / 110 s | 191 / 189 s | 31 miss, 6 proving, 2 replayed | 31 miss, 4 audit, 4 replayed |
| `go test ./...` | 949 / 958 s | **992 / 1,068 s** | 34 miss, 5 proving | 34 miss, 3 audit, 2 replayed |
| **job** | 1,237 / 1,250 s | **1,439 / 1,519 s** | 1.16x | **1.22x** |

The whole difference from 0c is `go test` (+76 s), and within it, trust
v2: 0c's proving runs used Go's test cache (5.6-10.7 s), while 0e's audits
run the full suite (40-41 s, 3 of them). The 2 replays saved 24 s.
Everything else is within run-to-run noise: per-step medians for setup,
import, lookup and record differ by 10 ms or less.

## History: totals, commits 2-40

| | Total | Median job | p90 job | Min-max job |
| --- | --- | --- | --- | --- |
| Plain, warm Go cache | 1,250 s | 18.9 s | 75.1 s | 13.4-78.8 s |
| Quod (main) | 1,519 s | 23.2 s | 91.2 s | 5.9-94.7 s |

Quod / plain total: **1.22x**; median job ratio 1.16x; Quod faster on **7 of 39** commits (4, 23, 33, 34, 35, 36, 37); Quod - plain = +269 s. Non-zero exits: none.

### Per step, commits 2-40

| Step | Plain median / total | Quod median / total | Quod outcomes | Quod exec median / total |
| --- | --- | --- | --- | --- |
| `go mod download` | 0.03 s / 1.8 s | 0.31 s / 13.5 s | exit 0 39 | 0.31 s / 13.5 s |
| `go build ./...` | 2.68 s / 174 s | 4.39 s / 248 s | miss 39 | 2.98 s / 187 s |
| `go vet ./...` | 1.25 s / 110 s | 3.50 s / 189 s | miss 31, audit 4, replayed 4 | 2.29 s / 145 s |
| `go test ./...` | 14.51 s / 958 s | 14.98 s / 1,068 s | miss 34, audit 3, replayed 2 | 13.82 s / 1,023 s |

### Where the difference goes (Quod - plain, seconds, commits 2-40)

Per step, split into the step's execution against plain's whole step (exec - plain wall) and Quod's own
time around it (wall - exec: setup, import, lookup, mount, diff, outputs, record, cleanup, process start).

| Step | Quod - plain | of which exec - plain | of which Quod around exec | on replays | on audits | on misses, heavy commits (plain go test > 30 s) | on misses, other commits |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `go mod download` | +12 | +12 | +0 | +0 | +0 | +3 | +8 |
| `go build ./...` | +74 | +13 | +62 | +0 | +0 | +22 | +52 |
| `go vet ./...` | +79 | +35 | +44 | -0 | +9 | +26 | +44 |
| `go test ./...` | +111 | +66 | +45 | -24 | +90 | +101 | -55 |
| plain's checkout (Quod's is not timed) | -7 | | | | | | |
| **job** | **+269** | +125 | +151 | -25 | +99 | +152 | +50 |

### Quod phases per keyed step, median ms over commits 2-40 (executions / replays)

| Step | n exec / replay | setup | import | lookup | mount | serve | overlay | seal | exec | diff | outputs | unmount | manifest | record | cleanup | wall - total_ms |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `go build ./...` exec | 39 | 253 | 601 | 57 | 125 | 110 | 0 | 13 | 2982 | 106 | 46 | 23 | 32 | 245 | 1 | 935 |
| `go vet ./...` exec | 35 | 253 | 360 | 77 | 123 | 109 | 0 | 13 | 2368 | 67 | 12 | 21 | 32 | 241 | 1 | 711 |
| `go vet ./...` replay | 4 | 248 | 356 | 115 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 17 |
| `go test ./...` exec | 37 | 252 | 356 | 72 | 123 | 109 | 0 | 13 | 14527 | 80 | 21 | 22 | 31 | 239 | 1 | 710 |
| `go test ./...` replay | 2 | 258 | 363 | 112 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 17 |

### Per commit

job = checkout + A + 3 steps (plain), A + 3 steps (Quod). Quod cells: outcome and wall; the last column is go test's first changed input.

| # | Commit | Subject | plain job | quod job | quod - plain | build | vet | test | plain test | test first changed input |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | `b1c8c0ae5a` | Merge pull request #14750 from cli/williammartin | 129.6s | 152.0s | +22.4s | miss 49.3s | miss 18.3s | miss 81.4s | 69.0s | `-` |
| 2 | `5b0cc54248` | chore(deps): bump the codeql-actions group with  | 13.4s | 49.0s | +35.6s | miss 4.0s | audit 3.2s | audit 41.5s | 10.0s | `-` |
| 3 | `ed40e856ff` | Merge pull request #14719 from cli/bagtoad/fix-h | 14.7s | 16.3s | +1.5s | miss 4.2s | miss 3.3s | miss 8.5s | 11.0s | `pkg/cmd/ruleset/view/view.go` |
| 4 | `5cfe30624f` | Bump Go to 1.27.3 (#14767) | 14.2s | 13.7s | -0.4s | miss 3.9s | miss 3.0s | miss 6.5s | 10.8s | `.github/golangci-lint-version` |
| 5 | `ad27e286ab` | Merge pull request #14770 from cli/bagtoad/merge | 14.1s | 47.9s | +33.9s | miss 3.9s | audit 3.1s | audit 40.6s | 10.7s | `-` |
| 6 | `401854ece8` | chore(deps): bump github.com/gabriel-vasile/mime | 17.2s | 19.7s | +2.5s | miss 5.2s | miss 3.3s | miss 10.9s | 12.9s | `.quod/gomod/cache/download/github.com/gabriel-vasile/mimetype/@v` |
| 7 | `1531837d85` | Merge pull request #14148 from heaths/issue3718 | 78.8s | 94.7s | +15.9s | miss 13.6s | miss 10.8s | miss 69.6s | 58.9s | `.quod/gomod/cache/download/charm.land` |
| 8 | `ad4a6456e1` | Merge pull request #14450 from cli/babakks/refre | 76.1s | 92.7s | +16.6s | miss 12.8s | miss 10.1s | miss 69.2s | 57.5s | `.gitignore` |
| 9 | `4f168ce66d` | Merge pull request #14451 from cli/babakks/refre | 76.7s | 92.4s | +15.7s | miss 12.1s | miss 10.4s | miss 69.6s | 58.0s | `api/http_client.go` |
| 10 | `a81663f7d8` | Merge pull request #14452 from cli/babakks/refre | 21.2s | 24.3s | +3.1s | miss 4.6s | miss 3.8s | miss 15.7s | 16.6s | `internal/authflow/flow.go` |
| 11 | `b7887068cb` | Merge pull request #14453 from cli/babakks/refre | 17.8s | 21.1s | +3.4s | miss 4.2s | miss 3.4s | miss 13.3s | 14.0s | `pkg/cmd/auth/token/token.go` |
| 12 | `bd0b0e085f` | Merge pull request #14454 from cli/babakks/refre | 18.9s | 21.5s | +2.7s | miss 4.1s | miss 3.4s | miss 13.7s | 14.8s | `pkg/cmd/auth/status/status.go` |
| 13 | `d8474903cf` | Merge pull request #14455 from cli/babakks/refre | 19.6s | 23.2s | +3.6s | miss 4.4s | miss 3.5s | miss 15.0s | 15.4s | `pkg/cmd/agent-task/capi/client.go` |
| 14 | `316e7a80fb` | Merge pull request #14456 from cli/babakks/refre | 16.7s | 18.3s | +1.6s | miss 3.9s | miss 3.0s | miss 11.2s | 13.3s | `script` |
| 15 | `4a289a9e19` | Merge pull request #14663 from cli/babakks/refre | 76.4s | 91.3s | +15.0s | miss 12.1s | miss 10.3s | miss 68.5s | 57.8s | `api/client_test.go` |
| 16 | `e62e84ca88` | Merge pull request #14665 from cli/babakks/refre | 19.2s | 21.4s | +2.2s | miss 4.8s | miss 3.8s | miss 12.5s | 14.3s | `api/http_client.go` |
| 17 | `56682b6faf` | Merge pull request #14672 from cli/babakks/refre | 74.2s | 88.2s | +14.0s | miss 11.8s | miss 10.0s | miss 66.0s | 56.2s | `internal/config/config.go` |
| 18 | `abf0e0c25a` | Merge pull request #14673 from cli/babakks/refre | 20.4s | 23.1s | +2.7s | miss 4.4s | miss 3.5s | miss 15.0s | 16.1s | `pkg/cmd/auth/auth.go` |
| 19 | `6314d776e1` | Merge pull request #14717 from cli/babakks/refre | 73.5s | 89.0s | +15.5s | miss 11.9s | miss 10.2s | miss 66.5s | 55.4s | `internal/config/config.go` |
| 20 | `547bf81aa5` | Merge pull request #14745 from cli/babakks/refre | 48.3s | 58.4s | +10.1s | miss 8.1s | miss 7.0s | miss 43.0s | 37.0s | `internal/text/text.go` |
| 21 | `ae7b971597` | Merge pull request #14752 from cli/babakks/refre | 40.7s | 47.3s | +6.6s | miss 5.2s | miss 5.9s | miss 35.9s | 33.7s | `Makefile` |
| 22 | `99750e1576` | Merge pull request #14768 from cli/babakks/refre | 21.2s | 25.6s | +4.4s | miss 5.8s | miss 3.6s | miss 15.6s | 16.6s | `.quod/gomod/cache/download/github.com/cli/oauth/@v` |
| 23 | `7948e8056e` | Merge pull request #14782 from cli/babakks/skill | 16.1s | 5.9s | -10.2s | miss 4.0s | **HIT** 0.8s | **HIT** 0.8s | 12.9s | `-` |
| 24 | `5056c0a8ef` | Merge pull request #14784 from cli/tommaso-moro- | 18.1s | 20.0s | +2.0s | miss 4.1s | miss 3.2s | miss 12.5s | 14.5s | `pkg/cmd/skills/install/install.go` |
| 25 | `ed84a97a55` | Merge pull request #14791 from cli/bdehamer-depe | 24.6s | 30.4s | +5.8s | miss 6.9s | miss 4.0s | miss 19.2s | 18.6s | `.quod/gomod/cache/download/github.com/digitorus/pkcs7/@v` |
| 26 | `e87ed4c751` | Document extrepo installation on Debian (#14796) | 15.8s | 16.6s | +0.8s | miss 3.9s | audit 3.1s | miss 9.3s | 12.4s | `docs/install_linux.md` |
| 27 | `64d175234b` | Merge pull request #14798 from cli/babakks/polis | 17.4s | 19.8s | +2.4s | miss 4.2s | miss 3.3s | miss 12.0s | 13.7s | `pkg/cmd/auth/login/login.go` |
| 28 | `059f4975b5` | Merge pull request #14804 from martinajir/attach | 27.5s | 33.6s | +6.1s | miss 5.6s | miss 4.5s | miss 23.2s | 21.2s | `internal/attachments/client.go` |
| 29 | `4f2cfe92e3` | Merge pull request #14805 from cli/bagtoad/attac | 46.4s | 54.2s | +7.8s | miss 6.5s | miss 6.5s | miss 40.9s | 37.3s | `internal/attachments/client.go` |
| 30 | `63837fb778` | Merge pull request #14815 from cli/babakks/auth- | 17.5s | 19.7s | +2.1s | miss 4.2s | miss 3.4s | miss 11.8s | 13.8s | `pkg/cmd/auth/status/status.go` |
| 31 | `0186b810be` | Merge pull request #14801 from tj-smith47/remove | 75.1s | 91.2s | +16.1s | miss 12.0s | miss 10.4s | miss 68.4s | 56.8s | `api` |
| 32 | `9be5608274` | Merge pull request #14824 from cli/dependabot/gi | 16.1s | 46.0s | +29.9s | miss 4.0s | **HIT** 0.8s | audit 40.9s | 12.7s | `-` |
| 33 | `5687d4ffe8` | Merge pull request #14837 from cli/williammartin | 16.2s | 10.9s | -5.3s | miss 4.1s | **HIT** 0.8s | miss 5.7s | 12.8s | `.github` |
| 34 | `93d24ce9ff` | Only save Go caches from trunk (#14840) | 16.3s | 8.4s | -7.9s | miss 4.0s | audit 3.2s | **HIT** 0.8s | 12.9s | `-` |
| 35 | `674516ff39` | Merge pull request #14854 from bobrik/ivan/norma | 16.8s | 13.5s | -3.2s | miss 4.1s | miss 3.2s | miss 6.0s | 13.3s | `pkg/cmd/extension/command_test.go` |
| 36 | `f7a06a18a9` | Merge pull request #14825 from cli/bagtoad/ghsa- | 16.2s | 10.8s | -5.3s | miss 4.0s | **HIT** 0.8s | miss 5.7s | 12.8s | `.github` |
| 37 | `8553724d15` | Merge pull request #14858 from cli/bagtoad/priva | 18.0s | 17.6s | -0.4s | miss 4.4s | miss 3.5s | miss 9.4s | 14.0s | `acceptance/testdata/pr/pr-checkout-with-url-from-fork.txtar` |
| 38 | `30eb70ca63` | Merge pull request #14878 from cli/dependabot/go | 31.3s | 36.4s | +5.2s | miss 10.8s | miss 5.5s | miss 19.4s | 20.2s | `.quod/gomod/cache/download/go.opentelemetry.io/otel/@v` |
| 39 | `443baeba05` | Merge pull request #14355 from cli/williammartin | 73.6s | 90.6s | +17.0s | miss 12.3s | miss 10.6s | miss 67.4s | 56.0s | `git` |
| 40 | `9490d204a2` | Merge pull request #14879 from cli/bagtoad/show- | 14.1s | 14.2s | +0.1s | miss 4.2s | miss 3.3s | miss 6.4s | 10.6s | `git/client_test.go` |

## Realistic traffic: PR head, merge, 10% re-runs

87 jobs (pr 40, merge 40, rerun 7). Non-zero exits: none.

| Jobs | n | Plain total | Quod total | Quod / plain | Plain median | Quod median | Quod faster on |
| --- | --- | --- | --- | --- | --- | --- | --- |
| all | 87 | 2,237 s | 2,443 s | **1.09x** | 16.9 s | 16.5 s | 45 of 87 |
| after commit 1 | 85 | 2,096 s | 2,280 s | **1.09x** | 16.9 s | 16.5 s | 44 of 85 |
| pr, after commit 1 | 39 | 1,311 s | 1,533 s | **1.17x** | 19.5 s | 24.6 s | 13 of 39 |
| merge, after commit 1 | 39 | 671 s | 673 s | **1.00x** | 16.4 s | 15.0 s | 25 of 39 |
| rerun, after commit 1 | 7 | 114 s | 73.5 s | **0.65x** | 16.6 s | 5.8 s | 6 of 7 |

### Keyed steps: outcomes and hit rate (all jobs)

| Step | phase | runs | miss | replayed | audit | other | replayed / unchanged-input | plain wall | quod wall | quod wall on replays |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `go build ./...` | pr | 40 | 40 | **0** | 0 | 0 | 0/0 | 225 s | 309 s | 0.0 s |
| `go build ./...` | merge | 40 | 40 | **0** | 0 | 0 | 0/0 | 96.7 s | 167 s | 0.0 s |
| `go build ./...` | rerun | 7 | 0 | **3** | 4 | 0 | 3/7 (43%) | 15.0 s | 17.8 s | 2.5 s |
| `go build ./...` | all | 87 | 80 | **3** | 4 | 0 | 3/7 (43%) | 337 s | 495 s | 2.5 s |
| `go vet ./...` | pr | 40 | 32 | **7** | 1 | 0 | 7/8 (88%) | 133 s | 209 s | 5.8 s |
| `go vet ./...` | merge | 40 | 6 | **28** | 6 | 0 | 28/34 (82%) | 41.9 s | 66.2 s | 22.1 s |
| `go vet ./...` | rerun | 7 | 0 | **6** | 1 | 0 | 6/7 (86%) | 6.3 s | 7.8 s | 4.7 s |
| `go vet ./...` | all | 87 | 38 | **41** | 8 | 0 | 41/49 (84%) | 181 s | 283 s | 32.5 s |
| `go test ./...` | pr | 40 | 35 | **3** | 2 | 0 | 3/5 (60%) | 1,069 s | 1,149 s | 2.7 s |
| `go test ./...` | merge | 40 | 35 | **3** | 2 | 0 | 3/5 (60%) | 537 s | 439 s | 2.5 s |
| `go test ./...` | rerun | 7 | 0 | **6** | 1 | 0 | 6/7 (86%) | 91.0 s | 45.7 s | 4.9 s |
| `go test ./...` | all | 87 | 70 | **12** | 5 | 0 | 12/17 (71%) | 1,697 s | 1,634 s | 10.0 s |

All keyed steps: 261 runs, 188 misses, **56 replayed of 73 unchanged-input runs (77%)**, 17 audits; replayed / all keyed runs 21%.

### go test replays against plain's go test on the same job

| # | phase | commit | quod replay | plain go test |
| --- | --- | --- | --- | --- |
| 2 | rerun | `5b0cc54248` | 0.7 s | 10.0 s |
| 5 | merge | `ad27e286ab` | 0.9 s | 10.7 s |
| 8 | rerun | `ad4a6456e1` | 0.8 s | 10.7 s |
| 13 | rerun | `d8474903cf` | 0.8 s | 14.0 s |
| 20 | rerun | `547bf81aa5` | 0.8 s | 13.2 s |
| 23 | pr | `9871098fbf` | 0.9 s | 13.5 s |
| 27 | rerun | `64d175234b` | 0.8 s | 13.5 s |
| 28 | rerun | `059f4975b5` | 0.9 s | 16.2 s |
| 32 | pr | `ca436970d5` | 0.9 s | 12.1 s |
| 32 | merge | `9be5608274` | 0.8 s | 13.0 s |
| 34 | pr | `76d03ad1a4` | 0.9 s | 12.6 s |
| 34 | merge | `93d24ce9ff` | 0.8 s | 12.6 s |

### Where the difference goes (Quod - plain, seconds, jobs after commit 1)

| Step | Quod - plain | exec - plain | Quod around exec | on replays | on audits | on misses, heavy commits (plain go test > 30 s) | on misses, other commits |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `go mod download` | +25 | +25 | +0 | +0 | +0 | +4 | +21 |
| `go build ./...` | +149 | +21 | +128 | -4 | +7 | +27 | +119 |
| `go vet ./...` | +96 | +12 | +85 | -4 | +16 | +31 | +54 |
| `go test ./...` | -72 | -171 | +100 | -142 | +149 | +116 | -195 |
| plain's checkout | -15 | | | | | | |
| **job** | **+184** | -114 | +313 | -150 | +172 | +178 | -1 |

### Per job

| # | phase | commit | plain job | quod job | quod - plain | build | vet | test | plain test |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | pr | `b1c8c0ae5a` | 128.2s | 150.2s | +22.0s | miss 48.9s | miss 18.0s | miss 80.4s | 67.7s |
| 1 | merge | `b1c8c0ae5a` | 13.0s | 12.8s | -0.3s | miss 3.7s | audit 3.2s | miss 5.5s | 9.9s |
| 2 | pr | `5b0cc54248` | 13.1s | 48.2s | +35.0s | miss 3.9s | audit 3.0s | audit 40.9s | 9.8s |
| 2 | merge | `5b0cc54248` | 13.6s | 45.5s | +32.0s | miss 3.6s | **HIT** 0.7s | audit 40.9s | 10.4s |
| 2 | rerun | `5b0cc54248` | 13.1s | 8.0s | -5.2s | audit 3.8s | audit 3.1s | **HIT** 0.7s | 10.0s |
| 3 | pr | `f3e2147f24` | 16.6s | 19.2s | +2.6s | miss 4.5s | miss 3.4s | miss 11.0s | 12.4s |
| 3 | merge | `ed40e856ff` | 14.0s | 15.0s | +1.1s | miss 3.9s | miss 3.0s | miss 7.8s | 10.7s |
| 4 | pr | `5cfe30624f` | 14.1s | 13.6s | -0.6s | miss 3.9s | miss 3.0s | miss 6.4s | 10.8s |
| 4 | merge | `5cfe30624f` | 13.9s | 11.1s | -2.8s | miss 3.6s | **HIT** 0.7s | miss 6.5s | 10.6s |
| 5 | pr | `985fbb2990` | 14.2s | 47.2s | +33.1s | miss 4.0s | **HIT** 0.8s | audit 42.2s | 10.7s |
| 5 | merge | `ad27e286ab` | 14.1s | 8.4s | -5.7s | miss 4.0s | audit 3.2s | **HIT** 0.9s | 10.7s |
| 6 | pr | `401854ece8` | 17.5s | 20.4s | +3.0s | miss 5.3s | miss 3.4s | miss 11.4s | 13.2s |
| 6 | merge | `401854ece8` | 13.0s | 10.4s | -2.6s | miss 3.7s | **HIT** 0.8s | miss 5.7s | 9.9s |
| 7 | pr | `350fa040f8` | 80.1s | 96.2s | +16.1s | miss 13.8s | miss 11.0s | miss 70.8s | 60.3s |
| 7 | merge | `1531837d85` | 16.9s | 19.5s | +2.5s | miss 4.4s | miss 3.4s | miss 11.4s | 13.1s |
| 8 | pr | `f2ef40d80d` | 76.7s | 92.6s | +15.9s | miss 13.2s | miss 10.4s | miss 68.4s | 57.8s |
| 8 | merge | `ad4a6456e1` | 14.3s | 12.0s | -2.3s | miss 4.0s | **HIT** 0.8s | miss 7.0s | 10.9s |
| 8 | rerun | `ad4a6456e1` | 13.9s | 5.5s | -8.4s | audit 3.7s | **HIT** 0.7s | **HIT** 0.8s | 10.7s |
| 9 | pr | `b593e2156a` | 76.3s | 91.9s | +15.6s | miss 12.1s | miss 10.5s | miss 69.0s | 57.5s |
| 9 | merge | `4f168ce66d` | 16.4s | 18.0s | +1.6s | miss 4.0s | audit 3.1s | miss 10.6s | 13.0s |
| 10 | pr | `98ea0f41da` | 21.9s | 24.9s | +3.0s | miss 4.7s | miss 3.8s | miss 16.1s | 17.2s |
| 10 | merge | `a81663f7d8` | 17.0s | 16.6s | -0.4s | miss 3.9s | **HIT** 0.7s | miss 11.7s | 13.5s |
| 10 | rerun | `a81663f7d8` | 16.6s | 42.7s | +26.1s | **HIT** 0.8s | **HIT** 0.8s | audit 40.9s | 13.3s |
| 11 | pr | `c1a3f11840` | 18.3s | 16.0s | -2.3s | miss 4.1s | miss 3.3s | miss 8.3s | 14.5s |
| 11 | merge | `b7887068cb` | 17.0s | 11.8s | -5.2s | miss 4.1s | **HIT** 0.8s | miss 6.6s | 13.6s |
| 12 | pr | `ea4f442b3e` | 18.8s | 17.2s | -1.5s | miss 4.3s | miss 3.4s | miss 9.2s | 14.8s |
| 12 | merge | `bd0b0e085f` | 17.3s | 11.8s | -5.5s | miss 4.1s | **HIT** 0.8s | miss 6.6s | 13.9s |
| 13 | pr | `9b2838de96` | 20.1s | 19.0s | -1.1s | miss 4.5s | miss 3.6s | miss 10.6s | 15.9s |
| 13 | merge | `d8474903cf` | 17.4s | 11.8s | -5.6s | miss 4.1s | **HIT** 0.8s | miss 6.7s | 14.0s |
| 13 | rerun | `d8474903cf` | 17.3s | 5.8s | -11.5s | audit 3.9s | **HIT** 0.8s | **HIT** 0.8s | 14.0s |
| 14 | pr | `c9d988a6f9` | 17.3s | 14.0s | -3.3s | miss 4.1s | miss 3.1s | miss 6.5s | 13.8s |
| 14 | merge | `316e7a80fb` | 17.3s | 11.6s | -5.7s | miss 4.0s | **HIT** 0.8s | miss 6.5s | 13.9s |
| 15 | pr | `153adc66ee` | 78.5s | 92.8s | +14.3s | miss 12.4s | miss 10.5s | miss 69.6s | 59.4s |
| 15 | merge | `4a289a9e19` | 14.1s | 14.6s | +0.5s | miss 4.2s | audit 3.2s | miss 6.9s | 10.7s |
| 16 | pr | `ee01699da7` | 19.5s | 21.9s | +2.4s | miss 5.0s | miss 3.8s | miss 12.8s | 14.6s |
| 16 | merge | `e62e84ca88` | 14.7s | 12.7s | -1.9s | miss 4.0s | **HIT** 0.8s | miss 7.7s | 11.3s |
| 17 | pr | `6897f68d23` | 74.3s | 90.0s | +15.8s | miss 12.1s | miss 10.4s | miss 67.2s | 56.2s |
| 17 | merge | `56682b6faf` | 16.9s | 16.9s | +0.1s | miss 4.1s | **HIT** 0.8s | miss 11.7s | 13.4s |
| 18 | pr | `0452816948` | 21.1s | 24.6s | +3.4s | miss 4.7s | miss 3.7s | miss 15.9s | 16.9s |
| 18 | merge | `abf0e0c25a` | 16.4s | 16.9s | +0.5s | miss 4.1s | **HIT** 0.8s | miss 11.7s | 13.0s |
| 19 | pr | `c1134186b0` | 75.1s | 90.8s | +15.7s | miss 12.1s | miss 10.2s | miss 68.1s | 56.8s |
| 19 | merge | `6314d776e1` | 16.9s | 17.2s | +0.3s | miss 4.2s | **HIT** 0.8s | miss 12.0s | 13.4s |
| 20 | pr | `21660dd773` | 48.8s | 59.5s | +10.7s | miss 8.4s | miss 7.0s | miss 43.8s | 37.5s |
| 20 | merge | `547bf81aa5` | 16.6s | 15.9s | -0.7s | miss 4.0s | **HIT** 0.8s | miss 10.7s | 13.2s |
| 20 | rerun | `547bf81aa5` | 16.4s | 2.7s | -13.7s | **HIT** 0.8s | **HIT** 0.8s | **HIT** 0.8s | 13.2s |
| 21 | pr | `a1d1667985` | 41.6s | 48.0s | +6.3s | miss 5.4s | miss 5.7s | miss 36.6s | 34.5s |
| 21 | merge | `ae7b971597` | 16.8s | 16.4s | -0.4s | miss 4.2s | **HIT** 0.8s | miss 11.1s | 13.3s |
| 22 | pr | `6ec1ba5454` | 21.6s | 25.9s | +4.4s | miss 5.9s | miss 3.8s | miss 15.9s | 16.9s |
| 22 | merge | `99750e1576` | 16.5s | 16.5s | +0.1s | miss 4.1s | **HIT** 0.8s | miss 11.2s | 13.0s |
| 23 | pr | `9871098fbf` | 17.0s | 6.4s | -10.6s | miss 4.3s | **HIT** 0.8s | **HIT** 0.9s | 13.5s |
| 23 | merge | `7948e8056e` | 16.5s | 46.9s | +30.4s | miss 4.2s | **HIT** 0.8s | audit 41.5s | 13.1s |
| 24 | pr | `c61327f400` | 69.3s | 86.9s | +17.6s | miss 13.2s | miss 9.8s | miss 63.1s | 51.1s |
| 24 | merge | `5056c0a8ef` | 18.7s | 16.5s | -2.3s | miss 4.3s | miss 3.2s | miss 8.6s | 15.1s |
| 25 | pr | `68987c3f9c` | 22.1s | 27.9s | +5.7s | miss 7.2s | miss 4.1s | miss 16.2s | 15.8s |
| 25 | merge | `ed84a97a55` | 23.7s | 26.0s | +2.4s | miss 5.3s | miss 4.1s | miss 16.3s | 18.2s |
| 26 | pr | `e87ed4c751` | 16.0s | 11.4s | -4.6s | miss 4.3s | **HIT** 0.9s | miss 6.0s | 12.5s |
| 26 | merge | `e87ed4c751` | 16.0s | 13.4s | -2.6s | miss 3.9s | audit 3.3s | miss 6.0s | 12.7s |
| 27 | pr | `da42d39540` | 18.3s | 17.2s | -1.1s | miss 4.3s | miss 3.5s | miss 9.0s | 14.3s |
| 27 | merge | `64d175234b` | 16.7s | 11.9s | -4.8s | miss 4.2s | **HIT** 0.8s | miss 6.6s | 13.4s |
| 27 | rerun | `64d175234b` | 16.9s | 2.8s | -14.1s | **HIT** 0.9s | **HIT** 0.8s | **HIT** 0.8s | 13.5s |
| 28 | pr | `fa812ef2b1` | 28.1s | 31.4s | +3.3s | miss 5.7s | miss 4.6s | miss 20.7s | 21.7s |
| 28 | merge | `059f4975b5` | 19.7s | 16.4s | -3.3s | miss 4.1s | **HIT** 0.8s | miss 11.2s | 16.2s |
| 28 | rerun | `059f4975b5` | 19.4s | 6.0s | -13.4s | audit 4.0s | **HIT** 0.8s | **HIT** 0.9s | 16.2s |
| 29 | pr | `17ac598e6a` | 46.9s | 51.6s | +4.7s | miss 6.6s | miss 6.6s | miss 38.2s | 37.7s |
| 29 | merge | `4f2cfe92e3` | 15.3s | 12.3s | -3.0s | miss 4.3s | **HIT** 0.9s | miss 6.9s | 12.0s |
| 30 | pr | `ab3efc24f8` | 18.2s | 17.5s | -0.7s | miss 4.3s | miss 3.5s | miss 9.4s | 14.3s |
| 30 | merge | `63837fb778` | 16.1s | 12.6s | -3.5s | miss 4.2s | **HIT** 0.8s | miss 7.3s | 12.7s |
| 31 | pr | `6eb83b0ea9` | 75.4s | 92.1s | +16.7s | miss 12.2s | miss 10.6s | miss 68.9s | 57.1s |
| 31 | merge | `0186b810be` | 49.4s | 59.9s | +10.5s | miss 7.4s | miss 7.1s | miss 45.1s | 38.8s |
| 32 | pr | `ca436970d5` | 15.5s | 6.0s | -9.5s | miss 4.0s | **HIT** 0.9s | **HIT** 0.9s | 12.1s |
| 32 | merge | `9be5608274` | 16.4s | 5.9s | -10.5s | miss 4.0s | **HIT** 0.8s | **HIT** 0.8s | 13.0s |
| 33 | pr | `b07f9b4016` | 16.1s | 14.8s | -1.2s | miss 4.0s | **HIT** 0.8s | miss 9.8s | 12.8s |
| 33 | merge | `5687d4ffe8` | 16.1s | 14.9s | -1.2s | miss 4.0s | **HIT** 0.8s | miss 9.8s | 12.7s |
| 34 | pr | `76d03ad1a4` | 16.0s | 5.9s | -10.0s | miss 3.9s | **HIT** 0.8s | **HIT** 0.9s | 12.6s |
| 34 | merge | `93d24ce9ff` | 15.9s | 5.9s | -10.1s | miss 4.0s | **HIT** 0.8s | **HIT** 0.8s | 12.6s |
| 35 | pr | `4a64788a49` | 16.3s | 18.0s | +1.7s | miss 4.1s | miss 3.3s | miss 10.2s | 12.9s |
| 35 | merge | `674516ff39` | 16.0s | 15.1s | -0.9s | miss 4.0s | **HIT** 0.8s | miss 10.0s | 12.7s |
| 36 | pr | `87c7ab51a9` | 15.8s | 12.2s | -3.6s | miss 4.0s | **HIT** 0.9s | miss 7.0s | 12.5s |
| 36 | merge | `f7a06a18a9` | 16.1s | 15.1s | -1.0s | miss 4.1s | **HIT** 0.8s | miss 9.9s | 12.8s |
| 37 | pr | `b17c1554e6` | 18.1s | 21.2s | +3.1s | miss 4.2s | miss 3.6s | miss 13.0s | 14.1s |
| 37 | merge | `8553724d15` | 16.9s | 15.7s | -1.1s | miss 4.0s | **HIT** 0.8s | miss 10.6s | 13.5s |
| 38 | pr | `0c5e7e910f` | 30.9s | 36.5s | +5.7s | miss 10.2s | miss 5.0s | miss 20.6s | 20.4s |
| 38 | merge | `30eb70ca63` | 15.6s | 14.0s | -1.6s | miss 4.1s | **HIT** 0.8s | miss 8.7s | 12.2s |
| 39 | pr | `934db7e662` | 71.7s | 87.9s | +16.1s | miss 11.5s | miss 10.1s | miss 66.0s | 54.3s |
| 39 | merge | `443baeba05` | 21.3s | 25.6s | +4.3s | miss 5.3s | miss 4.2s | miss 15.8s | 15.9s |
| 40 | pr | `f59a50225f` | 14.1s | 14.3s | +0.2s | miss 4.2s | miss 3.2s | miss 6.6s | 10.7s |
| 40 | merge | `9490d204a2` | 13.2s | 14.1s | +0.9s | miss 4.2s | audit 3.3s | miss 6.3s | 9.9s |

## Misses, lookup, sizes (`extra.py`)

### History

Misses by first changed input:

- `go build ./...`: .git (VCS stamping) 39, (none) 1
- `go vet ./...`: Go source file or a directory listing 22, module cache (dependency bump) 6, test file or testdata 3, (none) 1
- `go test ./...`: Go source file or a directory listing 23, module cache (dependency bump) 5, test file or testdata 4, (none) 1, .github/... 1, docs/... 1

go test lookup ms: run 2 88, median 74, last 81 (candidates at the last run: 34)
go test quod peak RSS MB: first 70, median 73.0, last 82
replays: 6, wall median 0.78 s (min 0.77, max 0.82)
sizes after the last commit (MB): state 8,426, cas 1,021, warm 6,460, deps 388, ledger 556, plain_gocache 4,862

### Traffic

Misses by first changed input:

- `go build ./...`: .git (VCS stamping) 79, (none) 1
- `go vet ./...`: Go source file or a directory listing 25, module cache (dependency bump) 7, test file or testdata 5, (none) 1
- `go test ./...`: .git (VCS stamping) 68, (none) 1, module cache (dependency bump) 1

go test lookup ms: run 2 31, median 118, last 174 (candidates at the last run: 69)
replays: 56, wall median 0.80 s (min 0.73, max 0.92)
ledger at the end: 978 MB


## Correctness

**No false hit. Nothing diverged, nothing was marked suspect.**

- **Ledgers** (`0e-combined-x86/ledgers.txt`). History: 107 misses, 7
  audits, 6 replays, 40 network runs (step A). Traffic: 188 misses, 17
  audits, 56 replays, 87 network runs. Each run has one key per step, and
  every key is `proven`. None is quarantined, every key has 0 divergences,
  and no run is marked `suspect`. The audits table holds 24 rows, all
  `diverged = 0`. `go vet` and `go build` compared byte-identical. `go test`
  compared identical up to normalized volatile fields (durations), as every
  `go test` does.
- **Audit schedule.** History: shadow audits at commits 2 and 5 for vet and
  for test, then decay audits at vet 26 and 34, and test 32. Traffic: 6
  shadow and 11 decay. A `go test` audit runs the whole suite with Go's test
  cache off (receipt: "shadow audit: would-be hit 1 re-executed"; 0 lines
  `(cached)`, 258 packages `ok`).
- **Spot check: every replay, not a sample** (`spotcheck.py`, log
  `spotcheck.log`). Each of the 62 replays (6 history, 56 traffic) was re-run
  plainly as uid 1001 on a fresh checkout of the same commit, with
  `go test` as `go test -count=1` so that every package executes. Exit codes
  and stdout were compared; `go test` lines were sorted, with durations and
  `(cached)` removed. **62 of 62 agree**; for the 14 `go test` replays (2 history, 12 traffic) all 319-321 result lines are equal. The plain re-runs took 0.8-69 s each. For `go vet` and `go build`, stdout is empty on success, so for those the
  check is the exit code (0b noted the same weakness).
- Every run of both series exited 0. On every job (40 history, 87
  traffic), Quod's `go test` output lists the same packages as plain-warm's
  on the same job, with no `FAIL` line (`pkgcheck.py`).
- Step A (`go mod download`, network) worked with Ubuntu's symlinked
  `/etc/resolv.conf` left in place (0b's F1 is fixed on main): 127 network
  runs, 0 failures.

## Where the remaining loss goes (phases)

The tables above split Quod's extra time per step into the execution
against plain's whole step (`exec - plain`) and Quod's own time around the
execution (`wall - exec`). History, commits 2-40: +125 s is execution (the
audits' full suites +~90 s, heavy commits' observed `go test` ~+110 s,
offset by faster `go test` misses elsewhere). +151 s is Quod's own time: 117
keyed step runs at a median 0.7-0.9 s each (setup 0.25 s, import 0.36 s, or
0.60 s for build, lookup 0.06-0.08 s, mount 0.12 s, record 0.24 s; replays
0.78 s in total). Traffic, after commit 1: Quod's own time is +313 s over 255
keyed runs, and the execution is -114 s (its `go test` beats plain's, and
replays are counted at zero execution).

Compared with 0c:

| median ms, `go test` execution | setup | import | lookup | record | wall - total_ms |
| --- | --- | --- | --- | --- | --- |
| 0c (exp/never-worse) | 240 | 350 | 60-80 | 240 | |
| 0e (main) | 252 | 356 | 72 | 239 | 710 |

The lookup index holds up: `go test`'s lookup is 81 ms at commit 40 of the
history (34 candidates) and 174 ms at the end of traffic (69 candidates).
0b measured 1.03 s at 33. Quod's peak RSS on `go test` is 70-82 MB (0b: 250
MB).

## Findings

- **F1 (cost, new with trust v2): audits are the largest single loss.** A
  `go test` audit is a full `-count=1`-style run: 40-41 s against 10-13 s
  for plain-warm and 5.6-9.5 s for 0c's trust-v1 proving runs on the same
  commits. 3 audits in the history cost +99 s, 5 in traffic +149 s. That is
  more than the 2 (history) or 12 (traffic) `go test` replays saved.
- **F2: `go test` reads `.git/HEAD`.** In traffic, 68 of its 70 misses name
  `.git/HEAD` as the first changed input: the PR job is on branch `pr`, the
  merge job on `main`. This blocks the PR-to-merge replay of `go test` that
  trust v2 targets. On GitHub Actions, the PR job checks out a detached merge
  ref and the push job a branch, so `.git/HEAD` differs there too. Which
  cli/cli test or go command reads it was not traced.
- **F3: `go test` now reads `docs/install_linux.md`.** It did not in 0b. 0c
  saw it too (commit 26 missed on it in both). So the docs-only commit 26 no
  longer has unchanged `go test` inputs. Sound, but an over-record for a
  docs-only change; not traced.
- **F4: the ledger grows ~11 MB per job** (556 MB after the history's 40
  jobs, 978 MB after traffic's 87). The store was 8.4 GB after the history,
  6.5 GB of it the warm Go cache. Plain's own GOCACHE was 4.9 GB. Nothing is
  evicted on either side.
- No mounts were left behind by any run (`mount` after the chain shows only
  the system's fusectl).

## Methodology

**Host.** A fresh CCX23 provisioned with 0b's `install.sh` (unchanged: Go
1.27.3, sha256-checked; Rust 1.98.0; bubblewrap 0.11.1; fuse3 3.18.2) and
`setup.sh`: SSH keys only, ufw default-deny with SSH allowed, user `runner`
(uid 1001), `git config --system safe.directory '*'`, FUSE `enable_uring=Y`.
`setup.sh` also disabled and masked every systemd timer that runs apt or
rewrites /etc, or that competes for CPU: apt-daily, apt-daily-upgrade,
unattended-upgrades, update-notifier, motd-news, man-db, dpkg-db-backup,
logrotate, sysstat, e2scrub, xfs_scrub, fstrim and tmpfiles-clean. Snap
refreshes were held. `/etc/resolv.conf` was left as Ubuntu's symlink. All
of this was done before the first measured run, and /etc did not change
afterwards: the host fingerprint was `b3:238f0aba` on every run. No
credentials were put on the box; the code was shipped with
`git archive origin/main | ssh tar` and built there (`cargo build --release
-p quod-cli`, 79 s).

**(1) History**: `history.py`, 0c's driver with only `OUT` and `QUOD`
changed, run with `N=40 COLD_EVERY=0`. For each of the 40 first-parent
commits ending at `9490d204a2ba`, it runs plain-warm and Quod in alternating
order (even commits plain first):

- *Plain-warm*: a fresh `git clone --shared` checkout at a constant path
  (new mtimes, as `actions/checkout` gives), then `go mod download`, then
  the three steps as uid 1001 with Quod's Go profile env and a fresh HOME.
  One GOCACHE and GOMODCACHE persist across all commits. This is a perfect
  `actions/cache`, with no restore or save time.
- *Quod*: one persistent state dir, `git checkout -f` in place (not timed),
  step A `quod run --allow-net`, then `quod run --mode replay --warm` for the
  three steps, with main's defaults (proofs 1, shadow 2, audit 5%).

**(2) Traffic**: `traffic.py`, trust-v2's driver with a new `MODE=both`.
The job sequence is trust-v2's: each commit's PR head first (the merge's
second parent, or the commit itself on a `pr` branch), then the merge on
`main`, then a re-run of the merge job with probability 0.10 (seed 42). That
gives 87 jobs: 40 PR, 40 merge, 7 re-runs. For every job, a plain-warm job
(as in (1); one GOCACHE across all 87 jobs) and the Quod job run back to
back, in alternating order. Quod uses one state dir for the whole series,
separate from (1)'s.

Job time: plain is checkout + step A + 3 steps. Quod is step A + 3 steps,
since its checkout is an in-place `git checkout` and is not timed (0b's
accounting). Commit 1 is reported but excluded from the history totals,
because every cache is empty there. In traffic, the "all" row includes it.

`chain.sh` ran build, history, traffic, the ledger dumps and the spot
check, one after another. `analyze.py` produces every table below, and
`extra.py` the miss reasons, lookup and size figures.

## Caveats

- One box, one corpus, 4 vCPUs. `go test ./...` takes ~110 s cold here.
  The ratios are what carry over, not the absolute times.
- The plain-warm baseline is generous: a perfect cache with no restore cost
  and never evicted. A real `actions/cache` restore of a 1-5 GB GOCACHE costs
  tens of seconds per job. Quod's warm cache is local here too; on a fresh
  runner it would need the same restore.
- Re-runs are 7 of 87 jobs at a 10% rate. Re-runs are where Quod wins most,
  so the traffic total depends on that rate.
- The `.git/HEAD` effect (F2) depends on how the driver names branches. A CI
  that checks out the merge job on the same ref name as the PR job would
  replay more `go test` merges.
- No run was dropped and no series was restarted. Everything is in
  `history-rows.jsonl` and `traffic-rows.jsonl`.

