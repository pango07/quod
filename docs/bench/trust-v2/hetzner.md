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

## Results

Pending: the chain is queued behind other agents' work on the box
(`/work/tv2/go.log`, `/work/tv2/logs/*.log`); this file is updated from
`/work/tv2/summary.md` when it lands.

## Correctness

Every audit in every series must agree (0 divergences, 0 suspect replays)
for the hit rate to mean anything: a divergence would say the normalization
or the step is not what trust v2 assumes. Reported with the results.
