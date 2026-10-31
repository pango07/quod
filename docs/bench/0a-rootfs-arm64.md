# Rootfs mode after the review (key v4): portability, acceptance, overhead

Steps run in a pinned root filesystem (`--rootfs`), served from its blobs
by the observer's FUSE server, under a fixed process state, with the host
class in the key (DECISIONS 2026-10-31 exp/rootfs entries). Same box as the
other 0a reports: Docker Desktop, 18 arm64 cores, 7.7 GiB, kernel
7.0.14-linuxkit, shared with three other agents throughout (one-minute load
averages 20-90), so the paired per-round ratios are the better estimator.
The first version's measurements (key v3, rootfs copied onto the state
filesystem) are in `0a-rootfs-v3-arm64.md`, superseded.

## Presentation (review M1)

With the real rootfs, inside a step: `stat` of `/usr/bin/git` and
`/usr/local/go/bin/go` shows mtime and atime 0 and owner `runner`;
`ls -f /usr/share/zoneinfo` (directory order) equals `ls -a` (sorted);
after `cat /usr/bin/git` its atime is still 0; umask 0022, `SigIgn` and
`SigBlk` 0, NOFILE 1024 soft and 65536 hard.

## Portability (review M2)

`scripts/portability.sh`, transcript in `0a-rootfs-portability.log`. Both
hosts are containers on one VM, so kernel, CPU, RAM, sysctls and seccomp are
equal by construction; what differs is the userland. The claim is that the
key ignores the userland and the result does not depend on it.

| | Host 1 | Host 2 |
| --- | --- | --- |
| Userland | Debian 12, glibc 2.36, Go installed | Ubuntu 24.04, glibc 2.39, no Go |
| bubblewrap | 0.8.0 | 0.9.0 (and 0.8.0 for the replay) |
| Store | shared state, volume ext4 | private, its own loop-mounted ext4 |
| Rootfs import | b3:3500d167 | its own export: b3:3500d167 |

- Host 2 fresh (bubblewrap 0.9.0, private store, empty ledger, its own
  module-cache download): the key differs from host 1's in exactly one
  host-class entry, `sandbox: bubblewrap 0.9.0` against `0.8.0`, so it
  executes. Manifest entries identical (the same 29,878 inputs); exit code
  0 and 0; stdout identical with go test durations normalized. Outputs,
  14,262 each: 4,102 byte-identical; 9,765 Go cache action entries differ
  (they hold a creation time); 664 cache output entries exist in one run
  only (named by content; cached test output holds durations); the rest is
  Go's trim.txt, dated telemetry counters, gh's device id and cache, and an
  ssh key pair a test generates.
- Host 2 with Debian's bubblewrap 0.8.0 first on PATH, against the shared
  ledger: host 1's class, REPLAYED host 1's proof (same key and manifest).
- The `--extra jq` rootfs misses (no manifests for its key); `--host-root`
  on host 2 is refused (no Go to fingerprint).

## Acceptance

`scripts/acceptance.sh` with `STILLPASS_ROOTFS`, log in
`0a-rootfs-acceptance.log`: A1-A6 pass; A7 mutates 23,939 files, 5,320
listings and 620 absent paths of the 29,879-entry manifest with 0 false
hits; A7b 10/10, each fresh execution reading exactly the replayed
manifest's paths; W1-W4 pass (29,843 warm inputs with an empty and a full
cache, 0 hidden).

## Overhead, host root vs rootfs

`stillpass bench --no-hit-rate --interleave --runs 7 --step "go test -skip
TestHuhPrompterConfirm ./..." --rootfs-src
/work/rootfs/go1.27.3-bookworm-arm64.tar --extra-series
STILLPASS_ROOTFS=b3:3500d167...`: plain (same sandbox, host root, no
FUSE), observed on the host root, observed in the served rootfs, one run of
each per round, first round dropped. The step skips cli/cli's
`TestHuhPrompterConfirm`: the first attempt of this bench stopped when it
hung to go test's 10-minute timeout in a host-root run (the flaky test of
DECISIONS; a bench refuses a run that is not a passing miss). Per round
(wall seconds; load before the host-root run):

| Round | Load | Plain | Host root | Rootfs | Rootfs / host root (wall) | (execution) |
| --- | --- | --- | --- | --- | --- | --- |
| 2 | 87 | 40.4 | 37.6 | 35.3 | 0.94 | 0.96 |
| 3 | 69 | 30.9 | 40.8 | 64.0 | 1.57 | 1.59 |
| 4 | 72 | 29.2 | 39.5 | 43.0 | 1.09 | 1.11 |
| 5 | 61 | 31.3 | 59.4 | 42.7 | 0.72 | 0.71 |
| 6 | 69 | 36.3 | 46.6 | 38.2 | 0.82 | 0.82 |
| 7 | 79 | 42.6 | 50.2 | 52.3 | 1.04 | 1.06 |

Median of the paired ratios 0.99 (wall) and 1.01 (execution); medians
42.85s rootfs against 43.69s host root; minima 35.3s against 37.6s. At load
averages of 60-90 the two modes cannot be told apart, and the claim is only
that: rootfs mode costs no more than the noise of this box. What it costs in
requests, independent of load (one run, observer events): the served root
answers about 34k requests per `go test ./...` (10,364 lookups of which
5,812 absent, 21,780 opens, 1,842 readdirs, no reads: passthrough), about as
many as the workspace observer (35k); at the 50-80 µs per round trip
measured on this VM that is 2-3 s of request latency spread over 18
parallel workers. The first version's copy cost nothing per run but
leaked the host filesystem (review M1). Host root against plain in the
same bench: +29.3% wall, +19.7% execution, both inflated by the load.

The raw report follows.

### stillpass bench report

Spike repo `https://github.com/cli/cli` at `9490d204a2ba`. Step A `go mod download`, step B `go test -skip TestHuhPrompterConfirm ./...`. Generated by `stillpass bench`.

#### Machine

| CPU | Cores | RAM | Disk (state dir) | Kernel | Arch |
| --- | --- | --- | --- | --- | --- |
| 0x000 | 18 | 7.7 GiB | ext4   /dev/vda1[/docker/volumes/stillpass-work-rootfs/_data] | 7.0.14-linuxkit | aarch64 |

#### Overhead (A8)

7 runs per series, interleaved, first dropped as warm-up. Overhead = observed median / plain median - 1. Target under 10%.

| Series | Median | p95 | Execution median | Runs (s) | Load average (1m) before -> after each run |
| --- | --- | --- | --- | --- | --- |
| Plain (same sandbox, no FUSE or overlay) | 33.79s | 42.58s | 33.79s | 40.4, 30.9, 29.2, 31.3, 36.3, 42.6 | 87.7->87.0, 70.9->68.8, 84.5->71.9, 69.4->61.4, 78.5->68.7, 69.2->78.9 |
| Observed (import, serve, overlay, sandbox, record) | 43.69s | 59.37s | 40.46s | 37.6, 40.8, 39.5, 59.4, 46.6, 50.2 | 87.0->75.7, 68.8->67.9, 71.9->67.6, 61.4->82.9, 68.7->73.3, 78.9->89.0 |
| Observed, STILLPASS_ROOTFS=b3:3500d167 | 42.85s | 63.97s | 39.73s | 35.3, 64.0, 43.0, 42.7, 38.2, 52.3 | 75.7->70.9, 67.9->84.5, 67.6->69.4, 82.9->78.5, 73.3->69.2, 89.0->90.0 |
| **Overhead** | **+29.3%** | +39.4% | +19.7% | | |
| Overhead, STILLPASS_ROOTFS=b3:3500d167 | +26.8% | +50.2% | +17.6% | | |

Observed phase breakdown, median over the same runs:

| Phase | Median | Share of observed |
| --- | --- | --- |
| import | 1.02s | 2.3% |
| lookup | 0.00s | 0.0% |
| mount | 0.06s | 0.1% |
| execution | 40.46s | 92.6% |
| output diff | 1.06s | 2.4% |
| ledger write | 0.15s | 0.3% |

#### Totals

- Observer overhead (0a, host): median +29.3%, p95 +39.4% (plain 33.79s, observed 43.69s)
