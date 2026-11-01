# exp/overhead4: first-run overhead on cli/cli, arm64

cli/cli at `9490d204a2ba`, step B `go test ./...` (step A `go mod download`), Docker Desktop arm64 VM (18 cores, 7.7 GiB, kernel 7.0.14-linuxkit), FUSE over io_uring enabled. Branch `exp/overhead4` from main `2a24fd8`.

**Result, rebased onto main `0652921` (exp/git-index), session r5: first-run overhead +10.2% median, +8.0% p95, paired per round +9.5% (plain 15.42s, observed 16.99s), down from main's +14.8% / +16.3% / paired +15.7% (plain 15.59s, observed 17.90s) in the same session.** On the tree before the rebase (main `2a24fd8`, session r2) it was +6.9% / +8.0% / paired +7.6% against main's +12.1%. The target (under 10%) is met on the paired measure and missed by 0.2 points on the median after the rebase: exp/git-index added ~0.3-0.4s per run (serving the `.git`, its index refreshed), of which this branch wins back ~0.24s by building it beside the import. See "Rebased onto 0652921" at the end.

## Protocol

- `quod bench --repo /work/cli --runs 10 --no-hit-rate --interleave`: plain and observed runs alternate (plain, observed[, extra], plain, ...), first round dropped as warm-up, 9 measured rounds. Plain is the same sandbox, uid and env on a fresh copy of the tree with its caches empty, no FUSE or overlay; observed is the whole `quod run` (setup, import, lookup, mount, step, output diff, ledger, cleanup) with a fresh state dir and the ledger empty, so every run is a miss; the CAS persists as between CI runs.
- One session runs main's binary (`2a24fd8`, plus this branch's `bench.rs` for the report format, so its run events lack the new phase fields: their setup shows as "unaccounted"), then this branch's, back to back.
- The whole session holds the shared heavy lock (`flock` on `tmp/heavy.lock` in both the shared `stillpass-work` volume, which the other two agents' containers lock, and this worktree's own): nothing else heavy ran during measurement. The load average before the first run of each session was 5.7 (r1) and 6.8 (r2); the 20-35 seen during the series is the benchmark's own `go test ./...` (18-way parallel) and is the same for both series.
- Reported: median and p95 (nearest rank) of wall per series, overhead = observed median / plain median - 1, and the paired overhead (per round, observed / that round's plain - 1), which cancels drift between rounds. Every observed run's manifest id is listed: all 40 observed runs in both sessions recorded the same manifest, `b3:2c777bc3...a5ca4`, 29,879 entries.

Raw logs: `0a-cli-cli-arm64-exp-overhead4-r1.log`, `-r2.log` (every run's wall, load before and after, observer counters, ring opcode histogram); generated reports `-r{1,2}-{before,after}.md`. `-explore.log` is the 2-round screen of each candidate on its own (zero-message open, zero-message opendir, negative entries + xattr ENOSYS, all of them) that chose what to keep; two rounds measure nothing, it is there for the request counts.

## Before and after (session r2, all three changes)

| Series | Median | p95 | Paired median | Execution median | Load (1m) during series |
| --- | --- | --- | --- | --- | --- |
| Plain, main session | 15.71s | 15.84s | | 15.71s | 14.6-33.9 |
| Observed, main `2a24fd8` | 17.61s | 18.07s | | 15.90s | 20.1-33.9 |
| **main overhead** | **+12.1%** | +14.1% | +12.5% | +1.2% | |
| Plain, branch session | 15.65s | 15.70s | | 15.65s | 27.5-35.3 |
| Observed, `exp/overhead4` | 16.73s | 16.95s | | 15.63s | 27.0-35.3 |
| Observed, `exp/overhead4` with answered opens (`QUOD_FUSE_ZERO_OPEN=0,QUOD_FUSE_ZERO_OPENDIR=0,QUOD_FUSE_XATTR_ENOSYS=0`) | 16.88s | 17.15s | | 15.71s | 27.0-35.3 |
| **exp/overhead4 overhead** | **+6.9%** | +8.0% | +7.6% | -0.2% | |
| exp/overhead4 with answered opens | +7.8% | +9.2% | +7.8% | +0.4% | |

Per round (paired), r2: main +14.9, +12.6, +10.6, +12.5, +13.1, +11.5, +12.5, +12.1, +13.7%; exp/overhead4 +8.1, +6.5, +5.5, +7.6, +6.5, +7.9, +7.3, +8.2, +7.9%. Runs (s): main observed 18.1, 17.8, 17.5, 17.5, 17.7, 17.5, 17.8, 17.5, 17.6 against plain 15.7, 15.8, 15.8, 15.5, 15.7, 15.7, 15.8, 15.6, 15.5; branch observed 16.7, 16.7, 16.6, 16.7, 16.6, 16.8, 16.8, 17.0, 16.9 against plain 15.5, 15.7, 15.7, 15.5, 15.6, 15.5, 15.7, 15.7, 15.7.

Session r1 (an hour earlier, branch without the CAS link change): main +15.9% median, +15.9% p95, paired +16.1% (plain 15.96s, observed 18.50s); branch +10.1% / +7.7% / paired +9.3% (plain 15.57s, observed 17.15s); answered opens +11.2% / +9.5% / +11.3%. The difference between r1's and r2's branch numbers (0.42s) is the output move (`store outputs` 0.60s -> 0.28s).

## Where the observed run's time goes (median, r2)

| Phase | main | exp/overhead4 | Change |
| --- | --- | --- | --- |
| setup (store, environment pin, key, ledger) | ~0.62s (in "unaccounted"; 0.61-0.88s measured with the phase split) | 0.33s | concurrent fingerprint + probe |
| import (workspace and dependency trees into the CAS) | 0.21s | 0.21s | |
| lookup | 0.00s | 0.00s | |
| mount (observer up 0.03s, overlay 0.00s, sealed snapshots 0.01s) | 0.04s | 0.04s | |
| execution | 15.90s (+0.19s over plain) | 15.63s (-0.02s) | zero-message opens |
| output diff | 0.61s | 0.32s | |
| - store outputs (hash 838 MB of build cache, move ~10k new blobs, one syncfs) | ~0.57s | 0.28s | link + unlink |
| - unmount, manifest | | 0.01s, 0.01s | |
| ledger write | 0.20s | 0.14s | one manifest insert, not two |
| cleanup (run dir) | (unaccounted) | 0.06s | |
| unaccounted (process start, exit) | 0.64s (incl. setup, cleanup) | 0.01s | |
| **observed - plain** | **1.90s** | **1.08s** | |

What the step asked the observer for (median per run):

| | Ring requests | Lookups (absent) | Opens | Releases | Opendir / releasedir | Readdirs | Getxattr | Reads | Serving CPU user / sys | Serving threads' run-queue wait |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| main (passthrough) | 70,525 | 3,243 (621) | 26,296 | 26,296 | 3,299 / 3,299 | 5,462 | 2,623 | 0 | 533 / 5,714 ms | 197 ms |
| exp/overhead4 (zero-message) | 6,505 | 3,243 (621) | 1 | 0 | 1 / 0 | 828-834 | 1 | 2,049-2,428 | 519 / 4,972 ms | 59 ms |

(Opcode histogram from the observer event's `ring_ops`, e.g. main `1:3243,3:1,14:26296,17:2,18:26296,22:2623,23:1,25:1,27:3299,28:5462,29:3299,39:1,52:1`, branch `1:3243,3:1,14:1,15:2049,17:2,22:1,23:1,25:1,27:1,28:828,39:1,52:1`.)

## Changes and their evidence

1. **Zero-message opens and opendirs, xattr ENOSYS** (commit 1). Requests -91%, run-queue wait of the serving threads 197 -> 59 ms. On this quiet box execution was already within noise of plain (main +1.2%), so the measured gain is small (+7.8% -> +6.9%/+7.6% paired, within one session); its value is that the step no longer waits on 64k round trips, which is what made loaded runs +22-59% in exp/overhead3. Not measured under load here (the other agents were held off by the lock).
2. **Setup** (commit 2): fingerprint sections concurrently (480 -> 270 ms, output byte-identical in 5/5 comparisons) and the sandboxed platform probe beside it: setup 0.61-0.88s -> 0.33s. The duplicate manifest insert: ledger write 0.20 -> 0.14s.
3. **Output move** (commit 3): `rename(2)` across directories serializes on the filesystem's rename mutex; 10k action entries of the Go build cache are new every run. Instrumented single runs: moves 380-408 ms -> 31-38 ms, hashing 50-185 ms, syncfs 10-17 ms.

## Tried, not kept

- Negative entries for absent lookups (node id 0 with the entry TTL): lookups stayed at 3,243 and absent lookups at 621 with it on; cli/cli's absent lookups are all distinct, so there was nothing to cache. Removed.
- Faster hashing of outputs: a standalone BLAKE3 pass over a fresh 806 MB Go build cache (16 threads) takes 0.10s on its first read; hashing was never the cost, the renames were.
- READDIRPLUS: not tried in code. It would make every child of a listed directory known to the kernel without a LOOKUP, so either those lookups go unrecorded (under-record) or readdir must record every child (an over-record of each listed directory's files, which would cost hit rate). Rejected on the argument.
- Larger max_write/max_pages: the mount is read-only and reads are now ~2k per run; no lever.

## What the remaining 1.08s is, and what it would take

- setup 0.33s: the fingerprint's critical path is `sha256sum` of the 123 MB node binary (0.21s, single-threaded); the platform probe runs beside it. Next: a content hash cached by (dev, inode, size, mtime, ctime) of the toolchain binaries in the store, or a parallel hash (a key change for every host). Either is a pin-integrity decision, not taken here.
- store outputs 0.28s: hashing the 838 MB build cache and 10k links. The build cache is an ignored output (never compared, never written by `--outputs` or the runner's replay), yet it is recorded; not recording ignored outputs would remove most of this and shrink the result JSON (ledger write). That changes what a result contains, so it is left for a decision.
- import 0.21s: rehashing ~30k workspace and dependency files each run; a stat cache keyed by (dev, inode, size, mtime, ctime) would skip it on unchanged files, at the cost of trusting ctime for inputs (the false-hit invariant's foundation).
- ledger 0.14s, cleanup 0.06s, mount 0.04s.
- Execution: within noise of plain on a quiet box.

## Gates

Details in DECISIONS.md (exp/overhead4 gates).

On 2efeba4, raw logs `0a-cli-cli-arm64-exp-overhead4-gate-*.log`:

- scripts/lint.sh: ok before each code commit.
- scripts/merge-gate.sh (io_uring enabled): ok.
- cli/cli acceptance: A1-A6, P1 ok; A7 0 false hits (23,939 f + 5,320 d + 621 a mutations); A7b 10/10; W1-W4, T1-T11, ring T7 ok.
- luxon (node) and starlette (python): A1, A2 ok, P1 fails on output nondeterminism (Jest order and coverage timestamps; pytest's duration line), identically on main 2a24fd8. Manifests: luxon's identical on main and branch; starlette's five runs (main x2, branch x2, branch with answered opens) differ only in pytest's random tempfile probe (`-python-manifests.log`).

## Rebased onto 0652921

main moved to `0652921` (exp/git-index: the observer serves a `.git` with its index refreshed for the served files, path-derived inode numbers, step key v7). Two consequences here:

- **The withheld-name scan needs every open.** exp/git-index added `Observed::opened` (files whose bytes were served), which quod-hooks' withheld-name scan uses to exempt files the step only stat()ed. Zero-message opens deliver only the first OPEN and then READs, so that set would shrink to the files actually read. The access log is unaffected (see `CasFs::record`), but the scan's exemption would widen, so quod-hooks serves with `zero_open: false` (its zero-message opendirs and xattr ENOSYS stay); `quod run`, which has no such scan, keeps zero-message opens. The mode test now also asserts that, for workloads that read what they open, the opened set is the same in all three modes.
- **Import grew** from 0.21s to 0.51-0.62s (main's own phase). `served_git` (the commit's objects, by a shallow fetch into the store unless the state dir has a memo; the bench's fresh state dirs never do) depends only on the checkout and the state dir, so it now runs on a thread beside the workspace import (commit `d2cdd23`): import 0.51s (r4) -> 0.38s (r5).

Sessions on the rebased tree (raw: `-r4.log`, `-r5.log`, reports `-r{4,5}-{before,after}.md`):

| Session | Binary | Plain median | Observed median | p95 overhead | Median overhead | Paired | Load (1m) during |
| --- | --- | --- | --- | --- | --- | --- | --- |
| r4 (VM just restarted, faster) | main 0652921 | 14.06s | 16.34s | +10.4% | +16.2% | +16.4% | 14-32 |
| r4 | branch 6d8928a (no import change) | 14.05s | 15.67s | +11.7% | +11.5% | +11.7% | 25-32 |
| r4 | branch, answered opens | | 15.68s | +12.1% | +11.7% | +11.9% | |
| r5 | main 0652921 | 15.59s | 17.90s | +16.3% | +14.8% | +15.7% | 24-33 |
| r5 | **branch d2cdd23** | 15.42s | 16.99s | +8.0% | **+10.2%** | **+9.5%** | 26-36 |
| r5 | branch d2cdd23, answered opens | | 17.01s | +8.0% | +10.3% | +9.5% | |

(r3, the first session after the rebase, is not used: from its third round on every run, plain included, took ~24% longer, 15.6s -> 19.4s, a box-wide slowdown with the lock held; its raw log is kept as `-r3.log`.) Load before each session's first run: r4 0.4, r5 15.5 (the merge gate and acceptance had just finished under the same lock).

Phases, r5 median: setup 0.32s, import 0.38s, mount 0.05s, execution 15.59s (+0.17s over plain), store outputs 0.39s, unmount 0.01s, manifest 0.01s, ledger 0.14s, cleanup 0.06s, unaccounted 0.01s; observed - plain = 1.57s. Main r5: import 0.62s, output diff 0.69s, ledger 0.19s, unaccounted (setup and cleanup) 0.61s, execution +0.30s.

Manifest identity on the rebased tree: all 30 observed runs of r5 (main, branch, branch with answered opens) and all 30 of r4 recorded `b3:e0c80a33...ed5ff0` (29,937 inputs). Requests: 6,117 ring requests against 70,892 with answered opens; run-queue wait of the serving threads 9 ms against 188 ms.

Remaining 1.57s and what it would take: as above (setup's node-binary hash; recording the build cache, an ignored output; rehashing ~30k files per import), plus the `.git` served per run: in CI with a persistent state dir the objects memo hits and most of it goes; the bench's fresh state dir pays the shallow fetch every run.

Gates on the rebased tree (d2cdd23, `-rebased-*.log`): lint ok on every rebased code commit; cli/cli acceptance A1-A6, P1, A7 0 false hits (23,983 f + 5,336 d + 620 a), A7b 10/10, W1-W4, T1-T11, ring T7 ok; merge gate ok on its second run, the first stopped in lint on an intermittent workspace test failure that main 0652921 shows too (`-rebased-lint-flake.log`; DECISIONS has the detail).
