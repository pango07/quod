# The gates on GitHub Actions

The heavy gates run on GitHub's hosted runners (`.github/workflows/gate.yml`)
so agents and the coordinator stop queueing on the dev VM's heavy lock.
Every gate is one `scripts/ci-gate.sh` command, the same command on the dev
box and on a runner; `ci-gate.sh` only wraps the scripts that own each gate
(`lint.sh`, `merge-gate.sh`, `acceptance.sh`, and the one cargo command of
the hooks suite) and adds corpus preparation, logs and the job summary.

`ci.yml` (fmt, clippy, unit tests, and the integration tests as root) is
unchanged and still runs on every pull request and every push to main.

## Running a gate on a branch

```sh
gh workflow run gate.yml --ref <branch>                         # every job
gh workflow run gate.yml --ref <branch> -f only=hooks,acceptance-cli
gh workflow run gate.yml --ref <branch> -f ref=<sha>            # another commit
gh run list --workflow gate.yml --branch <branch> -L 3
gh run watch <run-id>                                           # or: gh run watch (pick one)
gh run view <run-id> --log-failed
gh run download <run-id>                                        # the logs artifact of a failed job
```

`only` takes a comma-separated list of `lint`, `merge-gate`, `hooks`,
`acceptance-cli`, `acceptance-node`, `acceptance-python`, `cloud`, or `all`.
`ref` must be a commit that has `scripts/ci-gate.sh` (main after this
change, or a branch rebased on it). Each job's summary page shows the host
(kernel, CPUs, ring), the observer transport it used, every criterion line
(A1-A7 with the false-hit count, A7b, W1-W4, T1-T11, P1) and the test totals;
a failed job uploads `target/gate-logs/` (and, for acceptance,
`target/acceptance-evidence/`) as an artifact, kept 14 days.

The same gate locally, inside the dev container and under the shared lock:

```sh
scripts/dev.sh 'flock /work/tmp/heavy.lock scripts/ci-gate.sh merge-gate'
scripts/dev.sh 'flock /work/tmp/heavy.lock scripts/ci-gate.sh hooks /work/cli-quod'
scripts/dev.sh 'flock /work/tmp/heavy.lock scripts/ci-gate.sh acceptance node'
```

## Triggers

| Event | What runs |
|---|---|
| `push` to `exp/**`, `fix/**`, `cloud/**`, `ci/**` | fast gates: lint and merge-gate if anything outside `cloud/` changed (the push's diff; a new branch: its diff from main), cloud if `cloud/` changed |
| `pull_request` to main, new commits (`synchronize`) | the same fast gates |
| `pull_request` to main `opened`, `reopened`, `ready_for_review` | every Rust gate if anything outside `cloud/` changed; cloud if `cloud/` changed |
| `workflow_dispatch` | what `only` names (default all), whatever changed |

The `changes` job decides from the event and the diff, so a cloud-only push
runs only the cloud job (npm ci, lint, `next typegen`, tsc, test, build).
`concurrency` cancels a superseded run of the same ref or pull request (a
dispatch with `-f ref=<sha>` gets its own group, so it runs beside the push
run of its branch). A commit message with `[skip ci]` skips the push run.
Before merging a branch whose last full gate predates its last code change,
dispatch the corpus gates on it.

## The jobs

Measured on 2026-11-01 (runs 41349234113, 41351622901, 41352822987 in
pango07/quod), job wall time with warm Rust caches:

| Job | Wall time | Result on 5d296c4 (main) + this workflow |
|---|---|---|
| changes | 7 s | |
| lint | 5 min | fails `served_mtimes_follow_content` only (known: GitHub's kernel stores times at 100 ns; fix/cas-visibility) |
| merge-gate | 4 min | every ring suite passes; fails `transports_log_identically` on served times (the same 100 ns cause) |
| hooks | 46 min | ok: A7 0 false hits over 7,285 f + 1,690 d + 658 a; hooks_acceptance, hooks_git_index, hooks_secrets |
| acceptance-cli | 102 min | ok: A7 0 false hits over 23,983 f + 5,336 d + 621 a (912 s), A7b 10/10, W1-W4, T1-T11, T7 classic and ring |
| acceptance-node | 24 min | ok: A7 0 false hits over 22,053 f + 2,805 d + 87 a + 39 l, trust, T7 |
| acceptance-python | 21 min | ok: A7 0 false hits over 24,265 f + 982 d + 78 a + 4 l, trust, T7 |
| cloud | 2 min | ok |

The corpus jobs are slow because the runner has 2 CPUs: every cli/cli
execution compiles the module from an empty GOCACHE, and the suite executes
`go test ./...` about 25 times (A1, P1, A4-A6, ten A7b fresh runs, W1-W4,
trust). A7 alone is 15 minutes (10 on the dev VM's 18 cores).

| Job | What it proves |
|---|---|
| lint | `scripts/lint.sh` as the dev container runs it: fmt, clippy (default and all features) with -D warnings, and every workspace test as root with `QUOD_INTEGRATION=1` and the Go, Node and Python toolchains installed (so the integration tests and the Node and Python acceptance fixtures run, not only unit tests). |
| merge-gate | `scripts/merge-gate.sh` without its lint step (`QUOD_GATE_SKIP_LINT=1`; lint is its own job): the FUSE-over-io_uring suites (`uring`, `ring_faults`), the observer's failure paths on both transports (`faults`, `observer_faults`) and `transports_log_identically`. |
| hooks | `tests/hooks_acceptance.rs` on cli/cli at the pinned commit: A1-A7 (+A7b) through the runner-hooks path, the trust policy, secrets classification and leaks (S1-S4), the served git index (G1-G5), warm-cache poisoning. |
| acceptance-cli | `scripts/acceptance.sh go` on cli/cli: step A, A1-A7 with every recorded entry mutated (the "0 false hits" line), P1, A7b (10 samples), W1-W4, the trust suite T1-T11, T7. |
| acceptance-node | `scripts/acceptance.sh node` on moment/luxon, `QUOD_SKIP_P1=1` (Jest's suite order and coverage timestamps are not deterministic). |
| acceptance-python | `scripts/acceptance.sh python` on encode/starlette, `QUOD_SKIP_P1=1` (pytest's duration line). |
| cloud | `cloud/`: npm ci, lint, tsc, test, build. |

## The runner

`ubuntu-latest` for a private repository: Ubuntu 26.04, kernel 7.0 (azure),
x86_64, 2 CPUs, 7.7 GiB. The kernel has `CONFIG_FUSE_IO_URING`
(`fuse.enable_uring` exists, off by default); `ci-gate.sh setup` turns it on,
so the observer uses FUSE over io_uring as it does on the dev VM, and the
merge gate runs its ring suites. On a runner whose kernel lacks it,
`QUOD_RING_OPTIONAL=1` makes the merge gate print `SKIPPED the ring suites`
and run their classic cases instead of failing, and the observer falls back
to classic `/dev/fuse` (every job's host line says which).

`ci-gate.sh setup` makes the runner the dev image's equal: bubblewrap, fuse3,
sqlite3, `user_allow_other`, unprivileged user namespaces past AppArmor, and
the pinned Go, Node, Python and uv from `versions.toml` at `/usr/local`, the
paths Dockerfile.dev uses (the sandbox sees only `/usr`; the tests look
there). Rust comes from `rust-toolchain.toml`. The gates run as root, as in
the dev container: `ci-gate.sh` re-runs itself under sudo and passes the
environment by name, because Ubuntu 26.04's sudo (sudo-rs) ignores `-E`,
then hands `target/` and `~/.cargo` back to the runner user for the caches.

## What is cached, and why that keeps the tests meaningful

| Cache | Contents | Why it is safe |
|---|---|---|
| `Swatinem/rust-cache` (keys `lint`, `merge-gate`, `release`) | `~/.cargo` registry and git, and the target directory's third-party dependency builds | It drops the workspace's own crates, so every job compiles Quod from the checkout under test. Dependencies are pinned by Cargo.lock and the key includes the toolchain and the lockfile. |
| `corpus-<tc>-<commit>` | a full clone of the corpus repo (cli/cli, luxon, starlette) | Pinned by commit in versions.toml, which is in the key. Saved right after cloning, before any gate touches it; acceptance.sh checks the commit out again on a branch, and every suite works on a copy. |
| `corpus-hooks-<commit>-go<version>` | the cli/cli clone plus `.quod/gomod`, the module cache the hooks test seeds its jobs from | The seed is an operator's input to the hooks path, not Quod output: `go mod download` against go.sum, which verifies every module's hash. Quod's own step A is not involved. |

Not cached, on purpose:

- `QUOD_DEPS_DIR` (step A's dependency trees: Go module cache, node_modules,
  the venv). Step A is part of acceptance ("step A ok: dependency trees in
  ..., none in the checkout"); a warm dependency tree would turn `go mod
  download` and `uv sync` into no-ops and stop exercising Quod's network step.
  It costs about a minute per corpus.
- Any `GOCACHE` or Quod state directory (CAS, ledger, snapshots). The warm
  criteria (W1-W4) start from an empty cache by design, and every suite's
  state is a fresh temp dir; a carried-over ledger could replay what the test
  must execute.
- The toolchains: downloaded at their pinned versions each run (seconds).

## GitHub's runners against the dev VM

| | dev VM (Docker Desktop) | GitHub `ubuntu-latest` (private repo) |
|---|---|---|
| kernel | 7.0 linuxkit, aarch64 | 7.0.0-azure, x86_64 |
| CPUs, RAM | 18, plenty | 2, 7.7 GiB |
| FUSE ring | `enable_uring` after each VM restart | present, off by default; `setup` turns it on, observer events say `fuse-uring`, no fallbacks |
| file times | nanoseconds | 100 ns granularity: `served_mtimes_follow_content` and `transports_log_identically` fail (fix/cas-visibility) |
| sudo | root already | sudo-rs ignores `-E`; `ci-gate.sh` passes the environment by name (without `GITHUB_ACTIONS`, quod-hooks' `no_worker_no_owner` fails) |
| `/etc/resolv.conf` | a regular file | a symlink into `/run` (systemd-resolved), which the sandbox lacks; bwrap cannot bind the hooks surface's resolv.conf over it, so `setup` replaces the link with a copy. A Quod host-root run on any systemd-resolved host would hit the same: worth a fix in quod-hooks |
| home | `/root` | `/home/runner` is 0750: corpora and state live under `/work`, owned by root, as in the container (the step user maps to root outside, so a runner-owned checkout is not writable to step A) |
| host fingerprint | ~0.5 s | 11.8 s cold, 0.6 s warm: `ring_faults`' `a_ring_mount_held_through_its_teardown_fails_the_run` gives the run 9 s to mount its ring, which a cold first run missed twice. `ci-gate.sh` runs the fingerprint once before the merge gate (it prints the time), after which it passed |

Flakes seen on the dev VM (`concurrent_opens_of_a_new_ledger_migrate_once`,
`acceptance_python_fixture`'s P1 timing line) did not show in these runs;
none seen that are GitHub-only besides the cold-fingerprint timing above.

## Minute budget

A private repository on the free plan gets 2,000 Linux minutes a month;
GitHub bills each job rounded up to the minute, cancelled runs included.

| | Minutes |
|---|---|
| full gate (every Rust job; `changes` 1, lint 5, merge-gate 5, hooks 46, acceptance-cli 102, node 24, python 21) | about 205 |
| fast gate (`changes`, lint, merge-gate) | about 11 |
| cloud job | about 2 |
| `ci.yml` (check + integration), every pull request and push to main | about 10 |

A full gate is a tenth of the month, so the workflow runs it only when a
pull request is opened, reopened or marked ready, and on dispatch; pushes to
`exp/**`, `fix/**`, `cloud/**`, `ci/**` and new commits on a pull request run
the fast gate. A month of 6 full gates (1,230), 50 fast pushes (550) and 15
`ci.yml` runs (150) is about 1,930.

Spending it well:

- Dispatch only the corpus gates a change can affect: quod-hooks or
  quod-trust → `only=hooks`; quod-fs, quod-manifest, quod-replay or
  quod-sandbox → `only=acceptance-cli` first (it is half of a full gate),
  then node and python before merging; docs-only changes need nothing.
- Open pull requests as drafts while iterating; marking one ready runs the
  full gate once.
- acceptance-cli's ten A7b samples are about 45 of its 102 minutes;
  `QUOD_A7B_SAMPLES=3` would save about 30, at the cost of parity with the
  dev box's gate. Not done.
- If the budget is still short: a public repository's runners are free (and
  4 CPUs), or a self-hosted runner on a Linux box with the ring.
