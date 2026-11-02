# Trust v2 on cli/cli's history: hit rates before and after

Branch exp/trust-v2 (`8b3d1ad` + the bench drivers), the same box, corpus and
pin as 0b (docs/bench/0b-x86-hetzner.md on bench/x86-hetzner): Hetzner CCX23
(4 dedicated vCPUs, 16 GB), cli/cli pinned `9490d204a2ba` and the 39
first-parent commits before it, `go build ./...`, `go vet ./...`,
`go test ./...` through `quod run --mode replay --warm`, one persistent state
dir per series, step A (`go mod download`) with the network and never
memoized. Quod only: the plain baselines are 0b's.

## Method

Three series, each from an empty state dir, driven by `traffic.py` and
tabulated by `summary.py` (both in this directory; `chain.sh` runs them
under the box's shared heavy lock, one series per lock hold):

| Series | Traffic | Policy |
| --- | --- | --- |
| history-v2 | one job per commit, as 0b's history (checkout of the first-parent commit, step A, the three steps) | v2 defaults: `--proofs 1 --shadow 2 --audit 0.05` |
| traffic-v2 | each commit as its **pull request head** first (the merge's second parent when the commit is a merge, the commit itself on a `pr` branch otherwise), then as the **merge** on `main`, then with probability 0.10 (seeded, 42) a **re-run** of the merge job | v2 defaults |
| traffic-v1 | the same traffic, same seed | v1's: `--proofs 2 --shadow 0 --audit 0.05` |

The v1 history baseline is 0b's measured run (quod main `5d296c4`, proofs 2,
audit 5%): 120 keyed step runs, 2 replayed (both `go vet`, commits 33 and
34), 12 proving, 106 miss; `go build` never replays (it reads `.git`).

"Unchanged-input runs" below are the keyed step runs whose lookup found a
matching manifest (everything but a miss): the runs trust policy decides.
The hit rate that matters is replayed / unchanged-input runs; the rest of
those runs are the audits (shadow or scheduled), v1's proving runs, and
divergences, all of which execute.

## Where the numbers come from

Two boxes were involved. **Server 1** (5.161.250.249) is the shared box 0b
measured on; it holds the v1 history baseline (0b, 2026-11-01) and nothing
of this run except an aborted, never-measured attempt (its heavy lock was
held by other agents' chains for hours, so the run moved). **Server 2**
(5.161.36.37) is a fresh CCX23 of the same class (4 dedicated vCPUs, 16 GB,
Ubuntu 26.04.1, kernel 7.0.0-31) provisioned for this run alone with 0b's
`install.sh` and host changes (SSH keys only, ufw SSH-only, `/etc/resolv.conf`
made a regular file for 0b's F1, a `runner` user uid 1001, FUSE io_uring
on), the branch shipped by `git archive` (commit `ad4874a`, trust code
identical to `17e7220` on the rebased branch), built there in 79 s. Every
trust v2 number below is from server 2; the plain-warm baseline was re-run
on server 2 so the history comparison is on one box. Rows:
`rows/*.jsonl` here.

## Results

### History: 40 commits, one job each (server 2)

| | v1 (0b, server 1, `5d296c4`, proofs 2, audit 5%) | **v2 (server 2, defaults)** |
| --- | --- | --- |
| keyed step runs | 120 | 120 |
| misses | 106 | 106 |
| unchanged-input runs | 14 (12 proving, 2 replayed) | 14 (7 audited, **7 replayed**) |
| `go vet`: replayed / unchanged | 2 / 8 | **4 / 8** (commits 23, 32, 33, 36; audits at 2, 5, 26, 34) |
| `go test`: replayed / unchanged | 0 / 6 | **3 / 6** (commits 23, 32, 34; audits at 2, 5, 26) |
| `go build` | 0 / 0 (reads `.git`) | 0 / 0 |
| divergences, suspect replays | 0, n/a | **0, 0** |
| a `go test` replay / execution | – | 1.7–2.1 s / ~41 s |
| quod job wall, total (median) | 2,341 s (51.7 s) on server 1 | 2,427 s (53.5 s) on server 2 |

The per-key audit log is exactly the schedule: `go vet`'s 8 unchanged-input
commits were hits 1–8 of its key, hits 1, 2 (commits 2, 5) the shadow,
4 and 7 (26, 34) the decay. `go test` has `volatile` 3: its three
comparisons were identical only up to durations, as every `go test` is.

### Realistic traffic: PR head, merge, 10% re-runs (server 2)

40 commits became 87 jobs (40 PR, 40 merge, 7 re-runs; seed 42), 261 keyed
step runs per series, the same job sequence under both policies.

| | **v2 (defaults)** | v1 (`--proofs 2 --shadow 0`), first run | v1, clean re-run |
| --- | --- | --- | --- |
| replayed / unchanged-input runs, all steps | **57 / 74 (77%)** | 19 / 67 (28%) | _pending_ |
| `go test` | **13 / 18 (72%)**: PR 3/5, merge 4/6, re-run 6/7 | 1 / 16 (6%): PR 0/4, merge 0/6, re-run 1/6 | _pending_ |
| `go vet` | **41 / 49 (84%)**: PR 7/8, merge 28/34, re-run 6/7 | 18 / 45 (40%): PR 5/6, merge 7/33, re-run 6/6 | _pending_ |
| `go build` | 3 / 7 (re-runs only; `.git` differs between PR head and merge) | 0 / 6 | _pending_ |
| audits (shadow + decay) / proving executions | 17 (6 + 11) / 0 | 0 / 48 | _pending_ |
| divergences, suspect replays | **0, 0** | 0, 0 | _pending_ |
| total wall of the 261 keyed runs; of the replays | 5,002 s; 74 s | 5,035 s; 29 s | _pending_ |
| `go test` wall: executions / replays | 3,990 s / 28 s | 3,930 s / 1 s | _pending_ |

Why v1's merge runs almost never replay: the merge commit's tree is the PR
head's tree (first-parent merges, `.git` aside), so the merge run is that
manifest's *second* identical run, which v1 executes to prove (`go vet`:
26 of 33 merge runs "proving"; `go test`: 6 of 6). Only a re-run (the third
run) replays under v1. Under v2 the merge run replays once the key's two
shadow audits are spent (commits 1 and 2's merges), and the re-runs replay
too. `go test`'s v2 misses by first changed input: `.git` 66 of 69 (the PR
head and the merge differ in `.git`, and `go test` reads it through the
module's VCS stamping, see 0b's F6), so `go test` replays only PR-to-merge
pairs where `.git` is not read, re-runs, and docs-only commits.

**The first v1 run is handicapped and kept only as evidence** (`rows/
traffic-v1-aptrun.jsonl` once copied; `/work/tv2/traffic-v1-aptrun` on
server 2): Ubuntu's `apt-daily-upgrade` ran at 06:08–06:10 UTC, in the
middle of it, rewrote /etc (`/etc/alternatives`, `/etc/ld.so.cache`,
`/etc/apparmor.d`, ...), which the host fingerprint hashes, so from commit
28's PR job every step key changed (twice: once per upgrade phase) and v1
started proving from nothing on fresh keys for commits 28–40. v2's series
ran before the upgrade and saw one key per step throughout. The timers are
disabled and the series re-run from an empty state dir; that re-run is the
number to quote.

### Plain-warm baseline on server 2, against the v2 history

0b's plain-warm job (fresh checkout, `go mod download`, the three steps with
a Go build cache that persists across commits), re-run on server 2 after the
quod series, commits 2–40 (commit 1 has empty caches on both sides):

| commits 2–40 | plain, warm Go cache (server 2) | quod, trust v2 (server 2) | 0b on server 1 (plain / quod v1) |
| --- | --- | --- | --- |
| job total | 1,265 s | 2,285 s (1.8x) | 1,230 s / 2,341 s (1.9x) |
| job median | 18.9 s | 53.2 s (2.8x) | 18.3 s / 51.7 s |
| slower commits | – | 36 of 39 | 39 of 39 |
| `go test` total / median | 971 s / 14.6 s | 1,744 s / 42.8 s | – |
| `go build` total / median | 175 s / 2.8 s | 287 s / 5.6 s | – |
| `go vet` total / median | 111 s / 1.2 s | 225 s / 4.5 s | – |
| the 3 `go test` replays (commits 23, 32, 34) | 13.4, 12.5, 12.7 s | **1.7, 1.9, 2.1 s** | – |
| the 3 `go test` audits (commits 2, 5, 26) | 10.0, 10.5, 12.6 s | 41.2, 41.1, 42.4 s | – |

Trust v2 changes the hit rate, not the miss. The three commits where quod
beat plain are exactly the `go test` replays; every miss still runs the
whole suite under `-count=1` (~41 s) where Go's test cache re-runs only the
packages a commit touched (~14 s), and the shadow and decay audits cost a
full execution each. 0b's conclusion on total time stands until the miss is
cheaper (the per-step Go cache inside Quod, exp/never-worse). What v2 buys
on this history is 7 replays instead of 2, and in PR-then-merge traffic a
`go test` replay for 13 of its 18 unchanged-input runs.

## Correctness

Every comparison in every series agreed: history-v2 7 audits (4 shadow, 3
decay), traffic-v2 17 (6 shadow, 11 decay), traffic-v1 48 proofs; **0
divergences, 0 suspect replays**, every key `proven`. The replays served
the recorded result for trees whose manifests matched; the 0 false hits
over 29,940 mutations (A7) and the trust suite ran on the same code in the
GitHub gates (see the report). What the series cannot show: a divergence
that the normalization hides (it erases only durations and timestamps from
stdout/stderr; files are compared exactly) or a flaky test whose unlucky
run fell outside the 24 audits that executed.

## Correctness

Every audit in every series must agree (0 divergences, 0 suspect replays)
for the hit rate to mean anything: a divergence would say the normalization
or the step is not what trust v2 assumes. Reported with the results.
