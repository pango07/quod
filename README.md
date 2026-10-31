<p align="center">
  <img src="docs/brand/quod-logo.png" alt="Quod" width="520">
</p>

<h1 align="center">Your CI already did this.</h1>

<p align="center"><b>Push. Prove. Done.</b></p>

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

## Portable proofs

Steps run in a pinned root filesystem (`--rootfs DIGEST`, or
`STILLPASS_ROOTFS`): an OCI image imported once into the store, with nothing
of the host's userland visible and every write outside the workspace
recorded. The key pins the rootfs digest and the host platform (arch, kernel
major.minor, CPU features and count, page size), so a proof made on one
machine holds on another with the same rootfs and platform:

```sh
STILLPASS_DEV_NS=rootfs scripts/build-rootfs.sh             # build, export, import; prints the digest
STILLPASS_DEV_NS=rootfs scripts/build-rootfs.sh --extra jq  # a different rootfs (the miss case)
scripts/dev.sh 'stillpass run --repo /work/cli --rootfs b3:... --step "go test ./..."'
STILLPASS_DEV_NS=rootfs scripts/portability.sh              # Debian host and Ubuntu host, one ledger
```

Without a rootfs the key pins the host's fingerprint (`--host-root`), which
does not travel; the result records which mode produced it
(`stillpass inspect <step_key>`). A runner drives single steps through
`stillpass step --request R --response O` (see `crates/sp-cli/src/step.rs`).

The code still says `stillpass` (binary, crates, paths); the rename to `quod`
lands in one sweep once the in-flight branches merge.
