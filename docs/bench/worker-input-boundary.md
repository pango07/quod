# Worker-input boundary experiment

The corrected engine still reruns all 120 file/project instances when a
fresh checkout changes shared filesystem metadata. This experiment tests
an alternative to proving that individual main-process stat fields are
unobservable: run preparation afresh, then compare the inputs delivered to
each isolated test worker. No production behavior or replay eligibility
changes here.

[Gate 42168453693](https://github.com/pango07/quod/actions/runs/42168453693)
at `df8448c` passes three protocol/fingerprint regressions and two complete
plain Vitest runs of convex-helpers c11. Between the runs, it changes the
mtime of all 39 unique test paths by 60 seconds and verifies that every
file's content digest is unchanged. This is a diagnostic in a disposable
GitHub checkout, not a proposed timestamp normalization.

| Observation | Before | After |
| --- | ---: | ---: |
| File/project instances | 120 | 120 |
| Test states | 4,131 | 4,131 |
| Isolated fork workers observed | 120 | 120 |
| Uninspectable payloads / unknown messages / pending requests | 0 / 0 / 0 | 0 / 0 / 0 |
| Module fetch requests | 414 | 417 |
| Warm-module requests | 118 | 118 |

Both runs match fingerprint
`09ebb9e3a722220d3a21ddfe9f63992a8a074b0fcaa95764e54c37be90d469cd`.
The runner is Ubuntu 26.04 image `20261101.186.1`, Node 24.18.0, Vitest 5.0.0.
Neither run uses Quod or skips tests. Their durations are not a savings
measurement.

## What the comparison rules out

Hashing whole incoming packets is insufficient for useful reuse:

- `config.sequence` differs for all 120 workers. The inspected Vitest 5.0.0
  source defaults the global sequence seed to `Date.now()` and includes
  that seed in the serialized worker configuration. This explains a
  likely source of the difference; this diagnostic hashes the entire
  sequence object and does not independently identify its changed leaf.
- Worker IDs differ for 78 paired workers, and pool IDs for 93.
- All 120 workers have identical environment and execution-argument
  fingerprints, but none has an identical complete RPC transcript.
- All 122 module-resolution requests and 119 snapshot-path requests have
  matching arguments and responses. Of 363 paired module-fetch requests
  with the same arguments, only 79 have the same response fingerprint.
  All 118 warm-module requests have matching arguments and changed
  response fingerprints.

The complete transcript also includes worker outputs such as task updates,
whose timing values can vary. The loader-only comparison above keeps those
separate; it still finds no worker with an identical loader transcript.
The probe preserves request order and multiplicity. It does not equate an
uninspectable value with another uninspectable value.

## Implication for implementation

This establishes a measurable worker boundary, but does **not** justify
dropping shared metadata keys. A usable boundary would need to account for
the complete worker bootstrap, observable sequence/worker identity, actual
transformed module bytes and external modules, parent RPC state and order,
worker filesystem/environment inputs, and side effects. A temporary module
pathname alone is not its contents. Thread shared-memory channels, custom
pools and unknown protocol shapes also need their own treatment.

The follow-up below distinguishes module code/content changes from
transport/cache metadata in the changed fetch responses. The variable
configuration fields cannot simply be masked.
Matching test results in two runs is not proof that a test cannot read them.

This does not replace or weaken the existing native-observer investigation.
The real native-origin-engine 12-commit window completed separately as
[42167641326](https://github.com/pango07/ci-bench/actions/runs/42167641326),
with engine `081f20a`, Action `1aa08ca` and harness `62a5c2d`.
It had no full hits; see the [window result](convex-helpers-native-origin-window.md).
See [machine-readable diagnostic evidence](worker-input-boundary.json).

## Module-byte follow-up

[Gate 42169609242](https://github.com/pango07/quod/actions/runs/42169609242)
at `ed70aaa` passes four diagnostic regressions and two more complete
120-worker / 4,131-state plain runs. The same 39 metadata-only changes
preserve the established test fingerprint. The changed configuration leaf
is now identified directly: `sequence.seed` differs in all 120 pairs.

Before each transformed-module response is sent, the probe opens its
temporary file without following links or updating atime, hashes the bytes,
and verifies descriptor metadata stability around the read. It retains the
original path and response-field fingerprints. This observes bytes at send
time; it does not prove what a worker later reads or authorize replay.

Within paired workers, 1,063 transformed-module identities appear on both
sides with matching observed byte digests; none of those paired byte sets
differs. All 1,063 have different temporary-path fingerprints. Another 819
paired identities are external modules without captured bodies. There are
506 identities present on only one side and 67 paired identities with
different occurrence counts. Comparing the first paired occurrence also
finds 63 `invalidate` field differences. Fetch scheduling and warm-module
batches therefore remain part of the problem; this is not whole-transcript
equivalence. No worker has an identical complete RPC transcript.

This supports investigating validation of actual module bytes instead of
transport filenames. It does not permit dropping external-module inputs,
masking the seed/worker IDs, ignoring request order, or assuming identical
parent side effects. [Evidence and artifact digests](worker-module-bytes.json)
preserve the comparison and its limits.
