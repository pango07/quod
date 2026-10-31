# Observer overhead experiment (exp/overhead)

Where the execution-phase overhead of the 0a observer goes, and what the
`exp/overhead` branch did about it. Companion to `0a-cli-cli-arm64.md`
(the T12 numbers). Same box: Docker Desktop, 18 arm64 cores, kernel
7.0.14-linuxkit, with two other agents running benches on the same cores
throughout (load averages 30-65), so the per-series minimum and the quiet
rounds are the honest estimators; the medians in the table below include
loaded rounds.

## Where the time goes

Measured with per-request counters in the observer (reported in an
`observer` event at unmount; `stat -f <mount>` logs a snapshot) and
per-operation probes on a mounted tree:

- One `go test ./...` on cli/cli sends ~86k synchronous FUSE requests:
  45k open, 21k lookup (4k absent), 20k readdir, 1 getattr, 0 readlink.
  18,925 of the 21,051 manifest entries are the module cache.
- A round trip costs 50-80 µs on this VM: `stat` of 10k paths 0.80s on FUSE
  vs 0.01s on ext4; `cat` of 10k already-cached files 0.53s (the opens) vs
  0.08s; `go list -deps -test ./...` (Go's serial loading phase) 2.2-2.6s vs
  0.7s. That phase sits on the critical path, which is the execution gap.
- Overlayfs costs nothing measurable: the same overlay over a plain ext4
  copy of the tree matched the plain baseline (execution 14.8/14.9s vs
  plain 14.8/15.1s in quiet rounds).
- Daemon CPU, data copying and cold blobs are not the cost: pre-reading
  every blob, nicer worker threads and `FUSE_NO_OPEN_SUPPORT` (reads in
  userspace, no open requests) gave no measurable gain.

## What was kept

1. FUSE passthrough (Linux 6.9+): the kernel reads blobs directly; zero
   READ requests reach userspace, the OPEN that the log needs still does.
2. `FUSE_PARALLEL_DIROPS`, `FUSE_CACHE_SYMLINKS`, `FUSE_NO_OPENDIR_SUPPORT`.
3. The module cache is sealed: served from a read-only bind of the repo's
   `GOMODCACHE` (immutable module@version dirs), `gomod/cache` stays on the
   observed overlay, and every tree entry under the prefix is recorded.
   Manifests grow from ~21k to ~29.9k entries.

Acceptance with all of the above: A1-A6 pass, A7 mutated 23,939 f, 5,320 d,
620 a entries with 0 false hits, A7b 10/10 (replay agrees with a fresh
execution for every unread file mutated).

## Final interleaved bench

Series: plain; the final observer; the original observer
(`STILLPASS_NO_PASSTHROUGH=1,STILLPASS_SEAL_MODCACHE=0`); passthrough only
(`STILLPASS_SEAL_MODCACHE=0`). Rounds 2-5 ran under steady load (plain
17.0-17.3s); rounds 6-8 were heavily loaded (plain 24s). Execution medians
over rounds 2-5: plain 17.2s; original observer 19.5s (+13.4%); passthrough
only 18.95s (+10.2%); final 18.3s (+6.4%). The rest of the wall gap is the
output diff (~2.0s, storing ~900 MB of Go build cache per miss), owned by
a separate experiment.

Per-run log (execution in parentheses):

| Round | Plain | Final | Original | Passthrough only |
| --- | --- | --- | --- | --- |
| 1 (warm-up) | 17.2 | 25.1 (17.7) | 21.9 (19.5) | 21.8 (19.5) |
| 2 | 17.3 | 21.1 (18.6) | 21.9 (19.6) | 21.4 (19.0) |
| 3 | 17.3 | 20.5 (18.4) | 22.1 (19.7) | 21.5 (19.0) |
| 4 | 17.0 | 20.7 (18.1) | 21.3 (19.1) | 20.9 (18.9) |
| 5 | 17.1 | 20.7 (18.2) | 21.5 (19.4) | 21.1 (18.9) |
| 6 | 16.9 | 22.1 (18.6) | 32.0 (27.5) | 34.2 (27.2) |
| 7 | 23.9 | 32.4 (27.6) | 32.5 (27.8) | 30.5 (26.6) |
| 8 | 24.1 | 33.7 (28.8) | 32.4 (28.1) | 32.3 (28.0) |

## Report as written by `stillpass bench` (all 7 non-warm-up runs)

## Machine

| CPU | Cores | RAM | Disk (state dir) | Kernel | Arch |
| --- | --- | --- | --- | --- | --- |
| 0x000 | 18 | 7.7 GiB | ext4   /dev/vda1[/docker/volumes/stillpass-work-overhead/_data] | 7.0.14-linuxkit | aarch64 |

## Overhead (A8)

8 runs per series, interleaved, first dropped as warm-up. Overhead = observed median / plain median - 1. Target under 10%.

| Series | Median | p95 | Execution median | Runs (s) |
| --- | --- | --- | --- | --- |
| Plain (same sandbox, no FUSE or overlay) | 17.27s | 24.11s | 17.27s | 17.3, 17.3, 17.0, 17.1, 16.9, 23.9, 24.1 |
| Observed (import, serve, overlay, sandbox, record) | 21.15s | 33.68s | 18.58s | 21.1, 20.5, 20.7, 20.7, 22.1, 32.4, 33.7 |
| Observed, STILLPASS_NO_PASSTHROUGH=1,STILLPASS_SEAL_MODCACHE=0 | 22.12s | 32.50s | 19.73s | 21.9, 22.1, 21.3, 21.5, 32.0, 32.5, 32.4 |
| Observed, STILLPASS_SEAL_MODCACHE=0 | 21.49s | 34.19s | 19.04s | 21.4, 21.5, 20.9, 21.1, 34.2, 30.5, 32.3 |
| **Overhead** | **+22.5%** | +39.7% | +7.6% | |
| Overhead, STILLPASS_NO_PASSTHROUGH=1,STILLPASS_SEAL_MODCACHE=0 | +28.1% | +34.8% | +14.3% | |
| Overhead, STILLPASS_SEAL_MODCACHE=0 | +24.4% | +41.8% | +10.2% | |

Observed phase breakdown, median over the same runs:

| Phase | Median | Share of observed |
| --- | --- | --- |
| import | 0.20s | 1.0% |
| lookup | 0.00s | 0.0% |
| mount | 0.04s | 0.2% |
| execution | 18.58s | 87.9% |
| output diff | 1.99s | 9.4% |
| ledger write | 0.10s | 0.5% |

## Totals

- Observer overhead (0a, host): median +22.5%, p95 +39.7% (plain 17.27s, observed 21.15s)
