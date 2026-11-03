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

## Measured

(see below)
