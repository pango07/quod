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
3. The module cache is sealed: served from a read-only snapshot of
   `GOMODCACHE` (immutable module@version dirs) made by hard-linking CAS
   blobs into the run dir, `gomod/cache` stays on the observed overlay, and
   every tree entry under the prefix is recorded. Manifests grow from ~21k
   to ~29.9k entries.

Acceptance with all of the above: A1-A6 pass, A7 mutated 23,939 f, 5,320 d,
620 a entries with 0 false hits, A7b 10/10 (replay agrees with a fresh
execution for every unread file mutated). The acceptance log of the final
code is `0a-cli-cli-arm64-exp-overhead-acceptance.log` next to this file.

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

## Combined, after the rebase onto main (exp/misses merged)

`stillpass bench --no-hit-rate --interleave --runs 8`, plain vs the final
observer (passthrough + sealed module cache, snapshot by hard links) with
exp/misses' cheaper output diff, on `main` af38959 plus this branch. The
box was at load average 20-54 throughout and the plain series ranged 16.2
to 28.5s, so these are upper bounds; round 7 was the one quiet pair.

| | Plain | Observed | Execution | Overhead (wall / execution) |
| --- | --- | --- | --- | --- |
| All 7 non-warm-up runs, median | 25.37s | 30.80s | 27.99s | +21.4% / +10.3% |
| Round 7 (quiet) | 17.9s | 19.9s | 17.5s | +11% / -2% |

Observed phase medians: import 0.54s, mount 0.94s, execution 27.99s,
output diff 0.79s (1.99s before exp/misses), ledger write 0.10s. The mount
phase now holds the module-cache snapshot (~22k hard links and chmods,
~4.4k directories); at this load it cost 0.9s, roughly 0.3-0.4s when the box
is quiet (import, a comparable amount of syscalls, inflated 2.7x in the same
run). Per run (execution in parentheses): plain 16.2, 25.4, 28.5, 22.4,
23.8, 27.3, 17.9, 28.5; observed 35.0 (27.8), 32.5 (29.0), 31.8 (28.7),
24.9 (21.5), 30.8 (28.0), 32.7 (30.0), 19.9 (17.5), 29.5 (26.3).

Acceptance on the same tree: A1-A7 pass, A7 0 false hits over 29,879
entries, A7b 10/10 (replay and fresh execution read the same 29,879 paths),
W1-W4 pass with sealing on (`0a-cli-cli-arm64-exp-overhead-acceptance.log`).

## Round 2 (exp/overhead2)

### Shared snapshots

The module-cache snapshot is built once per distinct subtree under
`<cas>/snapshots/<key>` (inside the store: it is hard links into it) and
reused across runs and commits by every process sharing the store (key:
BLAKE3 over the format version and every entry's path, kind, hash, mode and
target, length-prefixed). Three misses with distinct step keys on the same
tree, load 5-7: build 778ms (26,637 entries), then reuse 3ms, 3ms; the mount
phase went from ~800ms to 34ms. Later bench numbers are therefore the
snapshot-warm steady state: a store that has seen the module cache once.

### FUSE over io_uring (`crates/sp-uring-probe`)

The same minimal read-only FUSE server with two transports, fresh mount per
run (cold kernel caches), three interleaved rounds at load 1.5-8.4,
reads served in userspace on both (`FUSE_NO_OPEN_SUPPORT`), ext4 for
reference. Kernel 7.0.14-linuxkit, FUSE API 7.45, `enable_uring=Y`.

| Probe | classic `/dev/fuse` | io_uring | ext4 |
| --- | --- | --- | --- |
| `stat` of 10k module-cache paths | 0.355, 0.355, 0.340s | 0.082, 0.079, 0.077s | 0.01s |
| `cat` of the same 10k files | 1.090, 1.075, 1.082s | 0.228, 0.233, 0.231s | 0.07s |
| `ls -R` of the module cache (4.6k dirs) | 0.494, 0.510, 0.512s | 0.085, 0.090, 0.099s | 0.01s |
| `go list -deps -test ./...` | 3.071, 3.041, 3.047s | 0.800, 0.796, 0.831s | 0.51s |

The ring cuts the per-request cost about 4.4x; Go's serial loading phase
goes from +2.5s over ext4 to +0.3s. This is the transport the observer
should move to; fuser 0.18 does not have it.

### Paired bench, round 2

`stillpass bench --no-hit-rate --interleave --runs 8`, plain vs the final
observer (passthrough, sealed module cache from a shared snapshot, exp/misses
output diff), on main 6372c70 plus exp/overhead2. Another agent kept the
box at load average 19-37; the load was steady through rounds 2-6 (plain
15.1-15.4s), so those pairs are the clean comparison.

| | Plain | Observed | Execution | Overhead (wall / execution) |
| --- | --- | --- | --- | --- |
| All 7 non-warm-up runs, median | 15.26s | 18.14s | 16.73s | +18.9% / +9.6% |
| Rounds 2-6 (steady load 19-31), median | 15.3s | 18.1s | 16.6s | +18% / +8.5% |

Both rows are the snapshot-warm steady state (the module-cache snapshot
already built in the shared store; mount 0.04s).

Per run (execution; load after the run): plain 15.3 (19.0), 15.3 (21.9),
15.1 (24.6), 15.2 (27.4), 15.4 (27.2), 15.7 (32.6), 18.4 (37.4); observed
18.1 (16.7; 20.7), 18.3 (16.6; 25.5), 17.9 (16.4; 28.3), 17.8 (16.5; 30.7),
18.1 (16.7; 31.1), 20.3 (18.6; 33.5), 20.7 (18.9; 36.4). Observed phase
medians: import 0.41s, mount 0.04s, execution 16.73s, output diff 0.64s,
ledger write 0.08s. The same bench with the snapshot rebuilt every run
(before it moved into the store) read +22.1% wall, mount 0.87s.

What is left: ~1.5s of execution, which the io_uring probe above says is
mostly the round trip of the ~10k requests the repo's own sources and
`gomod/cache` still send; ~0.6s of output diff; ~0.2-0.4s of import.

## Round 3 (exp/overhead3): the observer on the ring

`sp-fs` serves the same model over two transports, chosen once per mount:
FUSE over io_uring when root, `fuse.enable_uring=Y` and the kernel offers it
(one pinned queue per CPU, four entries each), else the classic `/dev/fuse`
loop through fuser. The run event's `observer` says which.

### Per-request cost of the real observer

A `sleep` step keeps the mount up; the probes run on the lower (FUSE) mount
directly, so they include the access log, counters and passthrough opens.
Two rounds, alternating, at load 53-65 (acceptance and other agents running).

| Probe (10k module-cache files) | classic | io_uring |
| --- | --- | --- |
| `stat` (14,642 lookups) | 0.44s, 0.51s | 0.06s, 0.07s |
| `cat` (10,000 opens, passthrough reads) | 0.64s, 0.58s | 0.20s, 0.21s |
| `ls -R` (9,284 readdirs) | 0.68s, 0.95s | 0.07s, 0.07s |
| observer CPU (user / system) | 89/453ms, 68/255ms | 25/117ms, 27/119ms |

Identical request counts on both transports; the ring answers each request
on the CPU that raised it and spends a quarter of the system time.

### Acceptance, both ways

With the ring (`observer: fuse-uring+passthrough+sealed`): A1-A6 pass, A7
mutated 23,939 f, 5,320 d, 620 a entries with 0 false hits, A7b 10/10
(replay and fresh execution read the same 29,879 paths), W1-W4 pass
(`0a-cli-cli-arm64-exp-overhead-acceptance-uring.log`). With the fallback
forced (`STILLPASS_FUSE_TRANSPORT=classic`, `observer:
fuse+passthrough+sealed`): the same (`0a-cli-cli-arm64-exp-overhead-acceptance.log`).
The sp-fs integration test `transports_log_identically` runs find-and-cat,
ls, stat of missing paths, symlinks, the stat -f probe and go test on a tiny
module over both transports and asserts identical access logs.

### Paired bench, round 3

The first attempt (plain, ring observer, classic observer, 8 interleaved
runs) ran at load average 43-65 and aborted in round 7 when a *plain*
baseline run failed to build four packages (no observer involved; the
harness requires the baseline to pass). Its six rounds carry no signal:
plain alone ranged 16.2-30.3s. The rerun below waited for a quieter box.

## Report as written by `stillpass bench` (all 7 non-warm-up runs, first bench, before the rebase)

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
