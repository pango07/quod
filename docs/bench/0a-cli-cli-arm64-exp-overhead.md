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

`sp-fs` serves one model (`CasFs`) over two transports, chosen once per
mount before the step starts: FUSE over io_uring when root,
`fuse.enable_uring=Y`, io_uring with 128-byte SQEs works and the kernel
offers the ring at INIT with FUSE 7.42-7.45 (one queue per possible CPU,
its thread pinned to that CPU at nice -10, four 128 KiB entries each), else
the classic `/dev/fuse` loop through fuser (16 threads). The ring comes up
whole or not at all: every queue reports its registration, then a statfs
from Stillpass itself must come back over the ring; any failure tears the
mount down and the classic transport serves all of it (`observer:
fuse+uring-fallback+...`). The run event's `transport` is `fuse-uring` or
`fuse`, and the ledger keeps it with the run (schema v2), so results a
transport produced can be found and purged.

### What is tested

All in the dev container (root, `enable_uring=Y`, Linux 7.0.14, FUSE 7.45),
each #[ignore]d because it needs the ring (on a host without it each fails
saying so; none compares classic with classic). CI exercises the classic
transport only; the ring is verified by these `--ignored` runs, raw output
in `0a-cli-cli-arm64-exp-overhead3-ring-tests.log`:

- `transports_log_identically` (crates/sp-cli/tests/transports.rs): eight
  shell workloads (find and cat of every file; ls; ls -lA; stat of missing
  paths; symlinks to a file, a directory and nothing; a 700-entry directory
  listed across several readdir offsets; stat -f; reads at offsets of a 3 MiB
  file), each on a fresh ring mount and a fresh classic mount, with
  passthrough on and with it off (every read then goes through the ring's
  READ path). Asserts identical stdout, identical access logs, and that the
  ring half reported `uring`, answered requests over the ring and none over
  the device. Then `go test ./...` on the tiny Go module through the CLI,
  once per transport: identical manifests, run events and ledger rows naming
  `fuse-uring` and `fuse`. Request counts can differ with identical logs (the
  classic transport needed 43 readdirs for listings the ring did in 34).
- Failure paths (crates/sp-fs/tests/uring.rs and
  crates/sp-cli/tests/ring_faults.rs, through hooks compiled in only with
  the `fault-injection` feature): a refused registration and a ring
  that never becomes ready fall back to classic before anything is served
  (and fail when the ring is forced); a queue thread that panics or sees an
  error completion, at the first request after the mount came up or halfway
  through `go test`, fails the step at once: the run exits 2 within about a second (0.3-1.1s)
  with no run, manifest or result row in the ledger, the observer's own
  error event names the dead ring queue and the injected cause, and the
  mount is gone.
- Unit tests in uring.rs: every reply encoder against #[repr(C)] mirrors of
  the FUSE 7.45 structs (sizes and field offsets), the SQE byte layout, CPU
  lists.
- What is not tested: kernels other than 7.0 (minors 42-44 are accepted on
  the strength of the uapi changelog alone), and a reply the kernel rejects
  over the ring, which the kernel never reports to the daemon (the encoder
  tests are the guard).

### Acceptance, both ways

`scripts/acceptance.sh /work/cli` at 2dfcf2d, once with
`STILLPASS_FUSE_TRANSPORT=uring` (forced: a ring that failed to come up
would have been a Stillpass error, not a silent fallback) and once with
`classic`, back to back, 14:40-15:11 UTC at load 16-48. Each log carries
the binary's own `observer` and `run` event lines after A1 naming the
transport (`"transport":"fuse-uring"`, `"observer":"fuse-uring+passthrough+sealed"`,
70,554 ring and 0 device requests; and `"transport":"fuse"`).

| | ring (`0a-cli-cli-arm64-exp-overhead-acceptance-uring.log`) | classic (`0a-cli-cli-arm64-exp-overhead-acceptance.log`) |
| --- | --- | --- |
| A1-A6 | pass; executions 21.8, 18.4, 18.1, 18.8s | pass; executions 21.3, 25.6, 29.7, 21.4s |
| A7 | 23,939 f, 5,320 d, 620 a mutated, 0 false hits, 451.0s | the same, 0 false hits, 455.8s |
| A7b | 10/10, every fresh execution read the replay's 29,879 paths (20.0-25.6s) | 10/10, the same (17.3-35.2s) |
| W1-W4 | pass, 29,843 warm entries with a full and an empty cache | pass, the same |

Both record the same manifests (A1 `b3:23ea6595`, 29,878 inputs, on both).

### The earlier "ring is 40% slower" was the box

The first acceptance runs of this branch (pre-fix code; the logs, since
replaced, are in git at 3dde4de) had the ring well behind the forced classic transport: first
execution 23.5s against 16.4s, the A4-A6 executions 25.5-31.5s against
16.2-16.3s, and A7 493s against 379s. A7 is the control. It mutates the
recorded tree in memory and asks the matcher, with no FUSE involved
(`sp_ledger::first_match` over CAS blobs), so a 30% slower A7 says the box
was slower during the ring run (the two ran 15 minutes apart on 18 cores
shared with other agents), not the transport; run back to back with the
fixed code, A7 took 451.0s in the ring run and 455.8s in the classic one,
and the A1 executions were 21.8s and 21.3s. What was checked:

- Device loop serializing traffic: no. Every ring run since reports 0
  requests answered over the device (the kernel holds requests until each
  queue has an entry, so the device only ever sees forgets and
  interrupts), the readiness probe would now catch a connection the kernel
  switched back to the device, and the kernel's once-per-boot
  `FUSE_IO_URING_CMD_REGISTER failed` message first appeared in this boot
  when the new failure test refused a registration on purpose, hours after
  those runs.
- Request batching: there is none to speak of. A cli/cli `go test` sends
  ~70.5k ring requests (the counted ones are 26.3k opens, 5.5k readdirs and
  3.2k lookups; the rest are mostly the release that follows every open and
  overlayfs's xattr probes, which the model answers without counting) in
  ~70k completion batches; 28-35 per run found all four entries of a queue
  busy, so the depth is not a limit either.
- Lock contention in the model: 16-54 contended lock acquisitions per
  `go test` run; negligible.
- Pinned queue threads under load: the one real effect, and smaller than
  the classic transport's own. A queue thread is pinned to the CPU whose
  requests it serves, so on a busy CPU it waits behind the step's
  processes. Per `go test`, the ring's 19 serving threads ran 142-238 ms in
  all and waited 275-1875 ms for a CPU; fuser's 17 ran 473-759 ms and
  waited 3567-23289 ms (`0a-cli-cli-arm64-exp-overhead3-gotest-sched.log` at
  load 29-35, and every run of the final bench below). Unpinned ring
  threads answer slower (cross-CPU wakeups: serial cat 1.7-4x slower at
  load 22) and were no faster end to end (exploration bench execution
  median 39.55s against 38.19s pinned). At nice -10 the woken
  thread runs ahead of the step's processes: its wait was lower than at
  nice 0 in all 8 rounds of the final bench (median 560 against 755 ms), so
  -10 is the default (`STILLPASS_URING_NICE`). Its end-to-end effect is
  below what this box can resolve: faster than nice 0 in 5 of 6 rounds of
  the exploration bench and in 2 of 7 of the final one. Where the ring can
  lose is a box loaded far past its cores with many requesters in parallel:
  at load 84-100, 18 parallel `cat`s of 10k files took 0.16-0.34s on the
  ring and 0.15-0.22s on classic, whose threads can run on any CPU.

Per-request cost with the real observer (`per_request_cost`, fresh mount per
probe, 10k module-cache files; wall seconds, min-max over the rounds;
`0a-cli-cli-arm64-exp-overhead3-per-request-nice0.log`, 3 rounds at load
~22 with the ring at nice 0, and `0a-cli-cli-arm64-exp-overhead3-per-request.log`,
4 rounds at load 84-100 with the default nice -10 and nice 0 side by side):

| Probe | ring, load 22 | classic, load 22 | ring, load 84-100 | ring nice 0, load 84-100 | classic, load 84-100 |
| --- | --- | --- | --- | --- | --- |
| `stat`, serial (11.3k lookups) | 0.049-0.067 | 0.363-0.546 | 0.064-0.138 | 0.073-0.221 | 0.105-0.237 |
| `stat`, 18 processes | 0.036-0.110 | 0.074-0.094 | 0.031-0.131 | 0.032-0.098 | 0.070-0.225 |
| `cat`, serial (10k opens, passthrough reads) | 0.195-0.272 | 0.811-1.390 | 0.229-0.271 | 0.227-0.258 | 0.325-1.757 |
| `cat`, 18 processes | 0.040-0.094 | 0.114-0.136 | 0.163-0.339 | 0.070-0.412 | 0.145-0.219 |
| `ls -R` (9.3k readdirs) | 0.072-0.085 | 0.527-0.833 | 0.083-0.146 | 0.072-0.100 | 0.115-1.068 |

Identical access logs on every variant (the probe asserts it). The serving
threads' own CPU per probe is a fifth to a half of fuser's at load 22 (cat
from 18 processes: 30-50 ms user and 60-80 ms system against 40-60 and
230-540 ms).

### Paired benches

Exploration, 7 interleaved runs per series, first dropped, load 40-103
(`0a-cli-cli-arm64-exp-overhead3-bench-explore.{md,log}`; the ring here is
pinned at nice 0, the default then). Execution medians: plain 30.77s
(wall), ring 38.19s (+24.1%), classic 44.01s (+43.0%), ring unpinned 39.55s
(+28.5%), ring at nice -10 32.33s (+5.1%). The ring beat classic in all six
paired rounds (execution 32.5/34.7, 23.1/25.9, 43.8/51.8, 49.3/61.9,
49.8/54.3, 25.6/36.2s); nice -10 beat nice 0 in five of six.

Final, 8 interleaved runs per series, first dropped, load 42-105
(`0a-cli-cli-arm64-exp-overhead3-bench.{md,log}`, 2dfcf2d, right after the
two acceptance runs). The plain baseline itself ranged 18.5-50.4s, so the
medians are noise-dominated; the paired rounds and the mechanism columns
carry the result.

| Series | Wall median | Execution median | Execution, rounds 2-8 (s) | Serving threads' run-queue wait, median |
| --- | --- | --- | --- | --- |
| Plain | 30.45s | (30.45s) | 30.5, 18.5, 19.0, 48.3, 34.1, 22.2, 50.4 (wall) | - |
| Ring (default: pinned, nice -10) | 36.58s | 32.60s | 42.5, 21.4, 21.2, 32.6, 34.3, 22.4, 32.8 | 560 ms |
| Classic (`STILLPASS_FUSE_TRANSPORT=classic`) | 38.14s | 34.40s | 35.9, 18.0, 42.9, 34.4, 24.0, 25.7, 43.0 | 12,078 ms |
| Ring at nice 0 | 24.82s | 22.24s | 21.6, 17.5, 37.1, 28.5, 22.2, 22.2, 43.3 | 755 ms |

Every observed run answered 70,523-71,605 requests over the ring and none
over the device, or (classic) none over the ring. Over both benches the
ring (either nice value) executed faster than classic in 22 of 26 paired
rounds. Against plain the observer cost +7.0% execution (+20.1% wall) at
the default and +13.0% (+25.2%) classic, at this load; the round-2 numbers
above (load 19-31) remain the better estimate of the overhead itself.

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
