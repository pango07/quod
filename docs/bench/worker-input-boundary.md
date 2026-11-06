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

## Configuration observations inside workers

[42173150531](https://github.com/pango07/quod/actions/runs/42173150531)
(`2d8d253`) and
[42174016009](https://github.com/pango07/quod/actions/runs/42174016009)
(`3288f8c`) each pass a plain run and an observed run across the same 39
metadata-only edits. Every run has 120 file/project instances and 4,131
states with the established fingerprint. The second gate also passes the
opaque-entry and heap-observer regressions.

The observed workers receive a proxy around their actual IPC configuration.
None of the 120 workers records a `sequence.seed` read. This is not enough
to remove the seed: a regression demonstrates that `util.inspect()` reads
the target without invoking the proxy traps, while `util.types.isProxy()`
exposes the instrumentation. No replay is authorized.

The native capability diagnostic finds no cached native addons or native
Worker objects at initialization in any worker, no addon loads, no new
Inspector entries and no watched heap API entries. One worker, for
`cli/openApiSpec.test.ts`, makes three synchronous subprocess calls.
All other 119 workers have no watched opaque entries. This is a narrower
observed capability set than the parent process, which loads Rolldown's
native addon. It does not exclude every native observation channel.

The only reported proxy wrapping gap is the later-created
`snapshotOptions.snapshotEnvironment` instance in 119 workers. That gap
and the native proxy inspection gap require treatment before a worker
configuration projection could be safe. The follow-up adds native entry
observation for `isProxy` and `getProxyDetails`; it still grants no replay.
These runs are diagnostics, not speed or cost measurements.
[Evidence and artifact digests](worker-config-observations.json) retain
the results and subprocess call locations.

## Native proxy inspection follow-up

[42175546182](https://github.com/pango07/quod/actions/runs/42175546182)
at `9da3c27` passes 28 regressions and two complete 120-instance /
4,131-state runs. Six new regressions verify that native proxy inspection
is observed even through cached function references, including nested
proxy inspection that reads the seed without firing JavaScript read traps.
The independent comparison of raw per-test names and states agrees.

The real workers make zero observed `getProxyDetails` calls. They make
8,276 `isProxy` calls: 6,028 from Node's CommonJS loader, 2,157 from this
probe, and 91 from Undici. The probe records call sites and counts, not
the argument identities. These calls therefore cannot simply be dismissed
as unrelated to the tracked configuration. The initial native-capability
findings remain unchanged, including the three subprocess calls in one
worker. The seed still has no observed JavaScript reads.

This narrows a concrete obstacle to worker input projection. It does not
yet supply a safe projection: original configuration ownership, native
argument identities and other observation channels, worker identity and
module inputs, parent RPC state and side effects remain unresolved.
Production uses the existing full keys, and the published Action pin is
unchanged. No speed or cost improvement is claimed from this diagnostic.
[Result, call sites and artifact digests](worker-proxy-observations.json)
preserve the evidence without storing native arguments or configuration
values.

## Bounded native arguments

[42177178825](https://github.com/pango07/quod/actions/runs/42177178825)
at `db85d26` passes 36 regressions and two full runs with the same 4,131
states. The diagnostic reads the actual paused script, recognizes a direct
one-argument call, verifies the callee's runtime identity, and follows only
supported lexical bindings and own data properties. It never evaluates
application expressions or invokes a getter to recover the argument.
Bound calls, multiple arguments, spread arguments, proxy receivers and
accessors stay unknown. Temporary Inspector handles are released after
each observation.

Of 8,276 `isProxy` entries, 7,688 arguments are observed objects and 585
are primitives. None is observed as a proxy. Three calls remain unknown,
one each in `Counter.injection.test.tsx`, `Counter.mock.test.tsx` and
`react.test.ts`. Their stacks pass through `pretty-format` into Node's
CommonJS loader; the required own data property is unavailable. The probe
does not invoke an accessor to resolve those calls. The three unknown
workers and the separate subprocess-using worker leave 116 workers with
no flags from these particular checks. This grants no replay eligibility.

Review then identified that private-field syntax must not be treated as a
same-named public property. Fix `e1ef777` rejects it explicitly, with a
fixture whose private field is a proxy and whose public property is an
ordinary object. The short fixture gate
[42177699152](https://github.com/pango07/quod/actions/runs/42177699152)
passes 37 regressions. The full workload was not repeated for that fix;
the two gate scopes remain separate in the evidence.

Configuration ownership, other native channels, complete module and
worker inputs, parent RPC state and side effects remain unresolved. No
production key, published Action, or savings claim changes.
[Evidence, source hashes and artifact digests](worker-native-arguments.json)
preserve the native argument classifications without values or object
references.

## Code consumed inside workers

[Gate 42181213496](https://github.com/pango07/quod/actions/runs/42181213496)
at `30776ed` passes ten regressions and three complete runs: an unobserved
baseline, an observed run, and another observed run after the same 39
timestamp-only file changes. All three have 120 file/project instances and
4,131 test states. An independent comparison of the raw reports agrees.
Node is 24.18.0, Vitest is 5.0.0, and the runner image is
`ubuntu-26.04/20261101.186.1`.

The worker bootstrap observes the actual evaluator arguments and source
returned by V8's debugger. It hashes JavaScript as UTF-16 code units and
WebAssembly as bytecode, without retaining source or configuration values.
A regression demonstrates why UTF-8 is insufficient for arbitrary source
strings: two different lone surrogates encode identically but produce
different JavaScript results. Another verifies different Wasm bytecode and
results, using the protocol's separate
[bytecode field](https://chromedevtools.github.io/devtools-protocol/tot/Debugger/#method-getScriptSource).
Original evaluator return values, promises and thrown errors are preserved.

| Observation | Per observed phase / paired result |
| --- | ---: |
| Inlined evaluator calls | 645 |
| External evaluator calls | 562 |
| Workers with identical evaluator code/identity sequences | 120 / 120 |
| Captured JavaScript script events | 53,209 |
| Captured Wasm script events | 117 |
| Paired script identities with equal code and occurrence counts | 48,145 / 48,145 |
| Workers with identical combined script-event sequences | 75 / 120 |
| Changed evaluator `tmp` fields | 645 |
| Changed evaluator `invalidate` fields | 46 |

JavaScript-only and Wasm-only event sequences agree in all 120 workers.
Their relative interleaving differs in 45 workers; the combined trace is
not reordered to manufacture equality. No paired code-body or occurrence
count differs, and no identity appears on only one side. All evaluator
metadata is inspectable in this run, but its full sequence differs in every
worker. Temporary paths and invalidation flags remain explicit differences.

Two failed gates exposed diagnostic limitations before this result.
`42179523607` incorrectly expected a compiled but unbound `vm.Script` to
appear immediately in the debugger. The corrected fixture verifies deferred
reporting, and reports explicitly declare incomplete compilation coverage.
`42180282486` completed all three matching test runs but rejected unsupported
Wasm observations in 117 workers. The successful gate captures those bytes
instead of dropping the errors.

This is useful evidence for a stable code boundary across metadata changes;
it is not a replay certificate. Debugger events are not an execution trace
and may omit unbound scripts. Matching module code does not establish equal
configuration, module metadata, Wasm imports, non-code data, parent RPC state
or side effects. The configuration/native-access probes ran separately, so
their observations are not a combined certificate for these runs.

The next worker-boundary work must account for how `tmp` and `invalidate`
are consumed, along with the already identified seed and worker identities.
No production key, published Action or savings claim changes.
[Independent comparison and artifact digests](worker-module-consumption.json)
preserve this result and both failed-gate scopes.

## Stored metadata and invalidation state

[Gate 42182839376](https://github.com/pango07/quod/actions/runs/42182839376)
at `9b1b699` passes 38 regressions and two runs with the same
39 timestamp-only edits. Both runs observe parent RPC traffic; only the
second installs the metadata/native observer inside workers.
Both full runs retain all 120 file/project
instances and 4,131 test states. Independent raw-state and observation-count
checks agree. The runtime is Node 24.18.0, Vitest 5.0.0, Vite 8.2.0 on
`ubuntu-26.04/20261101.186.1`.

The pinned Vite module-runner source consumes `invalidate` before assigning
the fetched object to `module.meta`. The diagnostic therefore observes two
different things: the node state immediately before `invalidateModule`,
and later access to `tmp`/`invalidate` through stored module metadata. It
checks the actual module-runner source digest before installing its hooks.
The regressions cover cold and populated invalidations, read/descriptor/
enumeration operations, shared metadata identity, mutation, restoration,
unsupported descriptors, original exceptions and raw/native read bypasses.

| Observation | Result |
| --- | ---: |
| Observed workers | 120 |
| Module nodes / stored metadata objects | 1,208 / 1,259 |
| Metadata assignments, including invalidation clears | 1,493 |
| Invalidation calls | 234 |
| Invalidations with no prior state to clear | 234 |
| Later observed reads/enumerations of the varying stored fields | 0 |
| Proxy-unwrapping calls / proxy arguments | 0 / 0 |
| Unknown native argument calls | 3 |
| Synchronous subprocess calls | 3, in one worker |

At each observed invalidation, the node was not evaluated, had no metadata,
map, promise or exports, and had an empty imports set. These are the fields
the pinned invalidator clears. No later `get`, `has`, descriptor or key
enumeration event was observed through the installed metadata proxies.

The three unknown native arguments remain in `react.test.ts`,
`Counter.mock.test.tsx` and `Counter.injection.test.tsx`; the separate
subprocess-using worker is `cli/openApiSpec.test.ts`. This leaves 116 workers
without these particular flags, not 116 replay-eligible workers. There are
no observed addon loads, cached native addons, new Inspector entries or
watched heap-reader entries in this combined metadata/native diagnostic.

Stored-object observation is not complete ownership. The original fetched
object exists before assignment into the graph, and retained aliases or
Promise/transport observers could read it without these traps. Earlier
transport reads are also outside the observation. The successful fixture
demonstrates that raw and native reads can bypass the proxy; no projection
is enabled by a zero-read result. The next proof must account for the
object's lifetime from transport delivery, along with worker configuration,
data inputs and parent-side behavior.

[Evidence and per-artifact digests](worker-module-metadata.json) retain the
full scope. Production keys, the Action pin and performance claims remain
unchanged.
