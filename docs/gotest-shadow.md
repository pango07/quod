# go test replay with shadow recording (`quod gotest`)

2026-11-02, branch exp/gotest-shadow (built on exp/vitest-shadow). This is the
vitest shadow design (docs/vitest-shadow.md) applied to `go test`, with the
package's test binary as the unit. The developer's job runs only the affected
packages, with plain go test and nothing recorded. A separate, non-blocking job
records them. Code: crates/quod-cli/src/gotest_shadow.rs.

## Units and keys

A package replays only when every input its record holds is unchanged:

- **Global:** `go env` (minus GOCACHE, which is content-addressed, and
  GOGCCFLAGS, which go spells with a fresh temp directory each time), every
  environment variable (minus the per-run CI ones, as `quod vitest` does),
  go test's flags, the directory, the CPU count, go.work, and quod's own code.
- **Compiled closure,** from one `go list -e -deps -test -json` call: every
  package the test binary links (`P.test` and its Deps). Each local package is
  hashed by the content of every compiled, cgo, assembly, syso and embedded
  file, plus its module's go.mod. A module-cache package is keyed by
  path@version (read-only and checked against go.sum). A standard package is
  covered by the toolchain. The package's default.pgo is included. A non-standard
  cgo package always runs, because the host's C toolchain and headers are not
  keyed. So does a package with load errors.
- **Runtime:** what the test binary touched while it ran, as strace saw it in
  the shadow. That is every file it opened (`f:`, by content), every directory
  it listed (`d:`), every path it stat'd or accessed (`s:`: type, permissions,
  a file's size), and every link it read (`l:`). Each is checked by its state
  now, on the current tree.

**Never recorded (the package always runs):**
- a failure;
- a process other than its own binary (any execve);
- network: any connect, sendto or sendmsg except to a loopback port or a unix
  socket the same process tree bound (DNS through 127.0.0.53 counts as
  network);
- a socket that is not AF_INET, AF_INET6 or AF_UNIX;
- a write, create, rename, unlink or chmod under the workspace, or outside the
  temp directories;
- any traced syscall the observer does not classify;
- non-UTF-8 output.

The whole environment is keyed, so a getenv needs no tracing. strace sees every
syscall from exec on, including package init before `m.Run` (go's own testlog
misses that) and cgo or `syscall.Open` reads.

Not inputs: /dev, the process's own /proc entries, the runtime's start-up reads
(transparent_hugepage, /sys/fs/cgroup), the test binary's directory, and
anything under a temp-directory entry the test itself created (a successful
mkdir, or `O_CREAT|O_EXCL`, directly in /tmp, /var/tmp, /dev/shm or $TMPDIR). A
read of a pre-existing temp path is an input like any other.

## The two jobs

**Critical path:**
`quod gotest --no-record --dir L --report R -- <go test args>`.
- go list, go env, hash, select.
- Run `go test <the same flags> <the affected packages>`, with nothing added.
  When every package must run, the developer's own patterns are passed
  instead.
- The replayed packages' recorded stdout is merged into go test's stream in go
  test's package order. In text mode a replay is printed before the first line
  of the next package that ran after it (go test prints packages whole, in
  order). In `-json` mode the merge goes by each event's Package. The exit code
  is go test's, and replays are never failures.
- Go's own test cache still works for the packages it runs: these are plain
  flags.
- The ledger is read, never written.
- Arguments it does not understand (coverage, profiles, `-exec`, `-args`,
  `-bench`, `-c`, `-o`, file arguments, `-shuffle=on`) run plain go test with
  no replay.

**Shadow:**
`quod gotest --dir L --audits 4 --expect-report R -- <go test args>`.
- It makes the same selection, plus 4 random replayable packages as audits.
- Runs them with `-count=1 -exec <wrapper>`. The wrapper runs each test binary
  under `strace -f --seccomp-bpf -y -e trace=%file,fchdir,socket,connect,bind,sendto,sendmsg,getsockname`,
  logged by package directory.
- A package records only if its trace is complete and its status and per-test
  results match the critical path's report for it. Otherwise it is quarantined
  as "recording changed behaviour".
- The replayed output is the critical path's, not the shadow's.
- An audit that differs from its record quarantines the package.

The hand-off is the vitest one: actions/cache keyed by branch and commit,
restoring the newest completed ledger.

## The critical path's own cost: go list reused

`go list -deps -test` and `go env` cost about 60 ms on afero and 350 ms on
expr. That is pure overhead on a commit where most packages run anyway.

The shadow stores its listing in the ledger, with what the listing depended
on:
- the go command's path and file identity;
- every environment variable and the go env file;
- the directory, the patterns and go list's flags;
- every path under the root (names and kinds, not contents);
- the contents of every go.mod, go.sum, go.work, go.work.sum and
  modules.txt;
- per .go file, its state and its header. The header is everything up to the
  end of the imports (build constraints, package clause, cgo preamble) plus
  every `//go:embed` line, read by a small Go lexer (`gotest_shadow/go_src.rs`).

The critical path walks the tree and hashes every .go file, which the closure
keys need anyway. It reuses the listing when everything above is unchanged and
every changed .go file still has its header. Otherwise it runs go list. A file
the lexer cannot read never reuses.

## Function-level narrowing

The unit stays the package's test binary.
- The shadow also runs with `-cover -covermode=set -coverpkg=<every local
  non-main package>`. The -exec wrapper copies each binary's
  `-test.gocoverdir`, and `go tool covdata textfmt` turns it into blocks.
- Each local, non-test .go file of a package's closure is split by the lexer
  into a **skeleton** and **bodies**. The skeleton is every token outside
  top-level function bodies, with comments dropped (directives kept) and
  newlines kept. The bodies are each function's exact bytes plus its start
  line.
- A function is **cold** for a package when it has coverage blocks and its
  test binary ran none of them.
- The record holds the closure's structure key: everything but those files'
  contents, so names, other file kinds, modules and deps are still in it.
  It also holds each file's state and the cold set. The ledger keeps the
  shapes by file state.

A package whose compiled code changed replays only if all of these hold:
- the structure key is unchanged;
- coverage was recorded;
- every changed file's new version lexes, with the same skeleton;
- every function that is not cold has the same body bytes and the same
  start line (an edit that shifts a hot function's lines runs the package);
- the runtime inputs and the global key are unchanged.

Everything else runs the package: package-level declarations, types,
signatures, method sets, imports, build tags, directives, embeds, cgo,
test files, and non-Go files. So does a function with no coverage blocks
(an empty body is never cold), and so does a file the lexer cannot split.

-cover changes what go test prints, so the shadow uses it only with
`--expect-report`; the replayed output is always the critical path's.
`QUOD_GOTEST_NO_COVER=1` turns narrowing off.

## Test names that vary

Some suites put addresses or times in subtest names (testify's
`ObjectsAreEqual(time.Date(...))`, `(*T)(0x1e95...)`). The recording check and
audits compare per-test results by name. When the raw names differ, they
compare the names with hex addresses, RFC 3339 times and `time.Date(...)`
literals normalized, but only when normalizing keeps the mapping one to one.
Otherwise the package stays quarantined. The bench's comparison normalizes the
same way.

## Measured

The bench uses pango07/ci-bench `bench-gotest.yml`
(`scripts/bench-gotest/shadow.sh`, copied there as `scripts/gotest/`), binary
bin-cb8e39b, go1.27.3, GitHub-hosted runners.
- 20 consecutive first-parent commits, oldest first. Commit 0 is the first run
  and is outside the headline.
- Per commit: plain, the critical path, the shadow.
- Plain has its own GOCACHE, warm across commits (a perfect actions/cache), and
  a shared module cache. Every checkout gets fresh mtimes, as in CI.
- Every critical `-json` stream is compared with plain's per package and per
  test, and the exit codes are compared.
- 4 mutation checks: a panic at the top of a covered function.
- Lever 1 runs use `QUOD_GOTEST_NO_COVER=1`.

| tier | project | mode | plain total | critical total | speedup total / median | go's own cache hit on plain | run / replayed (narrowed) | selection median | shadow compute | wrong | mutations | run |
|---|---|---|---:|---:|---:|---:|---|---:|---:|---:|---|---|
| 1 | spf13/afero (5 pkgs, ~4.5 s) | go list reuse | 98.8 s | 74.3 s | 1.33x / 1.00x | 40% | 43 / 57 | 92 ms | 95 s | 0 | 4/4 | 41586341834 |
| 1 | spf13/afero | + narrowing | 98.6 s | 72.3 s | 1.36x / 1.04x | 40% | 33 / 67 (10) | 93 ms | 105 s | 0 | 4/4 | 41586347979 |
| 2 | expr-lang/expr (62 pkgs, 4-10 s) | go list reuse | 155.3 s | 137.0 s | 1.13x / 0.98x | 49% | 570 / 691 | 48 ms | 272 s | 0 | 4/4 | 41586344930 |
| 2 | expr-lang/expr | + narrowing | 159.4 s | 140.9 s | 1.13x / 0.98x | 49% | 559 / 702 (14) | 50 ms | 320 s | 0 | 4/4 | 41586350705 |

- **Commits that change no Go code** (docs, CI): afero's critical path is
  10–20 ms against plain's 4.4 s. go's own cache missed there because afero's
  tests open files, and a fresh checkout's mtimes defeat go's cache. On expr it
  is 1.1–2.6 s against 4–5 s; the testify package runs every time (below).
- **Code commits** are at parity, within about 150 ms on expr. Both projects'
  commits nearly always touch a core package that every test binary links:
  afero's root, and expr's checker, vm, parser or builtin. That package's own
  tests execute the changed function, so it must run.
- **Selection:** with go list reused, afero takes 7 ms and expr 47 ms. go list
  runs only when the module's structure really changes (a new file, an import
  outside the closure, go.mod); then it takes 95 ms, or 500 ms with a go.mod
  change.
  - Two fixes got expr there. The 60 MB `-json` ledger was the critical path's
    main cost (220 ms of selection, plus a 50 MB report): outputs are now raw
    content-addressed blobs. And -json events are scanned, not parsed.
- **Narrowing** replays 10 (afero) and 14 (expr) packages over 20 commits.
  These are cheap packages whose tests never ran the changed functions.
- **The safety net:** expr's internal/testify/assert has subtest names with
  pointers and `time.Date(...)` that collide even after normalization. It is
  quarantined on the first shadow ("recording changed behaviour") and runs
  every time; the bench counts it flaky, not wrong.
- Earlier binaries (95131c7, before levers 1 to 3) measured afero 1.35x and
  expr 1.03x (runs 41579743789, 41579359948).
