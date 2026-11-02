# Process-level memoization: design and upper bound

Research spike, branch `exp/process-memo`. The question: can Quod memoize the
individual processes a step runs (a package's compile, a link, a test binary,
`tsc`, a pytest worker), automatically and without configuration, so that a
step whose inputs changed still replays the parts of itself that did not?
That is the ground language caches own today, and the 0b benchmark
(`docs/bench/0b-x86-hetzner.md`) is the reason to ask: on 40 real cli/cli
commits Quod's whole-step replay lost to a warm Go cache on every commit,
because a typical commit changes one file and Go re-runs a few packages
while Quod re-runs the step.

The measurement comes first (section 1), because the parked `exp/split`
branch already tried per-test-binary memoization on cli/cli and found it a
net loss. Section 2 is the design. Section 3 is the recommendation.

## 1. Upper bound, measured

Raw data and drivers: `docs/bench/process-memo/`. Host: the Hetzner CCX23 of
the 0b benchmark (4 dedicated vCPUs, 16 GB), Go 1.27.3, the same 40
first-parent commits of cli/cli ending at `9490d204a2ba`.

### 1.1 Method

Instead of a FUSE-attributed process tree (section 2.2 explains why the
current observer cannot give one), every process the Go command runs was
wrapped: `-toolexec` for compile, asm, vet and link, `-exec` for each
package's test binary, both pointing at `pmwrap`
(`docs/bench/process-memo/pmwrap.go`). The wrapper runs the tool, times it,
and computes a key from exactly what the tool reads:

- compile: argv (with `$WORK` normalized) and the content of every file named
  on argv, in `-importcfg` (the dependency archives) and in `-embedcfg`;
- link: argv and the content of every `packagefile` in `-importcfg` (the
  whole dependency closure, ~270-440 archives, 140-250 MB) and of the main
  archive;
- vet: argv, `vet.cfg` and every file it names;
- test binary: the binary's content, argv, cwd, and the Go `testing`
  package's own access log (`-test.testlogfile`, which the wrapper injects
  since `-count=1` turns it off): every `getenv` value, and the content of
  every path it opened or stat'ed (type for directories, `absent` for
  misses). Paths under `/tmp` and `/dev`, and paths under `$HOME` that did
  not exist before the unit started, are the unit's own scratch and are
  counted, not keyed.

Two keys per process: `key` (paths and contents: what a generic,
tool-agnostic memo would see) and `ckey` (contents only, with `$WORK` and
`GOCACHE` paths dropped from argv: what a tool-aware memo such as Go's own
cache keys on). The difference matters for links: after a package is
recompiled, its dependents' `importcfg` names the fresh archive under
`$WORK` in that run and under `GOCACHE` in the next, so a path-keyed link
misses twice for one change.

The driver (`pm_history.py`) runs each commit as uid 1001 on a fresh
checkout under exactly Quod's warm-miss conditions: one GOCACHE persisting
across commits, `GOFLAGS -count=1`, `GODEBUG=goindex=0`, a fresh HOME per
job; `go build ./...`, `go vet ./...`, `go test ./...`. Nothing is skipped:
the records say what a memo *could* have skipped. The last commit is then
run a second time on the same tree to measure key stability, and every test
binary once more under `strace -f -e execve` to find the ones that spawn
subprocesses. The offline replay (`pm_analyze.py`) walks the commits in
order keeping a store of keys: a process is a **hit** when its key was
produced on an earlier commit, **proven** when on two (the shipped trust
rule, proofs 2). Go's own verdicts come from the 0b plain-warm logs' `ok ...
(cached)` lines.

Caveats of the method, stated up front:

- The wrapper's hashing (~0.15 s per link for ~200 MB of archives) runs
  after the tool exits and is excluded from the process's wall time, but it
  loads the box, so the instrumented step walls are ~10 s above the bench's;
  use the bench's Quod execution times as the step reference.
- The test-unit read set is the Go testing package's log, which sees opens
  and stats made through `os` in-process and nothing a subprocess does.
  The strace survey bounds that gap.
- The wall-time estimate ("with hits free") removes a hit's wall divided by
  the step's measured parallelism (process-seconds per wall second). It is
  an upper bound: replay is not free, and the Go command's own serial work
  (loading, scheduling, output) does not shrink.

### 1.2 Where a Quod miss spends its time

Over commits 2-40, `go test ./...` under Quod's warm-miss conditions ran
30,000 processes. Their serial wall time, by kind:

| Kind | Processes | Serial wall | Share |
| --- | --- | --- | --- |
| link (one test binary per package) | 10,156 | 4,848 s | **75%** |
| test binary run | 10,082 | 888 s | 14% |
| compile | 4,654 | 491 s | 8% |
| vet | 5,105 | 276 s | 4% |

The step is a *linker* benchmark. With a warm GOCACHE, Go's cache answers
every unchanged compile and vet; `-count=1` makes it re-link and re-run every
one of the 259 test binaries, and a cli/cli test binary links in ~0.45 s on
this box (140-250 MB of archives in). The test runs themselves are 22 s
serial per commit, and 3 s of that is one package (`pkg/cmd/repo/create`).
This is what `exp/split` found the hard way: memoizing the test *runs* alone
recovers 14% of the process time.

`go build ./...` (156 links, 2,871 compiles) and `go vet ./...` (5,104 vets)
had **zero** hits: every process that runs there is one Go's own cache
already decided to run, and `go build`'s links stamp the commit hash. There
is nothing for a process memo to add on those two steps.

### 1.3 What a process memo could skip

Offline replay of the keys over the history, `go test ./...`, commits 2-40
(path-keyed `key`; content-only `ckey` in parentheses):

| Kind | Hit: key seen on an earlier commit | Hit wall | Proven: seen on two | Proven wall |
| --- | --- | --- | --- | --- |
| link | 5,755 of 10,156, 57% (58%) | 2,561 s, 53% (54%) | 4,680 (4,932) | 2,036 s (2,115 s) |
| test | 7,481 of 10,082, 74% (76%) | 643 s, 72% (74%) | 5,521 (5,895) | 470 s (496 s) |

Step wall, summed over commits 2-40:

| | `go test ./...` |
| --- | --- |
| Instrumented run (this measurement; ~10 s per commit above Quod's from the wrapper's hashing) | 2,315 s |
| 0b bench, Quod miss (execution) | 1,804 s |
| 0b bench, plain with a warm Go cache | 943 s |
| **Estimate with every hit replayed free** | **1,164 s** (50% less than instrumented; ~900 s scaled to Quod's execution time) |
| Estimate with only proven units replayed free (proofs 2) | 1,415 s (39% less) |

So the upper bound for a per-process memo on `go test ./...` is roughly
**parity with a warm Go cache over the whole history**, not a win over it,
and it splits two ways by commit:

- **Quiet commits** (a leaf package, docs, CI, a dependency bump that the
  sealed module cache absorbs: 24 of 39): 86-100% of process time is in
  hits; the estimate leaves 0.1-3 s of process time per commit against a
  plain-warm step of 10-14 s. Here a process memo beats Go's cache, because
  Go re-runs the same 38 "leak" packages on every commit (5 s serial, mostly
  `repo/create`) and re-links them, and a content key does not.
- **Heavy commits** (`api`, `internal/config`, `git`, a `go-gh` bump: 8 of
  39): 0-8% hits. Everything legitimately re-links and re-runs; the step
  costs 77 s instrumented, 62 s under Quod, 56 s plain-warm. A process memo
  saves nothing, and Quod's `-count=1`/`goindex=0` and observer overhead
  still cost 6 s over plain.
- In between (a shared package: 7 of 39): 13-61% hits.

Why links miss when nothing relevant changed (link hits 57% against test
hits 74%): **a link misses twice per change.** After a package is
recompiled on commit *k*, its dependents' `importcfg` names the fresh
archive under `$WORK` on commit *k* and under `GOCACHE` on commit *k+1*, and
the two files differ in content (146,856,698 against 146,856,808 bytes for
`api.test`'s inputs at commits 31 and 32: Go rewrites an archive's build id
when it stores it), so even the content-only key misses on *k+1* while the
linked binary is byte-identical (tests hit 259 of 259 on commit 32 while
links hit 34 of 261). The memo on the *test unit*, keyed by the binary's
content, recovers what the link memo loses downstream; the link's own time
on *k+1* is lost. Go's cache has the same input difference and does not
care, because it never caches links.

Why test units miss when their package did not change: 1,592 because the
binary changed (a dependency was recompiled), 1,007 (790 with the content
key) because argv changed, which is Go's `$WORK/bNNN` action numbering
shifting when the package graph changes: a path-keyed memo must normalize
those names or lose ~10% of hits.

### 1.4 Against Go's own test cache

Each test unit on commits 2-40, by what the content memo and Go's cache
decided for it (Go's verdict from the 0b plain-warm run of the same commit):

| | Go cached it | Go re-ran it |
| --- | --- | --- |
| memo hit | 6,417 units, 501 s | **1,064 units, 142 s** |
| memo miss | 440 units, 37 s (241, 23 s content-keyed) | 2,161 units, 208 s |

The win over Go is the top-right cell: 1,064 test runs (142 s serial, 88 s
of it `repo/create`, 3 s on every commit) plus their links (~480 s serial)
that Go re-runs and a content key does not. The bottom-left cell is the
loss: 440 units Go cached that the memo missed, all of them (reasons above)
test binaries whose bytes changed because a dependency was recompiled while
Go's action id, which hashes dependency *ids* rather than archive bytes,
did not. Go's key is coarser there and still sound.

### 1.5 Key stability, subprocesses, failures

- The last commit run twice on the same tree: 517 of 518 `go test ./...`
  processes produced the same key (the one exception a link of `git.test`);
  `go build ./...`'s 4 links never match across runs (VCS stamping).
- 11 of 259 test binaries exec a subprocess (strace survey: `git` runs 665
  `git`s; `pkg/cmd/repo/sync` 109; `gitcredentials`, `codespace`,
  `extension`, `surveyext`, `issue/create`, `internal/prompter`,
  `skills/search`, `pr/merge`, `internal/skills/installer`). The Go testing
  log sees none of what those subprocesses read; a memo must treat them as
  opaque or attribute their reads (section 2.2).
- No test unit failed or was killed in the 40 commits (the
  `TestHuhPrompterConfirm` hang `exp/split` met under load 30-110 did not
  occur at load 7).

### 1.6 The prototype (section 2 and `docs/bench/process-memo/pmemo.go`)

A standalone `-toolexec`/`-exec` shim that memoizes links (key: argv plus
every input's content, known before the link runs) and test binaries
(lookup by binary content, argv and cwd; then the recorded manifests
matched against the tree, as Quod matches step manifests), proofs 2,
failures never prove, a unit that stats an executable or leaves an orphan
is opaque. Run on the 40 commits with a fresh store, `go test ./...`,
paired with a plain-warm run on each commit:

| Commits 2-40 | Paired plain-warm | Prototype v1 | 0b Quod miss |
| --- | --- | --- | --- |
| `go test ./...` total | 1,048 s | **1,735 s** (+66%; slower on 39 of 39) | 1,804 s |
| Quiet commit (33-36) | 12.5-13.0 s | 16.7-17.1 s | 40.4-41.0 s |
| Heavy commit (7, 15, 31) | 65-69 s | 82-84 s | 62 s |

Over commits 2-40 the prototype replayed 4,675 of 10,156 links and 5,626
of 10,082 test units; 468 units were opaque (12 per commit: the 11
subprocess spawners plus `internal/prompter`'s `echo` probe), 1,723 test
units and 1,063 links ran as the second proof. Where a quiet commit's 17 s
went: the link lookups hash each link's ~200 MB closure before deciding
(~40 s serial, ~10 s of wall on 4 vCPUs: the generic cost of content-keying
inputs that live outside a content-addressed store, which Go avoids by
putting the hash in the file name), materializing 259 replayed binaries by
copy (2.0-3.9 s serial), the 12 opaque units (~4 s serial, `prompter` alone
2.3 s), and the Go command's own loading and scheduling under `goindex=0`.
A second version with hashes cached by inode identity for write-once files
(`hashFileCached`) cut the all-replay smoke run from 3.2 s to 1.0 s on six
packages; its 40-commit run is queued behind another agent's history on the
box and its numbers go into `docs/bench/process-memo/prototype-v2.md` when
it lands.

Mutation tests at the unit level (`pm_mutate.py`), v1: 40 proven test units
had one recorded workspace input corrupted (or a recorded absent path
created) and were run through the shim in replay mode, then plain: in all 40
the shim's exit code and verdict equalled the plain run's, and in 31 the
mutation made the test fail identically on both sides, which a replay could
not have produced. The harness lost the shim's per-unit outcome log (it wrote
to a root-owned directory), so "replayed or ran" is inferred from the
verdicts, not read: 0 false hits is established for the 31 units whose
mutation changed the result, and the 9 whose result was unchanged (the
mutated fixture did not matter to the test) are unproven either way. The
A7b sample (10 unread files mutated) agreed with plain in all 10 for the same
reason. The v2 run repeats both with the log in place.

## 2. Design

### 2.1 The unit: an exec'd process and its subtree

A **unit** is one `execve` inside the step and everything below it in the
process tree until the exec'd process exits: a `compile` (no children), a
`link`, a test binary and the `git` it spawns, a pytest worker, `tsc`. Units
nest: the `go` command is a unit whose children are units; the step itself
is the root unit, and whole-step memoization is the degenerate case. A unit
is a candidate for memoization when it exits normally, every descendant has
been reaped, and its interaction with the rest of the step went through
things Quod can record and reproduce: files, argv, env, cwd, exit code, the
bytes it wrote to stdout and stderr, and the files it wrote. Units whose
interaction went through anything else (a socket to a sibling, shared
memory, a pipe that is not stdout or stderr, a signal, a daemon left behind)
are recorded as **opaque** and never memoized; their parent's unit is then
opaque too, up to the step, which falls back to step-level memoization
exactly as today.

The **unit key** is the step key plus: the exec'd binary's content hash (or
its path when it is outside every observed tree: `/usr/local/go/pkg/tool`
is pinned by the image digest), argv, cwd, the environment, the parent
unit's key (so a unit is only ever replayed inside the same context: an
`eslint` spawned by `make lint` and the same `eslint` spawned by `npm test`
are different units), and a description of the inherited descriptors: which
of 0, 1, 2 are the parent's pipes or files, and that nothing else is
inherited (`close_range` already guarantees that at the step boundary; a
unit that inherits another descriptor is opaque).

The **unit manifest** is the unit's read set, exactly as a step manifest:
every path the unit's subtree looked up, read, listed or found absent, with
its kind and content, plus the set of paths it wrote. Reads of paths the
same subtree wrote earlier are not inputs (write-before-read is the unit's
own scratch). Reads of paths a *sibling* wrote earlier in the step are
inputs with that sibling's output as their content, which is how a link's
dependence on a compile is expressed without knowing what a link is.

The **unit result** is the exit code, stdout and stderr blobs, and the
written files (path, mode, content hash, or a deletion), applied to the
overlay on replay. Duration is recorded for the dashboard, never replayed.

### 2.2 Attributing reads to processes

The FUSE in-header carries the requesting thread id, and every recorded
access already carries it (`Req { pid }`, DECISIONS 2026-10-31); the
observer event reports the set of pids seen. Two things stop that from being
a per-process read set today, and both were measured on `exp/split`:

1. **The kernel answers from its caches without asking.** With zero-message
   opens and `FOPEN_KEEP_CACHE` (exp/overhead4), a file read by one process
   is served to the next from the page cache and its dentry from the dcache;
   only the first requester reaches the observer. `exp/split` had to serve
   files with entry and attribute TTL 0, directories with attribute TTL 0
   and no `default_permissions`, so that every lookup, stat and open of a
   file arrived (directory dentries still cached: a path walk costs nothing,
   a `stat` of a directory costs one GETATTR). That mount measured +3.7
   points of execution over the plain observer on an all-miss `go test
   ./...` (33.17 s against 32.03 s, paired under load), and it was a
   different, faster mount than the first attempt (+20.8%). It still does
   not see a second *read* of a cached page: the open is what is recorded,
   and the open is enough for "this process depended on this file", since
   the content served is the content keyed.
2. **Reads outside the observed tree are invisible.** The warm GOCACHE is a
   host directory bound over the overlay on purpose (DECISIONS: served
   through FUSE every cache probe would be an input and an identical re-run
   would miss). `$WORK` is a tmpfs. A `link` reads 200 MB of archives from
   exactly those two places and nothing from the workspace but its
   `importcfg`. A FUSE-attributed read set for a link is therefore empty,
   and the measurement above had to key links by hashing what `importcfg`
   names.

So attribution needs a second source beside FUSE for paths outside the
served trees. Three candidates:

| | seccomp user-notify on execve + openat family | ptrace (PTRACE_O_TRACEEXEC, seccomp stops) | LD_PRELOAD |
| --- | --- | --- | --- |
| Sees | every exec and open in the sandbox, with the caller's pid, argv (read from the tracee's memory via `pidfd_getfd`/`process_vm_readv`) and the resolved path | the same, plus the ability to rewrite the syscall or its return value (fake the exec's exit) | nothing from Go, which is static and makes raw syscalls; nothing from any static binary |
| Cost per event | one context switch to the supervisor; opens are the hot path (`go test ./...` is ~50k opens with the kernel caches on) | two stops per syscall unless filtered by a seccomp `RET_TRACE` policy; slower than notify | none |
| Interposing a replay | `SECCOMP_IOCTL_NOTIF_ADDFD` can inject a descriptor; the supervisor cannot make `execve` return as if the program ran, so a replay means answering the exec with ENOENT/EACCES... not an option; see below | can rewrite `execve` into an exec of a shim, or stop the tracee and `kill`+report an exit; the parent's `wait` sees whatever the tracee exits with | a hooked `execve` in the parent decides; useless for Go |
| Unprivileged | yes: the filter is installed inside the sandbox by a launcher, the notify fd handed out over a Unix socket; works in the user namespace bubblewrap creates | yes: Quod's uid owns the sandbox's user namespace, so it has CAP_SYS_PTRACE over it (yama scope 1 allows descendants) | yes |
| Windows/macOS | no | no (different APIs) | no |

The choice: **seccomp user-notify for observation, with the exec interposed
in the filter's own process rather than by the supervisor.** The notify fd
cannot make `execve` succeed with a fake result. What it can do is deliver
the exec request before the kernel acts on it; the supervisor then decides
*replay* or *run*. For *run* it answers `SECCOMP_USER_NOTIF_FLAG_CONTINUE`.
For *replay* it needs the process to become the replay: the simplest sound
way is to have every exec go through a shim. Two ways to get a shim in
front of every exec without configuration: (a) `binfmt_misc` is out
(needs root, global); (b) the supervisor answers the exec with an error
that the `exec` syscall is retried through a shim: not possible either,
the caller is `execve` in libc or the Go runtime and will not retry.

Hence the pragmatic mechanism, which is what `exp/split` built for Go and
what every build tool offers in some form: **a per-tool exec hook, found
automatically.** Go has `-toolexec` and `-exec`; `node` has
`NODE_OPTIONS=--require` for in-process hooks and the `PATH` shim for
spawned binaries; pytest-xdist and most runners spawn through `PATH`.
Quod's profile for the toolchain (`quod_key::profile`, where everything
Go-specific already lives) adds the hook to the step env (`GOFLAGS` gains
`-toolexec=/quod/unit -exec=/quod/unit`, a binary bound into the sandbox
like `/quod/started`), and the general case is a `PATH` shim directory
bound first on `PATH` containing a shim for every executable the step
exec'd in an earlier observation (the seccomp log says which). The shim
*is* the unit boundary: it asks the supervisor over a Unix socket (key,
argv, env, cwd, the three descriptors' identities), and either execs the
real binary (run) or writes the recorded stdout and stderr, applies the
recorded writes and exits with the recorded code (replay). seccomp
user-notify stays for *observation*: it is how Quod learns which binaries
the step execs, and how it tells, cheaply, that a unit did something it
cannot memoize (a `socket`, `shmget`, `mmap` with `MAP_SHARED` on a
writable file, a `kill` to a pid outside the subtree, a `clone` whose child
outlives the unit). Those syscalls are the only ones on the notify list
after the first observation; the filter is `RET_ALLOW` for everything else,
so the hot path (open, read, stat) costs nothing, and the read set comes
from FUSE (workspace) plus the hashes of what `importcfg`-like files name,
which brings us to the next point.

**Reads outside the served trees.** For the warm cache and `$WORK`, the
unit's read set must include files FUSE does not serve. Options, in order of
preference:

1. Serve the warm cache through FUSE too, as a separate mount with its own
   log that is *not* part of the step manifest but *is* part of unit
   manifests. The step-level objection (every probe an input, an identical
   re-run misses) does not apply to units: a unit's read set is what *it*
   read, and a link that reads 400 archives whose contents are unchanged is
   the same unit. The cost is the observer's per-request latency on a path
   Go hits hard (Go's cache lookups are `stat`+`open` per action, ~5k per
   step), measured at 50-80 µs per round trip. With zero-message opens off
   on this mount (attribution needs the open), ~10k extra round trips is
   ~0.6 s per step. Acceptable.
2. Hash on open through the notify path: `openat` on the notify list for
   paths under the cache, the supervisor hashing the file once and caching
   the hash by (device, inode, size, mtime, ctime). Go's cache files are
   written once and never modified, so this is sound for them and merely
   slow for a tool that rewrites cache files in place (every open hashes).
3. Trust the tool's naming: Go's cache paths embed the output id, which is
   the content hash. Fastest, and exactly what Go does; also Go-only and a
   trust assumption Quod cannot audit. Rejected as the mechanism, kept as
   the explanation of why Go's own cache is cheap where ours is not.

Option 1 is the design. `$WORK` is different: it holds files *this step*
wrote (compile outputs, importcfg files, test binaries). A unit's read of a
sibling's output is recorded by the overlay diff of that sibling (its
writes are known and hashed when it finishes), so a `$WORK` read is an
input whose content is a sibling unit's output, found by path in the
step's write log. That needs the writes attributed to the *writer* unit,
and in a parallel step (Go runs four actions at once) the overlay diff
alone cannot say who wrote what. Two ways: fanotify on the overlay's upper
dir (`FAN_MODIFY` with `FAN_REPORT_PIDFD` gives the writer's pid per event;
needs CAP_SYS_ADMIN in the initial user namespace, so root), or a per-unit
mount namespace with its own overlay, every unit's writes landing in its
own upper (unprivileged; ~2 ms per mount, so 500 units cost ~1 s). The
prototype sidesteps this for Go, whose link and test units write only their
`-o` output and the test log, both named on argv, and whose compile outputs
reach `$WORK` by Go's own hand. The general design uses the per-unit
overlay only for units the observation phase saw writing files not named
on their argv, which keeps the cost on the units that need it.

### 2.3 Correctness hazards and their handling

Every hazard is handled one of three ways: **key it** (the hazard's value is
part of the unit key, so a difference misses), **record it** (it is in the
manifest or result and replayed), or **opaque** (the unit is never
memoized, and so is every ancestor below the step).

| Hazard | Handling |
| --- | --- |
| Inherited descriptors other than 0, 1, 2 | Opaque. `close_range` at the step boundary guarantees the root unit has none; a unit that inherits one (a pipe to a sibling, a lock file, an eventfd) is detected by the shim (`/proc/self/fd`) and refused. |
| stdout/stderr are pipes the parent reads | Recorded as blobs; replay writes them back. Interleaving between the two is not preserved; a unit whose parent reads both through one descriptor (`2>&1`, the shim sees the same inode on 1 and 2) records them as one stream in order. |
| stdin | The shim gives the unit a pipe it owns in place of descriptor 0 and records what flows through it; a unit that read anything from it is opaque in the prototype (Go's tools and test binaries read nothing), recorded as an input in the general design. |
| The parent reads the child's output files | Recorded as writes; replay materializes them on the overlay before the shim exits, so the parent's `stat`/`open` after `wait` sees them. Content from the CAS by hard link where the destination filesystem is the store's, by copy otherwise. A tmpfs `$WORK` means a copy, and 259 test binaries of 30-50 MB is ~10 GB of copying per `go test`: too much. The design's answer is lazy materialization: a replayed file is a symlink into a read-only bind of the CAS until something opens it for writing, and a replayed test unit that is itself replayed never needs its binary materialized at all (phase 3 in 2.6). |
| Timing | Durations are recorded, never replayed. A replayed unit's stdout carries the timestamps of the run that produced it, as a replayed step's does today; `normalize()` strips go test durations before results are compared, and the same normalization applies to unit results. |
| pids | A unit's output that mentions its own pid differs run to run and fails to prove (the streak never reaches 2); such units are never replayed. Go's `-test.testlogfile` paths and `$WORK` names are normalized by Go itself (`go-build<random>` appears only in argv, which the shim normalizes before keying, as the step's env is). |
| Temporary directories | `/tmp` is a per-step tmpfs dropped whole (rootfs mode); a unit's writes there are its scratch and are not outputs. A unit that reads a `/tmp` path it did not write (a sibling's scratch) has an input it cannot name stably: opaque. Random names in `$HOME` (`t.TempDir()` under `TMPDIR=$HOME/...`) are the unit's own writes and excluded by the write-before-read rule. |
| Environment | Part of the unit key, after the step key's volatile list; a unit whose env differs from the parent's (the parent set a variable) is a different unit by key. |
| Network | A unit that calls `socket` with `AF_INET`/`AF_INET6` is opaque (the step has no network anyway: `--unshare-net`; a connect fails identically on every run, but the unit might behave differently with a listener present in a later run, so opaque is the sound choice). `AF_UNIX` to a path the unit itself created is scratch; to anything else, opaque. |
| Signals | A unit that receives a signal that is not from its own subtree and dies of it is a killed run, never memoized (as the step rule). A unit that sends a signal to a pid outside its subtree (`kill` on the notify list) is opaque. |
| Shared memory and mmap writes | `shmget`, `memfd_create` shared with another unit, `mmap(MAP_SHARED, PROT_WRITE)` of a file outside its scratch: opaque (all on the notify list). Private mappings and read-only shared mappings are reads. |
| Daemons | A unit whose subtree has a live process when the exec'd process exits (the shim is a subreaper and sees the orphan) is opaque, and so is the step if the daemon is still alive at step end (today's rule: bwrap's exit kills it, the step records nothing special; with units, the step records that unit opaque). |
| The go build cache's concurrency | Go writes cache entries with rename-into-place and a lock file per entry; two Go commands never share a `$WORK`. A unit replay that materializes a link output into `$WORK` races nothing. The warm cache is bound read-write and Go's own writes to it are Go's business, as today; a replayed *compile* would need to produce the cache entry Go expects (Go reads it back from `$WORK/b001/_pkg_.a` and copies it into the cache), which is a recorded write and works. |
| Cross-unit dependencies through shared state | A unit reading a path a sibling wrote records the sibling's output content as its input (2.2). A unit reading a path under `$HOME` that an earlier unit of the same step created (cli/cli's tests: `~/.config/gh`, `~/.cache/gh`) is the same case; the measurement counted these as inputs. |
| The step's own writes before the first unit | A unit's read set names what it read, whoever wrote it: a path the step (or the parent unit) wrote before the unit started is an input with that content, found in the step's write log. The unit key deliberately does not include the step's manifest-so-far, which would be the whole-step problem again. |
| Nondeterministic units | Fail to prove (two executions, different result) and are never replayed; the proof streak is per unit key. Flaky tests are the common case and are handled exactly as flaky steps are: the streak holds at 0 while results differ, a non-zero exit never proves. |
| Replayed units and the step result | A step that replayed any unit is itself not recorded as a step execution (its result was not produced in full), as `exp/split` did. Its unit executions are recorded. The step's own proof streak therefore advances only on runs with no unit replays; see 2.4 for the alternative. |
| Opaque units and attribution | A unit the observation phase found opaque is not hooked next time (no shim for it), so its cost is one notify event per offending syscall on the first run and nothing afterwards. |

### 2.4 Trust, ledger, key format, CAS

**Trust.** Unit results obey the step rules unchanged (`quod_trust`): a
result replays when its manifest matched and the same manifest produced the
same result `proofs` times in a row (default 2); failures and kill markers
never prove; audits re-execute a fraction of would-be hits and a divergence
quarantines the key. The streak is per (unit key, unit manifest) as it is
per (step key, manifest). What changes is the arithmetic: a step with 500
units needs each of them proven, which on a linear history means each
package's link and test binary have run unchanged twice before they
replay. The measurement reports hits under both rules (hit: seen once;
proven: seen twice). Per-unit audits are where the design pays for its
granularity: the 5% audit rate is per replay, so a step that replays 450
units audits 22 of them every time (~22 links, ~2 s), which is a real cost
and the right one: it is what makes a per-unit divergence visible
(dashboard, 2.5).

**Ledger.** Three tables beside the step's: `units (unit_key, step_key,
parent_unit_key, binary, argv_json, cwd)`, `unit_manifests (id, unit_key,
run_id, json)` and `unit_results (id, unit_key, manifest_id, run_id,
exit_code, stdout, stderr, writes_json, duration_ms)`, plus `unit_proofs`
and `unit_trust` with the step tables' shapes, and a `run_units (run_id,
unit_key, outcome)` row per unit per run so the dashboard can say "142 of
150 replayed" and name the 8. Unit manifests are small (a link: ~400
entries; a test binary: 1-50) next to step manifests (~30k), so the F4/F5
scaling findings of 0b do not repeat at the unit level, but the *count*
does: 500 units per step times 3 steps times 40 commits is 60k unit runs in
the history, which needs the index the step lookup already lacks (lookup by
unit key, then by the first differing entry).

**Key format.** The unit key is a new canonical JSON, versioned separately
from the step key (`{"v":1,"step_key":..., "parent":..., "binary":{"h":...}
| {"path":...}, "argv":[...], "cwd":..., "env":[[k,v],...], "fds":[0,1,2
identities]}`), hashed with the same `b3:` prefix. The step key schema does
not change; a step run with units enabled gets `"units":true` in its key
(as `warm` did, key schema +1), because the env differs (`GOFLAGS` gains
`-toolexec`/`-exec`, `PATH` gains the shim directory) and because a step
that may replay units is not the same artifact as one that may not. This
keeps every existing proof valid and isolates the change from
`exp/never-worse` (whose per-step costs stay per step) and `exp/trust-v2`
(whose looser proof rules apply to unit proofs through the same `Policy`).

**CAS.** Unit outputs are blobs like step outputs (`Cas::take_files` from
the unit's scratch). Test binaries deduplicate across commits when the link
is bit-identical, which Go's are: `$WORK` is random per run but `-trimpath`
rewrites it out, and the smoke test linked the same binaries twice to the
same hashes. Stdout and stderr are blobs. The store grows by the unit
outputs that step-level Quod never kept, test binaries above all: 259 of
them at 30-50 MB each is ~10 GB if every one changed on every commit; at the
measured miss rate (section 1.2) it is a few hundred MB per 40 commits,
comparable to the step-level CAS growth of 0b (+17 MB per commit). GC: unit
results are referenced by run rows; the existing snapshot gc pattern applies.

### 2.5 What the dashboard shows

The step line today: `[executed] go test ./...: ok in 41.0s; would replay:
no, closest manifest differed at pkg/cmd/auth/auth.go`. With units, an
executed step gains a second line and a drill-down:

```
[executed] `go test ./...`: ok in 9.8s (was 41.0s); 487 of 518 processes replayed, saved 31.2s
    31 ran: 16 links and 15 test binaries below pkg/cmd/auth (changed: pkg/cmd/auth/auth.go)
    2 audited, 0 diverged; 3 opaque (internal/prompter, pkg/cmd/extension/browse, acceptance: spawn git)
```

The job summary (`quod: job 12: 3 steps, 2 executed, 1 replayed, ...`)
gains `units: 1,012 replayed, 66 ran, 3 opaque, saved 68s`. In the cloud
dashboard, the step row's bar splits into replayed (grey), ran (blue) and
opaque (hatched) segments proportional to recorded duration, and the
drill-down lists units by duration with the input that changed
(`first_mismatch`, as the receipt names it today), so a developer sees
*which* package re-ran and why. A quarantined unit (a divergence in an
audit) shows as a red segment with the unit's argv and the two results'
diff, exactly as a quarantined step does. Units never appear for a step that
replayed whole.

### 2.6 Phased plan

1. **Observation only, Go.** The Go profile adds `-toolexec`/`-exec` hooks
   pointing at the `quod unit` shim; the shim runs everything and records
   unit keys, manifests (from the warm-cache FUSE mount and `importcfg`),
   results and durations into the ledger; the run event reports `units:
   {total, would_replay, would_save_ms}`; the dashboard shows "would
   replay". No behaviour changes. Gate: 0 false would-hits under A7 adapted
   to units (mutate every file a unit manifest names, assert the unit's key
   or manifest changes), key stability across two runs of one tree above
   95% of units, overhead under 5% of the step.
2. **Replay, links and test binaries, behind `--units`.** The shim replays
   proven units, materializing outputs by copy; step results are not
   recorded on runs with unit replays. Gate: the 40-commit history under
   `--units` beats plain-warm Go on total `go test` time; A7 at the unit
   level 0 false hits; the acceptance mutation tests (A7b: unread files
   mutated, replay agrees with a fresh run) at the unit level.
3. **Lazy outputs and the warm-cache mount.** Replayed outputs become
   symlinks into a read-only CAS bind until written; the warm GOCACHE is
   served by a second observer mount so unit manifests are complete without
   `importcfg` parsing (the Go-aware part of the shim shrinks to the hook
   flags).
4. **Generic exec interposition.** seccomp user-notify observation of
   `execve` and the opacity syscalls; the `PATH` shim directory populated
   from observation; Node and Python profiles get their hooks. Gate: a
   Node repo (`tsc`, `eslint` per package) and a Python repo (pytest-xdist
   workers) show unit replays with no profile code beyond the hook.
5. **Trust and dashboard.** Per-unit audits, quarantine, the drill-down,
   `quod status` per unit; unit-level ledger index.

## 3. Recommendation

**Do not build process-level memoization as the thing that makes Quod beat
language caches on their own turf. The upper bound on cli/cli says it
cannot: with every memoizable process replayed for free, `go test ./...`
over 40 real commits lands at about the same total as a warm Go cache
(~900-1,160 s against 943 s), because the commits where Go's cache wins big
are exactly the commits where nothing is memoizable, and on `go build` and
`go vet` there is nothing to memoize at all.** The prototype confirms the
shape: with links and test units replaying at 46% and 56%, `go test ./...`
over the history took 1,735 s against 1,048 s plain-warm, 17 s against 13 s
on a quiet commit, and the time left is the cost of keying inputs that live
outside the observed tree, materializing outputs, and the units that spawn
subprocesses.

What the measurement does say is worth building, in order:

1. **Stop losing on misses first.** Quod's miss is 41 s where plain-warm is
   10-14 s on a quiet commit, and 75% of that is re-linking 259 test
   binaries because `-count=1` turns Go's test cache off. `exp/never-worse`
   (Go's cache keyed by content inside Quod) attacks exactly this and is the
   larger, simpler win: it makes a Quod miss cost what a warm plain run
   costs, on every commit, including the heavy ones where a process memo
   has nothing to offer. The process memo is only ever worth as much as the
   gap that remains after it.
2. **Then memoize test units as the one Go-specific hook that beats Go**, on
   top of never-worse: the 38 leak packages Go re-runs every commit are 5 s
   of test time plus ~17 s of link time serial per commit (~5-6 s of wall on
   4 vCPUs, half of a quiet commit's plain-warm step), and a content-keyed
   unit memo takes them. That is phase 1-2 of section 2.6 restricted to
   `-exec`, which `exp/split` already built once; its loss then was the same
   link cost this measurement names, and never-worse removes it. Expect a
   quiet commit at ~6-7 s against Go's 10-14 s, nothing on heavy commits.
3. **Build the generic mechanism (seccomp observation, PATH shims, per-unit
   overlays) only on evidence from a non-Go corpus.** The claim that
   `tsc`, `eslint` and pytest workers behave like Go's test binaries is
   plausible and unmeasured; the same wrapper method (a `PATH` shim logging
   argv and an `strace -f -e openat` read set) costs a day per corpus and
   gives the same upper-bound table before any Quod code is written. The
   two correctness hazards that are expensive to handle generically, write
   attribution in a parallel step and reads outside the observed trees, are
   both cheap in the tool-specific hooks and both unavoidable in the generic
   one.

What would change the recommendation: a corpus whose steps are dominated by
long-running leaf processes with small, stable read sets and no language
cache of their own (integration test suites that shell out per test, docs
builds, linters without a cache). There the whole-step miss is total and
the per-process hit rate would look like cli/cli's quiet commits on every
commit. That is a different market from "beat Go's cache", and it is the
honest one for this mechanism.
