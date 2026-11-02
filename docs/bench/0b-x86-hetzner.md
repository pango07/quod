# 0b: first x86 numbers, cli/cli on a dedicated Hetzner box

Quod main `5d296c4`, cli/cli (pinned `9490d204a2ba` and the 39 first-parent commits before it), Hetzner CCX23 (4 dedicated AMD EPYC-Milan vCPUs, 16 GB, NVMe, Ubuntu 26.04.1, kernel 7.0.0-30), FUSE over io_uring on. Measured 2026-11-01 20:44 to 2026-11-02 00:11 UTC. Raw logs: `docs/bench/0b-x86-hetzner/`.

## Summary

**Short answer: not groundbreaking yet. On a real commit history, Quod made CI slower than a well-cached plain runner, not faster.**

What we did. We replayed 40 real, consecutive cli/cli commits, the way a CI system would see them, and ran `go build ./...`, `go vet ./...` and `go test ./...` on each one three ways:

- **plain, warm cache:** Go's own build and test caches carried over from the previous commit, as a good `actions/cache` setup gives you;
- **plain, cold:** a fresh runner with no cache at all (sampled on 8 of the 40 commits);
- **Quod:** a persistent store and ledger, as Quod Cloud would keep, with the trust rules as shipped (a result must be produced twice before it is replayed; 5% of replays are re-run and checked).

What happened (39 commits, the first one dropped because every cache starts empty there):

| | Total | Median per commit |
| --- | --- | --- |
| Plain, warm Go cache | **1,230 s** (20.5 min) | 18.3 s |
| Quod | **2,341 s** (39.0 min) | 51.7 s |
| Plain, no cache (8 sampled commits) | ~4,810 s (80 min, extrapolated) | 123.4 s |

- **Against a warm Go cache, Quod was 1.9x slower in total and slower on every one of the 39 commits** (2.8x the median commit). That is the comparison a competitor, or a customer with a decent `actions/cache` setup, will make.
- **Against a runner with no cache, Quod was about 2x faster.** But that is because Quod keeps Go's build cache between runs itself, not because of replay. Its replays saved essentially nothing here.
- **Replays were almost nonexistent:** 2 of 120 step runs replayed (both `go vet`), and each replay took 1.9 s, as long as the run it replaced. `go build` never replayed (0/40), `go test` never replayed (0/40).
- **Correctness held.** The full acceptance suite passed on this host with **0 false hits** over 29,938 mutated inputs. Every proof comparison (12 in the history, 3 in the replay series) was identical, and every spot-checked replay (the 2 history replays, one of each step in the replay series) matched a fresh plain run. Nothing diverged.

Why, in plain terms:

1. **Go already does most of what Quod does, at a finer grain.** Go's test cache skips each *package* whose inputs did not change. Quod replays a whole *step* only when *nothing* it read changed. A typical commit changes one file, so Go re-runs a few packages' tests (14 s median) while Quod has to re-run the whole step.
2. **When Quod misses, it re-runs every test.** To stay safe, Quod turns Go's test cache off inside its sandbox (`-count=1`: the test cache trusts file timestamps, which Quod cannot vouch for). So every Quod miss runs every package's tests (41 s median) even when Go would have re-run only a handful. This single choice is most of the gap.
3. **"Proven twice" rarely happens on a linear history.** A step replays only on the *third* time it sees the same inputs. Only 8 of 39 commits left `go vet`'s inputs untouched and 6 left `go test`'s (docs- or CI-only changes), and only once (commits 32-34) did three in a row share inputs: that is where both replays are.
4. **`go build` can never replay across commits.** It stamps the commit hash into binaries by reading `.git`, so `.git` is a real input and changes on every commit. Quod is right to miss; it just means the step is unreplayable.

Where Quod *is* fast: an exact re-run of unchanged inputs (a CI re-run, the same commit pushed to two branches, a merge commit identical to its PR head). There the whole job replays in **4.6 s against 13.3 s with a warm Go cache (2.9x) and 124.5 s with none (27x)**. Nearly all of the win is `go test` (1.3 s vs 10.1 s: Go's test cache still re-runs ~40 cli/cli test packages on an identical tree); `go build` gains 0.8 s and `go vet` is 0.4 s slower than Go's cache. See "Replay speed". That traffic exists in real CI but was rare in this history.

Two more numbers:

- **First-run overhead (a miss with no cache) is +3.2%** median and paired on `go test ./...` (108.4 s plain, 111.9 s observed). That is low because the step is long on 4 vCPUs; in absolute terms it is 3.5 s, 2.3x the 1.5 s measured on the 18-core arm64 box.
- **Quod's own cost** is small in CPU (~2 CPU-s per step) but grows with history: lookup time and memory are linear in the number of earlier runs of a step (1.0 s and 250 MB after 33), and the ledger grew to 566 MB in 40 commits (findings F3-F5).

What would change the picture (not measured here; each is a design decision):

- letting Go's own test cache work inside Quod on misses (keyed by content instead of timestamps), so a miss costs what a warm plain run costs;
- splitting `go test ./...` per package so a one-file change replays most of it (exp/split tried per-test-binary memoization on an earlier observer and found it a net loss; worth re-measuring);
- counting a match from another branch or PR run as a proof, so real CI traffic (PR run, then merge) proves results without three identical commits in a row.

## Correctness on this host (before any measurement)

`scripts/acceptance.sh go /work/cli-acc` on main `5d296c4`, host mode, ring transport (`fuse-uring`, no fallback), log `0b-x86-hetzner/acceptance-go.log`:

- A1-A6 pass; P1 (second execution identical up to go test durations) passes.
- **A7: mutated 23,983 f, 5,335 d, 620 a entries of the 29,938-entry manifest in 765.9 s: 0 false hits.**
- A7b 10/10: unread files mutated, replay agrees with a fresh execution, both read the same paths.
- W1-W4 pass (warm agrees with cold; W4: 29,849 entries with a full and an empty cache, 0 hidden).
- Trust suite T1-T6, T8-T11 pass; T7 (injected observer panic aborts the step) passes on the classic and the ring transport.
- Hooks acceptance (`QUOD_INTEGRATION=1 cargo test --release -p quod-cli --test hooks_acceptance`, tiny Go module): 3 passed (`hooks-acceptance.log`).

Two earlier attempts are kept as evidence, not results: attempt 1 (`acceptance-go-attempt1-dns.log`) failed at step A on DNS (finding F1); attempt 2 (`acceptance-go-attempt2-host-changed.log`) failed A3 because I created the `runner` user and wrote `/etc/gitconfig` while it ran. The host fingerprint hashes /etc, so A3 ran under a new key and correctly missed (b3:718ac0f6 -> b3:09faeb49 mid-run). Not a Quod defect; the fingerprint did its job. The merge gate's lint (clippy and the workspace unit tests) was not run: no code changed, and it costs ~15 min of the 4 vCPUs.

## 1. Real commit history (the headline)

40 consecutive first-parent commits of cli/cli ending at the pinned `9490d204a2ba` (2026-10-19 to 2026-10-30), each run plain with a warm Go cache, through Quod with a persistent store and ledger, and (every 5th commit) plain with no cache. Commit 1 is the warm-up (all caches empty) and is excluded from totals and medians.

### Totals (commits 2-40)

| | Total | Median job | p90 job | Min-max job | vs plain-warm |
| --- | --- | --- | --- | --- | --- |
| Plain, warm Go cache | 1,230 s | 18.3 s | 74.3 s | 12.9-79.4 s | |
| **Quod (replay mode, proofs 2, audit 5%, warm)** | **2,341 s** | **51.7 s** | 86.5 s | 48.3-89.0 s | **+90% total; median commit 2.78x (1.12-3.77x); slower on 39/39** |
| Plain, no cache (commits 5, 10, ..., 40) | ~4,810 s (123.4 s x 39) | 123.4 s | | 122.7-124.5 s | +291% |

Time saved by Quod: **-1,111 s against a warm Go cache** (it cost 18.5 min more over 39 commits); **~2,470 s against no cache** (51%), all of it from Quod's own warm Go build cache, not from replays (the two replays saved 0.0 s each by their receipts).

Per step, median and total over commits 2-40 (cold: 8 commits):

| Step | Plain-warm median / total | Quod median / total | Plain-cold median | Quod hits |
| --- | --- | --- | --- | --- |
| go build ./... | 2.7 s / 170 s | 5.4 s / 282 s | 40.4 s | **0/40** |
| go vet ./... | 1.2 s / 108 s | 4.3 s / 226 s | 15.4 s | **2/40** (commits 33, 34; 1.9 s each) |
| go test ./... | 14.4 s / 943 s | 41.4 s / 1,804 s | 67.4 s | **0/40** |
| step A (go mod download) | 0.0 s / 5 s | 0.7 s / 34 s | 0.0 s | never memoized |

Where a Quod miss's time goes (median over misses, from the run event): `go test` setup 0.64 s, import 0.44 s, mount 0.11 s, **execution 39.2 s**, output diff 0.07 s, ledger 0.21 s, plus 1.1-2.1 s outside the run's phases (F3). The execution is the whole suite (`-count=1`); plain-warm's 14.4 s is Go re-running only the packages a commit affected.

### Why each miss missed

From each run event's `first_mismatch` (the first changed input of the closest earlier manifest, the path the receipt names):

| Reason | go build | go vet | go test |
| --- | --- | --- | --- |
| no manifest yet (commit 1) | 1 | 1 | 1 |
| `.git` changed (VCS stamping: `.git/index` 38, `.git/objects/pack` 1) | **39** | 0 | 0 |
| listing of `.github` (a file added or renamed there) | 0 | 0 | 2 |
| Go source file or a source directory's listing | 0 | 21 | 20 |
| test file or testdata | 0 | 3 | 4 |
| module cache (a dependency bump: `.quod/gomod/cache/download/...`) | 0 | 6 | 6 |
| `go.mod` | 0 | 1 | 1 |
| **inputs unchanged, but only the 2nd identical execution ("proving")** | 0 | **6** | **6** |
| replayed | 0 | 2 | 0 |

The "proving" rows are the docs- and CI-only commits: 2, 5, 23, 26, 32 and 36 for vet (which then replayed at 33 and 34), 2, 5, 23, 26, 32 and 34 for test. With proofs 1 (the matcher alone) they would have replayed: 8 vet and 6 test hits instead of 2 and 0. At ~40 s per `go test` that is ~4 min of 39 min; it would not change the conclusion. The two `go test` misses on `.github` (commits 33 and 36) changed only CI files, and added or renamed a file there; the `go test` manifest records the listing of `.github` and the receipt names it (which process lists it was not traced; `go vet`'s manifest does not have it, and vet proved on 36). Sound, but an over-record for a CI-only commit.

### Audits and trust

Ledger at the end (`audits`, `trust`, `runs` tables):

- `audits`: 12 rows, all phase `proof` (the second execution of a manifest compared with the first), **0 diverged**. No `audit`-phase rows: with 2 replays, the 5% schedule (every 20th replay of a key) never fired.
- `trust`: 2 keys `proven` (vet, test: 2 replays, 12 comparisons, 0 divergences), 1 `observing` (build: never proved, every manifest new).
- `runs`: 106 miss, 12 proving, 2 replayed, 40 net (step A).
- Spot-check (`spotcheck.py`; `spotcheck-history.log`, `spotcheck-replay.log`): each replayed step re-run plainly on a fresh checkout of the same commit (`go test` as `go test -count=1` so it really executes), exit code and stdout compared (go test lines sorted, durations and `(cached)` removed). Both history replays (`go vet` at 33 and 34): exit 0 and identical (empty) stdout, a weak check since vet prints nothing on success. From the replay series, `go build`, `go vet` and `go test` at the pinned commit: identical; for `go test`, all 321 lines equal. 5 of 5 agree.

**No false hit, no divergence.**

### Per commit

"job" = checkout + step A + three steps. Quod cells are the outcome and the step's wall time; the last column is `go test`'s first changed input.

| # | Commit | Subject | plain-warm job | plain-cold job | quod job | build | vet | test | test first changed input |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | `b1c8c0ae5a` | Merge pull request #14750 from cli/williammartin | 125.9s |  | 139.9s | miss 47.5s | miss 18.1s | miss 69.6s | `-` |
| 2 | `5b0cc54248` | chore(deps): bump the codeql-actions group with  | 12.9s |  | 48.4s | miss 4.3s | proving 3.6s | proving 39.8s | `-` |
| 3 | `ed40e856ff` | Merge pull request #14719 from cli/bagtoad/fix-h | 14.5s |  | 48.9s | miss 4.5s | miss 3.6s | miss 40.1s | `pkg/cmd/ruleset/view/view.go` |
| 4 | `5cfe30624f` | Bump Go to 1.27.3 (#14767) | 13.6s |  | 48.4s | miss 4.4s | miss 3.5s | miss 39.8s | `go.mod` |
| 5 | `ad27e286ab` | Merge pull request #14770 from cli/bagtoad/merge | 13.6s | 123.1s | 48.7s | miss 4.4s | proving 3.5s | proving 40.1s | `-` |
| 6 | `401854ece8` | chore(deps): bump github.com/gabriel-vasile/mime | 17.0s |  | 51.3s | miss 5.7s | miss 3.9s | miss 41.0s | `.quod/gomod/cache/download/github.com/gabriel-vasile/mimetype/@v` |
| 7 | `1531837d85` | Merge pull request #14148 from heaths/issue3718 | 79.4s |  | 89.0s | miss 14.1s | miss 11.3s | miss 62.6s | `.quod/gomod/cache/download/charm.land` |
| 8 | `ad4a6456e1` | Merge pull request #14450 from cli/babakks/refre | 74.7s |  | 87.0s | miss 13.4s | miss 10.8s | miss 61.9s | `.quod/gomod/cache/download/github.com/cli/oauth/@v` |
| 9 | `4f168ce66d` | Merge pull request #14451 from cli/babakks/refre | 74.3s |  | 85.8s | miss 12.5s | miss 10.7s | miss 61.8s | `api/http_client.go` |
| 10 | `a81663f7d8` | Merge pull request #14452 from cli/babakks/refre | 20.8s | 122.7s | 52.1s | miss 5.1s | miss 4.3s | miss 41.9s | `internal/authflow/flow.go` |
| 11 | `b7887068cb` | Merge pull request #14453 from cli/babakks/refre | 17.7s |  | 49.4s | miss 4.8s | miss 3.9s | miss 40.1s | `pkg/cmd/auth/token/token.go` |
| 12 | `bd0b0e085f` | Merge pull request #14454 from cli/babakks/refre | 18.3s |  | 50.0s | miss 4.8s | miss 4.0s | miss 40.4s | `pkg/cmd/auth/status/status.go` |
| 13 | `d8474903cf` | Merge pull request #14455 from cli/babakks/refre | 19.3s |  | 51.0s | miss 5.1s | miss 4.2s | miss 41.1s | `pkg/cmd/agent-task/capi/client.go` |
| 14 | `316e7a80fb` | Merge pull request #14456 from cli/babakks/refre | 16.4s |  | 49.3s | miss 4.7s | miss 3.7s | miss 40.2s | `script` |
| 15 | `4a289a9e19` | Merge pull request #14663 from cli/babakks/refre | 74.8s | 123.3s | 86.5s | miss 12.7s | miss 11.0s | miss 62.1s | `api/client_test.go` |
| 16 | `e62e84ca88` | Merge pull request #14665 from cli/babakks/refre | 19.0s |  | 53.1s | miss 5.7s | miss 4.4s | miss 42.3s | `api/http_client.go` |
| 17 | `56682b6faf` | Merge pull request #14672 from cli/babakks/refre | 73.6s |  | 86.4s | miss 12.6s | miss 11.1s | miss 62.0s | `internal/config/config.go` |
| 18 | `abf0e0c25a` | Merge pull request #14673 from cli/babakks/refre | 20.0s |  | 51.6s | miss 5.2s | miss 4.3s | miss 41.4s | `pkg/cmd/auth/auth.go` |
| 19 | `6314d776e1` | Merge pull request #14717 from cli/babakks/refre | 73.1s |  | 86.2s | miss 12.6s | miss 11.0s | miss 61.9s | `internal/config/config.go` |
| 20 | `547bf81aa5` | Merge pull request #14745 from cli/babakks/refre | 48.1s | 123.0s | 69.5s | miss 9.1s | miss 7.8s | miss 51.9s | `internal/text/text.go` |
| 21 | `ae7b971597` | Merge pull request #14752 from cli/babakks/refre | 40.4s |  | 63.6s | miss 6.1s | miss 6.6s | miss 50.2s | `internal` |
| 22 | `99750e1576` | Merge pull request #14768 from cli/babakks/refre | 21.7s |  | 54.3s | miss 6.8s | miss 4.6s | miss 42.0s | `.quod/gomod/cache/download/github.com/cli/oauth/@v` |
| 23 | `7948e8056e` | Merge pull request #14782 from cli/babakks/skill | 16.0s |  | 50.3s | miss 4.9s | proving 4.0s | proving 40.7s | `-` |
| 24 | `5056c0a8ef` | Merge pull request #14784 from cli/tommaso-moro- | 18.0s |  | 50.8s | miss 5.2s | miss 4.1s | miss 40.8s | `pkg/cmd/skills/install/install.go` |
| 25 | `ed84a97a55` | Merge pull request #14791 from cli/bdehamer-depe | 24.2s | 123.4s | 56.6s | miss 8.1s | miss 5.1s | miss 42.8s | `.quod/gomod/cache/download/github.com/digitorus/pkcs7/@v` |
| 26 | `e87ed4c751` | Document extrepo installation on Debian (#14796) | 15.7s |  | 50.6s | miss 5.2s | proving 4.1s | proving 40.6s | `-` |
| 27 | `64d175234b` | Merge pull request #14798 from cli/babakks/polis | 17.2s |  | 51.7s | miss 5.3s | miss 4.3s | miss 41.4s | `pkg/cmd/auth/login/login.go` |
| 28 | `059f4975b5` | Merge pull request #14804 from martinajir/attach | 27.5s |  | 57.9s | miss 6.7s | miss 5.5s | miss 45.0s | `internal/attachments/client.go` |
| 29 | `4f2cfe92e3` | Merge pull request #14805 from cli/bagtoad/attac | 46.1s |  | 68.8s | miss 7.8s | miss 7.7s | miss 52.7s | `internal/attachments/client.go` |
| 30 | `63837fb778` | Merge pull request #14815 from cli/babakks/auth- | 17.1s | 124.5s | 51.7s | miss 5.4s | miss 4.5s | miss 41.1s | `pkg/cmd/auth/status/status.go` |
| 31 | `0186b810be` | Merge pull request #14801 from tj-smith47/remove | 74.2s |  | 87.8s | miss 13.2s | miss 11.5s | miss 62.5s | `api` |
| 32 | `9be5608274` | Merge pull request #14824 from cli/dependabot/gi | 15.8s |  | 51.2s | miss 5.2s | proving 4.2s | proving 41.0s | `-` |
| 33 | `5687d4ffe8` | Merge pull request #14837 from cli/williammartin | 15.9s |  | 48.4s | miss 5.3s | HIT 1.9s | miss 40.5s | `.github` |
| 34 | `93d24ce9ff` | Only save Go caches from trunk (#14840) | 15.8s |  | 48.3s | miss 5.3s | HIT 1.9s | proving 40.4s | `-` |
| 35 | `674516ff39` | Merge pull request #14854 from bobrik/ivan/norma | 16.1s | 123.5s | 51.3s | miss 5.3s | miss 4.3s | miss 41.0s | `pkg/cmd/extension/command_test.go` |
| 36 | `f7a06a18a9` | Merge pull request #14825 from cli/bagtoad/ghsa- | 15.8s |  | 51.1s | miss 5.4s | proving 4.2s | miss 40.9s | `.github` |
| 37 | `8553724d15` | Merge pull request #14858 from cli/bagtoad/priva | 17.7s |  | 52.3s | miss 5.6s | miss 4.7s | miss 41.3s | `acceptance/testdata/pr/pr-checkout-with-url-from-fork.txtar` |
| 38 | `30eb70ca63` | Merge pull request #14878 from cli/dependabot/go | 29.6s |  | 63.4s | miss 11.4s | miss 6.3s | miss 44.2s | `.quod/gomod/cache/download/go.opentelemetry.io/otel/@v` |
| 39 | `443baeba05` | Merge pull request #14355 from cli/williammartin | 70.6s |  | 85.6s | miss 12.7s | miss 11.2s | miss 61.0s | `git` |
| 40 | `9490d204a2` | Merge pull request #14879 from cli/bagtoad/show- | 13.9s | 123.7s | 52.4s | miss 5.6s | miss 4.5s | miss 41.5s | `git/client_test.go` |

## 2. First-run overhead (`quod bench --interleave`)

`quod --state /work/bench/state bench --repo /work/bench/cli --runs 10 --no-hit-rate --interleave` on cli/cli `9490d204a2ba`, step `go test ./...`: plain (the same sandbox, uid and env on a fresh copy of the tree with empty caches, no FUSE or overlay) and observed (the whole `quod run` with a fresh state dir, so every run is a miss with an empty Go cache) alternate; round 1 dropped, 9 measured. Generated report: `0b-x86-hetzner/overhead-report.md`; log `overhead-bench.log`. 23:11-23:48 UTC, load 6.9-8.7 (the benchmark's own `go test` on 4 vCPUs).

| Series | Median | Max of 9 | Execution median | Runs (s) |
| --- | --- | --- | --- | --- |
| Plain | 108.35 s | 108.91 s | 108.35 s | 108.1, 108.3, 108.2, 108.0, 108.9, 108.9, 108.4, 108.6, 108.6 |
| Observed | 111.88 s | 112.17 s | 109.42 s | 112.2, 111.7, 111.9, 111.7, 111.8, 111.9, 112.0, 111.9, 111.8 |
| **Overhead** | **+3.2%** | +3.0% | +1.0% | |

**Paired (per round): median +3.2%**, rounds +3.8, +3.2, +3.4, +3.4, +2.6, +2.7, +3.4, +3.0, +3.0%. Every observed run recorded manifest `b3:659238f2...` (29,937 inputs).

Phases of the observed run (median): setup 0.67 s, import 0.65 s, mount 0.12 s, execution 109.42 s (**+1.07 s over plain**), output diff 0.55 s (storing ~840 MB of build cache 0.49 s), ledger 0.24 s, cleanup 0.20 s, unaccounted 0.03 s. Observed minus plain: 3.53 s.

Read it carefully: the percentage is low because the step is long on this box (108 s here against 15.4 s on the 18-core arm64 VM, where the same build measured +9.7%). In absolute terms the fixed costs are ~2.3x arm64's (3.53 s against 1.50 s): setup, import and output storing are single-threaded hashing and syscalls that scale with per-core speed, not with core count. On a 16-vCPU runner the step would be shorter and the percentage closer to arm64's. Observer: `fuse-uring+nice-10+passthrough+zero-opendir+sealed`, 56,938 ring requests, 0 over /dev/fuse, serving CPU 0.49 / 1.14 s user/sys, serving threads waited 0.40 s for a CPU.

This is the cost of a first run (a miss with no Go cache). In the history, Quod's misses ran with its warm Go cache, and the fixed costs per step were 1.5-3.5 s (setup 0.64, import 0.44-0.67, mount 0.11, ledger 0.21, plus F3's 1.1-2.1 s): small next to `go test`'s 40 s, as large as the whole step for `go build`/`go vet` with a warm cache.

## 3. Replay speed (unchanged inputs)

The pinned commit, inputs unchanged, 5 interleaved rounds (series order rotated each round), round 1 dropped, 4 measured. Driver `0b-x86-hetzner/replay.py`; rows `replay-rows.jsonl`. Before round 1: one plain job filled the warm GOCACHE, and two Quod jobs executed each step twice (the two proofs). Series:

- **plain-warm (a):** fresh checkout, GOCACHE from the earlier job (a warm `actions/cache`); fresh HOME.
- **plain-warm, same workspace:** the same job again right after, on the same checkout (a persistent self-hosted workspace: mtimes kept).
- **plain-cold (b):** fresh checkout, empty GOCACHE.
- **Quod:** step A plus three `quod run --mode replay --warm` (shipped trust defaults), persistent store and ledger.
- **Quod, warm cache deleted:** the same after `rm -rf <state>/warm`: a fresh runner that has only Quod's store and ledger (what Quod Cloud would give a new runner).

Median of 4 (min-max):

| Step | Plain-warm | Plain-warm, same workspace | Plain-cold | Quod replay | Quod replay, no Go cache |
| --- | --- | --- | --- | --- | --- |
| go build ./... | 2.10 s (2.07-2.11) | 2.03 s | 40.93 s (40.72-41.12) | **1.28 s** (1.24-1.30) | 1.28 s |
| go vet ./... | **0.86 s** (0.84-0.87) | 0.82 s | 15.52 s (15.30-15.64) | 1.28 s (1.25-1.31) | 1.27 s |
| go test ./... | 10.14 s (10.06-10.58) | 9.41 s | 67.86 s (67.37-68.10) | **1.30 s** (1.28-1.32) | 1.29 s |
| step A (go mod download) | 0.03 s | 0.03 s | 0.03 s | 0.70 s | 0.71 s |
| **Job (checkout + A + 3 steps)** | **13.3 s** | 12.3 s | **124.5 s** | **4.6 s** | **4.6 s** |

Every Quod step in the 5 rounds replayed (30 replays, 0 executions); the ledger's 3 keys are `proven` with 10 replays each. No audit fired: the 5% schedule audits the 20th replay of a key and this series did 10 per key. The 3 proof comparisons were identical.

What it means:

- **(a) Against a warm Go cache, a replay of the whole job is 2.9x faster (4.6 s vs 13.3 s), and almost all of it is `go test`: 1.3 s vs 10.1 s (7.8x).** Go's test cache does not reach the floor: 36-42 of cli/cli's test packages re-ran on every warm plain job even on an identical tree (their tests read files with fresh mtimes, or the job's fresh HOME); `pkg/cmd/repo/create` and `pkg/cmd/extension/browse` alone take ~5.7 s. Quod's content-keyed replay has no such leak. For `go build` Quod saves 0.8 s; for `go vet` it is 0.4 s *slower* than Go's cache.
- **(b) Against no cache, 27x (4.6 s vs 124.5 s)**, and Quod's replay does not need its own warm Go cache (identical with it deleted).
- Quod's replay floor is ~1.3 s per step (setup ~0.6 s, import ~0.45 s, lookup at 2-3 candidates ~0.1 s, plus process start and the result written back) and ~0.7 s for step A, which plain finishes in 0.03 s.
- This is the one situation where Quod clearly beats a good cache, and it is real CI traffic (re-runs, the same tree pushed to a PR branch and then merged, several workflows on one commit), but it is not what the commit history above contained: there, inputs were unchanged three times in a row only once.

## 4. Resource cost

From the 120 history step runs (the quod process itself: its FUSE observer threads, hashing, ledger; not the step's `go` processes, which plain pays too):

| Step | quod CPU per run, median / max | quod peak RSS, median / max | Observer serving CPU while mounted, median |
| --- | --- | --- | --- |
| go build ./... | 2.25 s / 5.11 s | 169 MB / 286 MB | 0.16 s |
| go vet ./... | 2.04 s / 2.66 s | 155 MB / 237 MB | 0.37 s |
| go test ./... | 2.15 s / 2.80 s | 156 MB / 250 MB | 0.40 s |

Total quod CPU over the history: 258 CPU-seconds (on a box where the steps themselves used ~4 vCPUs for 39 minutes). The observer is cheap; the fixed costs (fingerprint, import, lookup, ledger, warm-cache check) are what costs wall time. Peak RSS and lookup time grow linearly with the number of earlier manifests of the key (F4). `go test`, by commit:

| Commit | Candidate manifests | Lookup | quod peak RSS |
| --- | --- | --- | --- |
| 1 | 0 | 0.00 s | 74 MB |
| 10 | 7 | 0.26 s | 99 MB |
| 20 | 17 | 0.54 s | 154 MB |
| 30 | 25 | 0.78 s | 198 MB |
| 40 | 33 | 1.03 s | 250 MB |

About 30 ms and 5 MB per candidate. Unbounded: at 1,000 manifests per key that would be ~30 s and ~5 GB per lookup.

Store growth over 40 commits (`du -sb` after each commit):

| After commit | State dir | CAS (blobs, snapshots) | Warm Go cache | Dependency trees | Ledger (sqlite) | Plain side's own GOCACHE |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | 1,594 MB | 389 MB | 837 MB | 353 MB | 16 MB | 843 MB |
| 9 | 3,105 MB | 522 MB | 2,105 MB | 357 MB | 121 MB | 2,118 MB |
| 17 | 4,269 MB | 655 MB | 3,009 MB | 357 MB | 248 MB | 3,028 MB |
| 25 | 5,383 MB | 788 MB | 3,872 MB | 357 MB | 365 MB | 3,898 MB |
| 33 | 6,301 MB | 921 MB | 4,557 MB | 357 MB | 466 MB | 4,587 MB |
| 40 | 7,099 MB | 1,066 MB | 5,059 MB | 407 MB | 566 MB | 5,095 MB |

Per commit: +17 MB CAS, +14 MB ledger, +108 MB warm cache (the same growth as Go's own cache on the plain side: neither is evicted here). The ledger's ~4.7 MB per executed step run (a ~30k-entry manifest and its result) is what Quod Cloud would store per CI step; at 3 steps per push that is ~14 MB per commit before compression.

## Findings

- **F1 (bug, portability): `--allow-net` steps have no DNS on stock Ubuntu.** `/etc/resolv.conf` there is a symlink to `../run/systemd/resolve/stub-resolv.conf`. In host mode the sandbox binds `/etc` but not `/run`, so the link dangles inside the sandbox and Go falls back to `[::1]:53` (`dial tcp: lookup proxy.golang.org on [::1]:53: ... connection refused`). Step A (`go mod download`) fails on the first run of any Ubuntu runner with systemd-resolved, which is most of them. Workaround used here: a regular `/etc/resolv.conf` with the upstream servers from `/run/systemd/resolve/resolv.conf`. A fix would bind the resolved target (or `/run/systemd/resolve`) for egress steps, as the rootfs path already binds `/etc/resolv.conf` explicitly.
- **F2 (bug, robustness): a `quod run` killed by SIGTERM leaves its FUSE (`fuse.cas`) and overlay mounts and a fusectl mount in /tmp behind.** Seen when I stopped an aborted history run; cleaned up with `umount -l`. Nothing replayed from it (the run never recorded), so not a correctness issue, but a runner pool would accumulate mounts.
- **F3 (performance): every quod step pays a cost proportional to the warm Go cache.** Wall time outside the run event's own phases (`wall - total_ms`) grew linearly from 1.1 s to 2.1 s per step over the 40 commits while the warm cache grew from 0.8 GB to 5.1 GB. This is most likely the warm cache's snapshot and post-step delta check (DECISIONS M1 and MINOR 5: every entry's identity is compared); not profiled. Unbounded without cache eviction.
- **F4 (scaling): lookup time and memory grow linearly with the history.** Each lookup deserializes every earlier manifest of the step key (~30k entries each): ~30 ms and ~5 MB of RSS per candidate, 1.03 s and 250 MB at 33 candidates. A replay at commit 34 spent 0.76 s in lookup (27 candidates); with setup (0.63 s) and import (0.44 s) it took 1.9 s, longer than plain `go vet` with a warm cache (1.2 s). The polyglot report saw the same on luxon (0.07 s at 1 candidate, 0.73-0.87 s at 28-33). This needs an index (e.g. by the hash of the first entries, or a trie over manifest entries) before any real repo's history.
- **F5 (size): the ledger grew to 566 MB over 120 step runs** (~4.7 MB per executed run: each stores a ~30k-entry manifest and its result). Store total 7.1 GB, of which 5.1 GB is the warm Go cache (Go's own warm cache on the plain side reached the same 5.1 GB).
- **F6 (expected, but decisive): `go build ./...` reads `.git`** (VCS stamping: `git status`, `git log`), so it misses on every commit; the receipt names `.git/index` on 38 commits and `.git/objects/pack` on 1. `-buildvcs=false` would make it replayable, at the cost of the binaries' embedded revision.

## Methodology

### Host

Hetzner CCX23, Ashburn: 4 dedicated vCPUs (AMD EPYC-Milan, 2 cores x 2 threads as the guest sees them), 16 GB RAM, no swap, 160 GB NVMe (ext4 on `/dev/sda1`), Ubuntu 26.04.1, kernel 7.0.0-30-generic. FUSE over io_uring on (`echo Y > /sys/module/fuse/parameters/enable_uring`); every observed run's event says `transport: fuse-uring`, `uring_fallback: false`. Nothing else ran on the box during measurements; everything ran sequentially from one driver. The guest exposes no cpufreq interface (`/sys/devices/system/cpu/cpu0/cpufreq` is absent), so the CPU governor could not be pinned. Load averages (1 min) before each run: 4.2-9.1, median 6.1, all from the benchmark's own `go` processes on 4 vCPUs.

Toolchains as `versions.toml` pins them: Rust 1.98.0, Go 1.27.3 (tarball checked against `go_sha256_amd64`), Node 24.22.2, Python 3.12.16 (python-build-standalone 20261026), uv 0.13.6; bubblewrap 0.11.1, fuse3 3.18.2, sqlite 3.46.1.

Quod: main `5d296c4` shipped with `git archive HEAD`, built on the server with `cargo build --release -p quod-cli` (1m14s).

Host changes, all before the measured runs (the host fingerprint hashes /etc, so a change mid-run is a miss): SSH `PasswordAuthentication no` (it was on) and `KbdInteractiveAuthentication no`, ufw default deny incoming, SSH allowed; `/etc/resolv.conf` made a regular file (F1); a `runner` user, uid 1001, for the plain baseline (Quod runs steps as uid 1001; cli/cli's permission tests fail as root). Host fingerprint for every measured run: `b3:5a2f737c`.

### History

Driver `0b-x86-hetzner/history.py`, run by `chain.sh` (the exact command of every run is in its row of `history-rows.jsonl`; each run's stdout and stderr, Quod's receipts included, in `history-run-outputs.tgz`). The 40 first-parent commits ending at `9490d204a2ba`, oldest first. For each commit, in alternating order (even commits plain first, odd Quod first):

- **plain-warm:** `git clone --shared` of the commit into a constant path (`/work/hist/plain/cli`, as on a CI runner; a fresh checkout like `actions/checkout`, so file mtimes are new), then as uid 1001 with Quod's Go profile env (`CGO_ENABLED=0 GOFLAGS=-mod=mod GOTOOLCHAIN=local LANG=C.UTF-8 TZ=UTC GOTELEMETRY=off`, a fresh HOME, a persistent GOMODCACHE): `go mod download` (with GOPROXY), then `go build ./...`, `go vet ./...`, `go test ./...` (GOPROXY=off), each `/bin/sh -e -c`, with one GOCACHE kept across all 40 commits. This is a perfect `actions/cache`: no restore or save time, never evicted. A real one costs tens of seconds per job to restore ~1-5 GB, which this baseline does not pay.
- **plain-cold** (every 5th commit: 5, 10, ..., 40): the same with an empty GOCACHE at the start of the job, shared by its three steps.
- **Quod:** one persistent state dir (store, ledger, warm GOCACHE, dependency trees) and one checkout (`git checkout -f -B quod-hist <commit>`), then `quod run --allow-net --step "go mod download"` (step A, never memoized) and `quod --state S run --repo R --commit C --mode replay --warm --step "<step>"` for the three steps. Shipped trust defaults: proofs 2, audit 5%. `--warm` is what the runner hooks do for go steps (DECISIONS, M1): Go's build cache persists in the state dir; GOFLAGS gains `-count=1` and GODEBUG `goindex=0`.

"Job" time is checkout + step A + the three steps (Quod's checkout is a `git checkout` in place and not timed; plain's fresh clone is 0.2 s). Commit 1 is reported but excluded from totals: both caches are empty there. Hits, outcomes and the first changed input come from each run's `run` event (`outcome`, `hit`, `first_mismatch`); CPU and peak RSS of the quod process from `/proc/<pid>/stat` and `VmHWM`, sampled every 100 ms (self only, not the step's processes; the last <100 ms is unsampled).

### Overhead and replay speed

The protocols are in their sections (2 and 3); the overhead uses `quod bench` unchanged, the replay series the driver `0b-x86-hetzner/replay.py` with the same plain env and Quod flags as the history.

## Caveats

- One host class, one corpus. luxon and starlette were not run (time); arm64 numbers in `0a-*.md` are from a different, 18-core machine and are not comparable in absolute terms.
- 4 vCPUs make `go test ./...` ~6x slower than on the 18-core dev VM (108 s vs ~15-17 s), which inflates every absolute number; the ratios are what to compare.
- The plain-warm baseline is generous (free, perfect cache). The plain-cold baseline is sampled on 8 commits; its spread was 122.7-124.5 s, so extrapolating the total is safe.
- The history walks first-parent merges in order, each once. Real CI also re-runs the same tree (PR head then merge, re-runs, several branches), where replays would be more frequent; this benchmark does not model that traffic, and the replay-speed section bounds what such a re-run gains.
- No cherry-picking: every run is in `history-rows.jsonl`; no run failed (every exit code 0); the slow plain-warm commits (70-79 s at 7-9, 15, 17, 19, 31, 39) are dependency bumps (`go-gh`, `glamour`) and changes under `api/`, `internal/config`, `git/`, which nearly every package imports, so Go re-runs most tests there (at commit 15, 283 of 320 test lines were not cached).
- A first history run (N=30) was stopped after 2 commits to restart with N=40 once the per-commit time was known; it is not used (its driver log is `history-aborted-n30.log`; its 2 commits matched the N=40 run's shape: plain-warm 14.9 s, Quod 49.5 s on the second commit).
- The CPU and RSS sampling of the quod process misses its last <100 ms and counts only the quod process, not bwrap or the step.
