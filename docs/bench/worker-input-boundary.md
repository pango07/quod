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

The immediate next experiment, if this path is pursued, is to distinguish
module code/content changes from transport/cache metadata in the changed
fetch responses. The variable configuration fields cannot simply be masked.
Matching test results in two runs is not proof that a test cannot read them.

This does not replace or weaken the existing native-observer investigation.
The real corrected-engine 12-commit window is running separately as
[42167641326](https://github.com/pango07/ci-bench/actions/runs/42167641326),
with engine `081f20a`, Action `1aa08ca` and harness `62a5c2d`.
See [machine-readable diagnostic evidence](worker-input-boundary.json).
