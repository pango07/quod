# bench-lite baseline (main 3ddaaca)

bench-lite is the fast benchmark every branch runs (`scripts/bench-lite.sh`,
the `bench-lite` job of gate.yml): a generated Go + Node + Python monorepo
and an 18-job scripted history, each job run plainly and through Quod on the
same machine, interleaved, with every Quod result compared against the plain
run of the same tree. This is its baseline: Quod's code at main `3ddaaca`
(the branch exp/bench-lite adds only the benchmark), measured on GitHub's
`ubuntu-latest` (x86_64, 2 CPUs, kernel 7.0.0-1017-azure, FUSE over
io_uring) in [run 41484064103](https://github.com/pango07/quod/actions/runs/41484064103),
2026-11-02. Machine-readable: `lite-baseline.json` (the summary
`--against main` compares with) and `lite-baseline-rows.jsonl.gz` (every
step run: command, wall, exit, outcome, normalized output, Quod's phases).

## The fixture

`scripts/bench-lite/fixture.py` writes it the same way on every machine (fixed
names, dates and contents; the commit hashes are the same everywhere), with
no network:

- `go/`: module `example.com/lite`, a shared lib imported by alpha, beta and
  gamma, two standalone packages (a sieve, a SHA-256 chain) and `gitinfo`,
  whose test runs git in the checkout and reads `.git/HEAD` (as cli/cli's do).
- `node/`: a workspace of four packages (core, fmt and stats on core, cli on
  both) plus a vendored zero-dependency helper; tests with `node --test`.
- `py/`: package `lite` (four modules), unittest tests, a `requirements.lock`
  with hashes and the wheels it pins, installed with `pip --no-index`.
- `tools/fetch.py`: the network step, a download from a loopback HTTP server
  the driver runs, checked against a pinned SHA-256.
- `docs/`: read by nothing.

Every source file has a `Rev`; each test prints a checksum of what it imports,
so a stale replay prints something a plain run does not.

Steps per job: `py-deps` (venv and pip install from the wheel dir) and
`net-fetch` run `--allow-net` (never memoized); `go-test` (`go test -v ./...`,
`--warm`), `node-test` (`node --test`) and `py-test` (`python -m unittest`) are
keyed, `quod run --mode replay` with the shipped trust defaults (proofs 1,
shadow 2, audit 0.05, deferred audits run by the driver right after each
job). `node --test` also runs through `quod step` in a small rootfs (sh, node
and their libraries). Plain runs get a fresh `git clone --shared` checkout per
job and a GOCACHE that persists (plain-warm, as in 0b/0c).

## Results

| # | scenario | plain s | quod s | quod/plain | replayed | ran | audited | quod step | wrong |
|---|---|---:|---:|---:|---:|---:|---:|---|---:|
| 0 | init | 13.96 | 20.62 | 1.48x | 0 | 3 | 0 | ran 1.38 | 0 |
| 1 | go-pkg | 3.38 | 7.23 | 2.14x | 0 | 1 | 2 | diverged 1.36 | 0 |
| 2 | go-shared | 3.87 | 7.77 | 2.01x | 0 | 1 | 2 | diverged 1.39 | 0 |
| 3 | docs-only | 3.06 | 7.22 | 2.36x | 1 | 0 | 2 | diverged 1.39 | 0 |
| 4 | node-pkg | 3.02 | 7.10 | 2.35x | 1 | 1 | 2 | ran 1.39 | 0 |
| 5 | py-src | 3.01 | 6.66 | 2.21x | 1 | 1 | 1 | diverged 1.39 | 0 |
| 6 | dep-bump | 3.00 | 6.64 | 2.21x | 1 | 1 | 2 | diverged 1.42 | 0 |
| 7 | pr-head | 3.44 | 6.47 | 1.88x | 1 | 2 | 0 | ran 1.42 | 0 |
| 8 | merge | 3.03 | 6.11 | 2.02x | 1 | 1 | 1 | diverged 1.39 | 0 |
| 9 | rerun | 2.97 | 5.96 | 2.01x | 2 | 0 | 2 | diverged 1.34 | 0 |
| 10 | py-src-2 | 2.98 | 6.73 | 2.26x | 1 | 1 | 1 | diverged 1.42 | 0 |
| 11 | revert | 3.04 | 5.88 | 1.93x | 2 | 0 | 2 | diverged 1.39 | 0 |
| 12 | unread | 3.02 | 6.89 | 2.28x | 0 | 3 | 0 | diverged 1.37 | 0 |
| 13 | go-pkg-2 | 3.20 | 6.31 | 1.97x | 1 | 1 | 1 | diverged 1.39 | 0 |
| 14 | node-shared | 3.02 | 6.04 | 2.00x | 2 | 1 | 0 | ran 1.39 | 0 |
| 15 | py-test-add | 3.00 | 6.60 | 2.20x | 1 | 1 | 1 | diverged 1.34 | 0 |
| 16 | mixed | 3.22 | 7.00 | 2.17x | 0 | 2 | 1 | diverged 1.39 | 0 |
| 17 | go-shared-2 | 3.86 | 6.91 | 1.79x | 1 | 1 | 2 | diverged 1.44 | 0 |

Jobs 1-17: plain 54.1 s, Quod 113.5 s (**2.10x**), deferred audits 4.6 s
after the jobs. Keyed steps: 16 replayed, 18 ran, 22 audited. **Wrong
replays 0**; no Quod result differed from plain; no step failed.

| step | runs | replayed | executed | diverged | plain median s | Quod overhead per miss (median) | replay median s |
|---|---:|---:|---:|---:|---:|---:|---:|
| py-deps | 18 | 0 | 17 | 0 | 0.897 | +0.74 s (+82%) | - |
| net-fetch | 18 | 0 | 17 | 0 | 0.051 | +0.72 s | - |
| go-test | 18 | 7 | 10 | 0 | 0.07 | +0.89 s | 0.736 |
| node-test | 18 | 0 | 17 | 13 | 1.297 | +0.75 s (+57%) | - |
| py-test | 18 | 9 | 8 | 0 | 0.709 | +0.77 s (+108%) | 0.712 |
| node-test (quod step) | 18 | 0 | 17 | 14 | 1.297 | +0.09 s (+7%) | - |

The same run in the dev container (aarch64, 18 CPUs) gives the same outcome
in every cell (replayed, ran, audited, diverged; the fixture and the trust
state are deterministic) with Quod at 1.08x of plain: what differs between the
two hosts is the fixed cost per step.

## What it shows on main

1. **`node --test` never replays.** Its summary line `ℹ duration_ms 1297.4`
   (and TAP's `duration_ms: 12.3`) is not one of the durations Quod's result
   normalization erases (`quod_trust::normalize`), so every shadow audit of a
   `node --test` step diverges, the key is quarantined, and it executes on
   every job: 13 of its 18 `quod run` runs and 14 of 18 `quod step` runs
   were audits that diverged. Node's own test runner is the default for small Node projects
   (commander.js uses it). A fix branch should see node-test's `replayed`
   column go from 0 to about 9.
2. **A fixed ~0.7 s per `quod run` step on GitHub's runner**, replay or miss:
   a replay of `py-test` takes 0.71 s against 0.06 s on the dev VM. `quod step`
   with a rootfs costs +0.09 s per execution on the same runner, so most of the
   0.7 s is what `quod run` does on the host root and `quod step` does not.
   The likely part is the host fingerprint (docs/ci.md measured it at
   0.6-0.9 s warm on this runner); not yet profiled. With
   steps of a few seconds it is Quod's whole overhead here (2.1x); on 0c's
   cli/cli steps it is ~1 s of 15-60.
3. **A new top-level entry misses every step** (`unread`: `misc/notes.txt`;
   all three keyed steps first differ at `.`, the workspace root's listing).
   A new file inside an existing directory nobody lists (`docs-only`) misses
   nothing.
4. **PR head to merge**: `go-test` misses at `.git/HEAD` (branch `pr`, then
   `main`; DECISIONS, exp/git-head-replay), as on cli/cli. Node's and Python's
   steps, which do not run git, replay across it.
5. **Revert and re-run** replay from the earlier manifests (`revert`: 2
   replayed, the third is `node-test`).
6. **Network steps** (`py-deps`, `net-fetch`) run every job, `--allow-net`, at
   +0.7 s each. `quod run` has no network detection or memory: a step declares
   it. The detect-and-remember path (exp/py-node-ready, ledger v7) is on the
   runner hooks path, which bench-lite does not drive.

## Comparing a branch

```sh
scripts/bench-lite.sh --against main          # dev container, under the heavy lock
gh workflow run gate.yml --ref <branch> -f only=bench-lite
```

The job summary prints the table with a `Δ quod s` column and the per-step
overhead beside the baseline's. Seconds compare only on the same host class
(the table says when they do not); the outcome columns compare everywhere.

Runner noise: the next push of the same code (run 41486482488) got a slower
runner of the same class: plain 82.7 s against 54.1, Quod 161.9 against
113.5, quod/plain 1.96x against 2.10x, every outcome cell identical. So on
GitHub, read a branch's effect from the outcome columns and the quod/plain
ratio (about ±7% between runs), not from the `Δ quod s` seconds, which move
by up to 40% with the runner.
