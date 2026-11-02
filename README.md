<p align="center">
  <img src="docs/brand/quod-logo.png" alt="Quod" width="520">
</p>

<h1 align="center">Your CI already did this.</h1>

Quod replays any CI step whose inputs haven't changed, and shows you the
proof. It watches what each step actually reads, at the filesystem level, in
any language, and when every one of those inputs is byte-identical to a run it
has already proven, it returns that run's result in a fraction of a second,
with a receipt naming the run and the inputs that matched.

No config. No build rewrite. No guessing: a step that can't be proven runs.

```text
QUOD  REPLAYED
  proved by run 01JCK3Q9 on 2026-11-14 09:12 UTC
  step b3:4c1e9a07  manifest b3:9a07f2d1  21,073 inputs matched
  recorded 17.0s  replay 0.25s  saved 16.7s
```

## Status

Early. The engine is proven on one real repo (GitHub's CLI, `cli/cli`): a
17-second test step replays in 0.25s with byte-identical output, and mutating
every one of ~30,000 recorded inputs produces zero false hits. The runner,
portable proofs, the trust layer and Node/Python support are in progress. See
`docs/spec.md` for the product, `DECISIONS.md` for every decision and
measurement, and `docs/REVIEWS.md` for the review record.

## Develop

Linux only. On macOS, run everything through the dev container:

```sh
scripts/dev.sh scripts/check-host.sh   # every Environment row
scripts/dev.sh cargo test --workspace  # unit, property, golden, integration
scripts/acceptance.sh                  # A1-A7 on the pinned spike repo
```

Steps run under a toolchain profile (Go, Node or Python; `quod_key::profile`),
detected from the repo or given with `--toolchain`. The acceptance suite
runs per profile: `scripts/acceptance.sh go|node|python [checkout]` on the
corpus repos pinned in `versions.toml` (cli/cli, moment/luxon,
encode/starlette); results in `docs/bench/polyglot-corpus.md`. The
pinned rootfs below holds only Go, so Node and Python steps pin the host.

The heavy gates (lint, merge gate, hooks and CLI acceptance) also run on
GitHub's runners: `gh workflow run gate.yml --ref <branch>`; see
`docs/ci.md` for what each job proves, its duration and the minute budget.

## Portable proofs

Steps can run in a pinned root filesystem (`--rootfs DIGEST`, or
`QUOD_ROOTFS`): an OCI image imported once into the store and served
from it, with nothing of the host's userland visible, a fixed process
state, and writes outside the workspace recorded (except /tmp, scratch; the
workspace overlay's upper still lives on the host's state filesystem). The
key pins the rootfs digest and the host class, read as the step sees it
(kernel release, build and config, CPU, exact RAM, rlimits, sysctls, cgroup
limits, binfmt_misc, LSM label, seccomp mode, filesystem types, sandbox
version; DECISIONS lists what is not keyed), so
a proof made on one host replays on another host of the same class whatever
its userland, and nowhere else. Measured so far, on two containers sharing
one VM (so kernel and hardware are equal by construction): an Ubuntu
container replays a Debian container's proof when both use the same
bubblewrap; with another bubblewrap it is another host class (the two key
documents differ in that one entry only), executes, and reads exactly the
same inputs to the same result.

```sh
QUOD_DEV_NS=rootfs scripts/build-rootfs.sh             # build, export, import; prints the digest
QUOD_DEV_NS=rootfs scripts/build-rootfs.sh --extra jq  # a different rootfs (the miss case)
scripts/dev.sh 'quod run --repo /work/cli --rootfs b3:... --step "go test ./..."'
QUOD_DEV_NS=rootfs scripts/portability.sh              # Debian host and Ubuntu host
```

Without a rootfs the key pins the host's fingerprint (`--host-root`), which
does not travel; the result records which mode produced it
(`quod inspect <step_key>`). A runner drives single steps through
`quod step --request R --response O` (see `crates/quod-cli/src/step.rs`).

The binary is `quod`, the crates `quod-*`, the workspace state `.quod/` and
the environment `QUOD_*`. Quod is pronounced "kwod", from Q.E.D.

Using it from a checkout (observe mode is the default: every step executes
and is recorded; the banner says what would have replayed):

```sh
quod run -- go test ./...           # the step as a command line
quod run --step "go test ./..."     # or as a script
quod run --mode replay -- go test ./...   # replay results that proved themselves
quod status                         # hits, misses, time saved, trust per step key
quod inspect <step key | manifest id | run id>
```

Without `--repo` the repo is the git checkout around the current directory,
and the step runs in that directory, as it would without Quod.

A result replays only after the same inputs produced the same result twice
(`--proofs`); a fraction of replays is re-executed and compared (`--audit`,
default 5%), and any divergence quarantines the step key until it proves
itself again. `--timeout` (default 30m) kills a step that runs too long and
records nothing. `--allow-net` runs a step with network, unmemoized, with
its dependency trees (module cache, venv) in the state dir rather than the
checkout.

See `DECISIONS.md` for the record of choices made along the way.
