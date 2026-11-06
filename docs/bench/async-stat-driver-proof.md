# Async stat proof: outcomes and the real driver

This work advances the proof needed to stop test-file timestamps from
invalidating every Vitest file. **No size-only dependency is enabled, and
these diagnostics establish no new speed or cost savings.**

## Native results and hidden constructor observers

The diagnostic now checks actual identities of the native stat function's
closed-over binding, converter, constructor and base constructor. A binding
replacement restored immediately after dispatch remains visible in the
operation's earlier checkpoints. Its result inspection reads the fulfilled
promise immediately before its awaiting consumer runs, through the owned
hook's private lexical context. It introduces no global object bridge,
replacement Stats object or promise reaction.

Node's rejection path returns a rejected promise from its native error
handler. The first rejection fixture incorrectly replaced the public
`then` method with a throwing function and failed. The corrected fixture
compares the diagnostic with a plain baseline, confirming the same reaction
count and caught rejection. See
[Node's promise filesystem implementation](https://github.com/nodejs/node/blob/v24.18.0/lib/internal/fs/promises.js)
and [successful gate 42152413463](https://github.com/pango07/quod/actions/runs/42152413463).

A second fixture installs an inherited `mtimeMs` setter **before** the
observer starts. Native Stats construction invokes that setter, exposing
the result to another consumer even though an initial prototype snapshot
still matches and the returned object has a normal own `size` property.
The diagnostic now checks that none of the native constructor's assigned
fields are present on its prototype chain, and pins the actual V8 source
of the constructor/converter to the audited Node 24.18.0 source.

The initial prototype-chain check conservatively rejected even the normal
case. V8 omits `[[Prototype]]` for a null prototype; the corrected traversal
accepts that omission only at the intrinsic Object prototype obtained from
a fresh literal, not through the mutable global `Object`. See
[V8's internal-property implementation](https://github.com/nodejs/node/blob/v24.18.0/deps/v8/src/runtime/runtime-debug.cc#L182).

## Production guard regression and package

The inherited setter also exposed a production type-proof gap.
[Baseline 42153208302](https://github.com/pango07/quod/actions/runs/42153208302)
passed all 11 observer fixtures and 57 proof tests; the new startup-setter
test failed because the recorder emitted a type key without retaining
complete metadata. That baseline proves incorrect dependency classification,
not a measured stale replay by the old binary.

Engine `7ff2b2f` rejects this narrowing when any constructor-assigned field
is inherited, including from the Stats base or Object prototype.
[Audit/build 42153386167](https://github.com/pango07/quod/actions/runs/42153386167)
passed 173 tests, clippy, compilation and the built-binary regressions.
The new startup-setter scenario first warmed to a replay, then changed only
a timestamp. The corrected binary replayed zero files, selected two tests
and matched plain. All metadata regressions retained wrong = 0; the useful
native type-only replay still worked after a timestamp/permission edit.

Action `7a029b726084db6682e1a44ff19c8334d01b99a0` packages the exact bundle
from [integration gate 42153886571](https://github.com/pango07/quod/actions/runs/42153886571).
Its archive is `bin-7ff2b2f`, SHA-256
`113849356097d1859d02855eb784ec68980d76aafa32cccc7cb53a366728b40a`.
Harness `ac4d9c6d72df6cf50eab299a87dbc3acdce451d6` pins that package.

## Complete suite and production execution path

[Full-suite diagnostic 42153389848](https://github.com/pango07/quod/actions/runs/42153389848)
passed 32 proof regressions and 11 observer fixtures. All 354 stat operations
had unchanged observed native bindings and absent inherited constructor
fields. The 120 test-file operations produced native size values; the 234
temporary-cache operations rejected and were caught. The full test-state
fingerprint remained unchanged. Metadata stayed complete: 285 `sm:`,
12 `sl:` and 104 `st:` diagnostic keys.

[Real-driver diagnostic 42153708463](https://github.com/pango07/quod/actions/runs/42153708463)
then exercised the Rust planner, actual driver, worker setup and benchmark
report preload. It uses the validated `4d99319` Rust binary with the current
`7ff2b2f` JavaScript through the existing `QUOD_VITEST_JS_DIR` development
override. The staged driver includes hashes of both diagnostic modules.
Its disposable ledger is not used for replay or savings measurements.

| Check | Observed |
|---|---:|
| File/project instances | 120 |
| Per-test states matching the benchmark | 4,131 |
| Certified local size consumers | 120 |
| Consumers with an inspected native size result | 120 |
| Consumers with only the owned promise callbacks | 120 |
| Consumers with unchanged native bindings | 120 |
| Consumers without inherited construction fields | 120 |
| Replayed files | 0 |

Fingerprint:
`09ebb9e3a722220d3a21ddfe9f63992a8a074b0fcaa95764e54c37be90d469cd`.

The actual ledger changes the next performance action. Its shared inputs
have **39 successful complete metadata observations**, all test files read
by the async size consumer. Its remaining shared metadata keys are failures:
242 `sm:` and 254 `sl:`. Thus the successful realpath/callback observations
from the isolated configuration diagnostic are not established shared
blockers in this production execution path. Seven other successful complete
metadata dependencies belong only to `cli/openApiSpec.test.ts`: the package
manifest, checkout root, redocly shim, and node/npm/npx/sh executables.
These per-file inputs still require separate evidence before refinement.

See [machine-readable driver evidence](async-stat-driver-proof.json).
The next step is to complete startup/native provenance and exclude other
observation channels before emitting a size query. A matching source hash,
initial snapshot, local consumer or successful benchmark alone is not enough.
The current constructor-field guard does not attest every possible earlier
preload modification. Production replay integration and a fresh-runner hit
remain unverified, followed by the required complete 12-commit window.
