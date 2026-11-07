# Real npm units: anonymous descriptor metadata

[Gate 42357911268](https://github.com/pango07/quod/actions/runs/42357911268)
passed at `89cc5bf09197b0a6fea8407c0c85f8553957001a`. It runs the packaged
`bin-2bd297b` engine against the original convex-helpers c11 revision,
twice each for `npm run build` and `npm run test:release`. Every command
exited successfully; every unit declined recording because it observed
metadata on a descriptor without a stable pathname. No replay was enabled.

This answers a concrete question raised by the full-window fallback: these
commands do not merely need a better mapping from file descriptors to paths.
The trace contains real anonymous pipes and Unix-domain sockets.

| Unit | Sample | Descriptor metadata entries | Anonymous entries | Types |
| --- | ---: | ---: | ---: | --- |
| build | 1 | 672 | 44 | 44 pipes |
| build | 2 | 676 | 44 | 44 pipes |
| test:release | 1 | 26,207 | 2,765 | 209 pipes; 2,556 Unix sockets |
| test:release | 2 | 26,218 | 2,765 | 209 pipes; 2,556 Unix sockets |

Counts are syscall entries, not distinct descriptors; some entries have
their return on a later resumed line. The first release sample also has one
undecoded descriptor query returning `EBADF`. The other three samples have
none. That isolated failure does not explain the repeated anonymous reads.

For example, the first build reads metadata on stdout pipe `18408`; the
second reads stdout pipe `22627`. Both report FIFO type and zero size in
strace's abbreviated output. Equal displayed fields do not establish equal
full metadata or which fields native application code consumes. The
production parser correctly declines to convert these observations into
ordinary pathname keys.

## Consequence for the original target

The unit wrapper captures stdout/stderr through pipes. Looking up a guessed
`/proc/self/fd/1` after execution would inspect the observing process's
descriptor, not recover the terminated child's captured-output descriptor.
Additional pathname bookkeeping cannot make anonymous metadata into a
stable file-content input. The trace alone also does not justify reducing
the observation to file type.

E2 compares records only after their input keys can be constructed. It
cannot repair missing input state or justify omitting these metadata reads.
Do not relax the descriptor guard to restore earlier hit rates. A subsequent
design needs a complete representation and validation of observable
descriptor state, a justified boundary around its consumption, or an
explicitly different execution contract. This is the first reproduced
blocker, not proof that it is the only remaining blocker.

## Method and limits

The [probe](../../scripts/bench-vitest/unit-descriptor-origin.py) invokes the
unmodified packaged binary. A wrapper around `strace` preserves each trace
after tracing exits and before Quod parses/deletes its copy. Each sample has
a fresh ledger; samples share one installed checkout, so the second build
starts with the first build's outputs. This diagnostic is not a job-speed
measurement, complete equivalence test, or successful replay demonstration.

The first diagnostic
[42357708948](https://github.com/pango07/quod/actions/runs/42357708948)
captured one build and then failed because sandbox PID reuse collided with
its trace filename. The corrected gate uses unique temporary filenames.
Neither gate changes production cache policy.

The successful run uses Node 24.18.0, strace 6.19 and runner image
`ubuntu-26.04/20261101.186.1`. The Action is pinned to
`a1b9a37a4bf38e1883a4afc07d397616fa985942`; the binary SHA-256 is
`75d703fca2c87c21808a8556f5ca2af47b316527c58426ee008a369c05feae84`.
Artifact `13568636096` is 4,428,754 bytes, SHA-256
`1e08413cb12296c3c72504a224c7adfdeb8129670a488fce1129a31266b41a3e`.
Its archive size and digest, all four raw-trace digests, entry counts and
descriptor categories were independently checked. The
[compact evidence](unit-descriptor-origin.json) retains those identities,
the exact rejection messages and representative trace entries.
