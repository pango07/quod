# exp/misses: making a miss cheap

Spike repo `https://github.com/cli/cli` at `9490d204a2ba`, step B `go test ./...`.
Same box as `0a-cli-cli-arm64.md` (Docker Desktop, arm64, 18 cores, kernel
7.0.14-linuxkit), but shared with two other agents for the whole session, so
absolute times are 20-60% above that report's and vary run to run. Every
comparison below is between runs interleaved within one loop; nothing here
should be read against the quiet-box numbers except the phase breakdowns.

## 1. The output tax (every miss)

`Overlay::finish` used to copy ~840 MB / 14k files of Go build cache into the
CAS with an fsync per blob. It now hashes them in place, flushes the store's
filesystem once with syncfs(2), and renames the new files to their blob paths
(`Cas::take_files`). Base binary (commit 3eb2f63) and new binary interleaved,
ledger cleared before every run so every run is a miss, CAS persisting per
binary. `diff` is the output-diff phase from the run event.

| Pair | Base diff | New diff | New breakdown (unmount / walk / store / syncfs) | New bytes |
| --- | --- | --- | --- | --- |
| 1 (empty CAS) | 2.56s | 0.62s | 0.02 / 0.05 / 0.49 / 0.02 | 839 MB |
| 2 | 3.55s | 0.84s | 0.39 / 0.07 / 0.24 / 0.07 | 3.3 MB |
| 3 | 3.61s | 1.16s | 0.28 / 0.07 / 0.62 / 0.10 | 3.3 MB |
| 4 | 3.54s | 1.37s | 0.54 / 0.07 / 0.55 / 0.12 | 3.3 MB |
| 5 (heaviest load) | 3.35s | 2.65s | 0.49 / 0.11 / 1.13 / 0.34 | 3.2 MB |

Steady state (CAS populated) is pairs 2-5: only the ~7k cache entry files Go
rewrites every run are new, so the base's cost was never the copy but ~7k
fsyncs. The new path's remaining cost is the overlay unmount (overlayfs syncs
its upper on unmount: the writeback of the 840 MB Go wrote, which has to
happen somewhere) and hashing 840 MB from the page cache; the syncfs itself
is 20-120 ms because the unmount already flushed most of it.

In the later `stillpass bench` run (6 observed runs, same loaded box) the
output diff phase was 0.75s median, 3.9% of the observed run, against 1.74s
and 8.8% in the T12 report on a quiet box.

## 2. Warm state (`--warm`)

The Go build cache from earlier warm runs persists as a host directory
(`<state>/warm/<image>/gocache`) bound over GOCACHE inside the sandbox, on
top of the overlay. Not observed, not an output, never copied. Warm runs have
their own step key: GOFLAGS gains `-count=1` and GODEBUG=goindex=0 (see
DECISIONS.md for why both of Go's path-keyed caches must be off).

`stillpass bench --one-file --runs 6`: `api/client.go` (imported by nearly
every package) gets a comment appended before each pair, so every run is a
miss; cold and warm interleaved; first pair dropped (for warm it is the run
that fills the cache).

| Series | Median | p95 | Execution | Output diff | Runs (s) |
| --- | --- | --- | --- | --- | --- |
| Cold miss | 20.72s | 28.75s | 19.36s | 0.78s | 28.8, 26.5, 19.4, 19.2, 20.7 |
| Warm miss | 9.24s | 11.82s | 8.67s | 0.07s | 11.7, 11.8, 9.1, 9.2, 9.1 |
| **Warm / cold** | **0.45x** | | | | |

A change to `api` still relinks and re-runs most test binaries; what the warm
cache removes is compiling the standard library and the ~400 dependency
packages. A leaf-package change would do better; a go.mod change worse.

Smoke run on the same box (single runs, not medians): same tree, cold 27.8s
vs warm-with-empty-cache 27.2s (the warm env costs nothing by itself); one
file changed, cold 24.6s vs warm 15.3s; second change warm 11.9s; replay of
the warm key 0.7s. Warm outputs: 26 entries, ~70 KB (Go telemetry counters
under HOME and the empty GOCACHE mount point) against 14,262 entries and
840 MB cold.

## 3. What the warm run reads

Manifest sizes on cli/cli, same tree:

| Env | Cache | Inputs recorded |
| --- | --- | --- |
| cold (Schemas env) | empty, every run | 21,051 |
| warm env, module index on (rejected) | empty | 21,049 |
| warm env, module index on (rejected) | full | **7,101** |
| warm env, goindex=0 and -count=1, before the vet over-record | empty | 9,046 |
| warm env, goindex=0 and -count=1, before the vet over-record | full | 9,023 |
| warm env as shipped (plus the vet over-record) | empty | 9,536 |
| warm env as shipped | full | 9,531 |

With Go's module index on, a full cache hid 14k inputs from the observer:
the index caches every module's file list in GOCACHE keyed by the module's
path, and a warm run never re-reads those directories. With it off, the 23
entries present only with an empty cache are the assembler's
`#include "textflag.h"` probes beside `.s` files (absent lookups that do not
happen when the asm action is cached); each parent directory is listed in
both manifests, so a matching listing already implies the absence. The
acceptance suite's W4 asserts exactly this coverage on every run.

On the first full acceptance run W4 then found 18 `f` entries hidden by a
full cache: build-constraint-excluded files of the main module
(`internal/flock/flock_windows.go`, `internal/licenses/embed_darwin_amd64.go`,
`internal/prompter/echo_darwin_test.go`, ...). go test caches each package's
vet verdict keyed by the compiled files, while vet's buildtag analyzer also
parses the ignored files; a cached verdict is reused whatever they contain.
Warm runs now over-record every Go-toolchain source file in every listed
directory, which puts those files in both manifests.

The cold env's 12k extra entries are the module indexer walking whole
modules (`.github`, `testdata`, nested `go.mod` probes) plus go test's
test-cache input hashing (an lstat of every child of every directory a test
opened): inputs of Go's caches, not of the build.

## Acceptance

`scripts/acceptance.sh /work/cli` on the final code: A1-A7 pass; A7 mutated
12,480 f, 4,548 d and 4,024 a entries with 0 false hits; A7b 10/10 unread
files mutated, replay agrees with a fresh execution; W1-W3 warm agrees with
cold on the same tree and after a one-file change and the warm key replays;
W4 9,536 inputs with an empty cache against 9,531 with a full one, the 5
being asm include probes covered by their parent listing, 0 hidden.
`cargo test --workspace` in the dev container (FUSE and sandbox integration
tests, the tiny-module acceptance with W) passes.
