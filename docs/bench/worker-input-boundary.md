# Worker-input boundary experiment

The corrected engine still reruns all 120 file/project instances when a
fresh checkout changes shared filesystem metadata. This experiment tests
an alternative to proving that individual main-process stat fields are
unobservable: run preparation afresh, then compare the inputs delivered to
each isolated test worker. No production behavior or replay eligibility
changes here.

**Native-argument correction:** earlier reports reconstructed property
arguments from their descriptors after argument evaluation. A getter can
return a proxy and replace its property before that observation. Therefore
those earlier object/primitive classifications are not evidence of the
actual passed value for member expressions. Commit `e35bbb1` rejects such
arguments as unknown and adds self-replacing-getter regressions. The
historical counts below remain recorded diagnostic output, not a native
access exclusion certificate. The corrected combined run below leaves
6,028 CommonJS property arguments unknown in each observed phase, across
all 120 workers.

**Native-callee correction:** the same ambiguity also applies to the
function expression. A property getter can return a bound intrinsic, then
replace its own property or owner with the unbound intrinsic before the
pause. Comparing that later value can assign the wrong argument positions.
Commit `2d0f3dd` therefore leaves all member callees unknown. Earlier member-
callee classifications, including subsequent corpus diagnostics built
before this correction, cannot establish actual argument identity.

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

## Hook lifetime observation

[Gate 42185034887](https://github.com/pango07/quod/actions/runs/42185034887)
at `4bfd278` passes 52 regressions and repeats the two full runs with 39
timestamp-only changes. Independent comparison confirms the same 120
file/project instances and 4,131 test states. All workers report empty
Promise/async-hook registries at the beginning and end, and zero entries
into the watched registration, async-resource access and private-binding
functions during observation. The 235 module invalidations are all cold;
the stored metadata observer again records no reads of the varying fields.

The observer watches the actual function objects, including cached
references and native installation functions. A fixture registers a hook,
retains a resolved metadata object, then unregisters it: an empty final
registry alone would miss this access, while the lifetime observer catches
it. Nine new fixtures cover the four Promise registration variants, cached
registration, async hooks, resource access, private binding access and the
inactive case. The earlier fixture gate failed because the four Promise
registration closures share a V8 breakpoint location; they now use one
truthfully grouped label. A separate fixture now uses an available private
binding instead of assuming `async_wrap` is publicly exposed.

This narrows one pre-assignment alias channel but does not establish full
metadata ownership. Transport callbacks and direct raw aliases still need
observation, as do worker configuration, non-code inputs and parent-side
behavior. No production replay is enabled by this result. The runtime
remains Node 24.18.0, Vitest 5.0.0, Vite 8.2.0 on
`ubuntu-26.04/20261101.186.1`.
[Independent verification and artifact digests](worker-hook-lifetime.json)
preserve the completed run and failed predecessor scope.

## Metadata from transport delivery

[Gate 42186718807](https://github.com/pango07/quod/actions/runs/42186718807)
at `c6a2873` passes 57 regressions and two complete runs across the same
39 timestamp-only changes. Independent artifact analysis confirms all
120 file/project instances and 4,131 test states agree. The runtime and
runner image match the preceding hook-lifetime gate.

The observer now wraps fetch metadata when the worker receives its RPC
response, and wraps the derived result before the awaiting Vite loader
receives it. The transport wrapper returns the original Promise and
preserves its receiver, thrown errors and rejections. Five new regressions
cover these properties, shared warm-result aliases, descriptors, early raw
aliases, unsupported thenables and accessors.

| Observation | Result |
| --- | ---: |
| Module RPC requests / responses | 533 / 533 |
| RPC metadata objects delivered, including warm batches | 3,222 |
| Transport invocations / settled results | 2,640 / 2,640 |
| Unique observed metadata objects | 4,452 |
| Workers with metadata reads before graph assignment | 120 |
| Workers with observed reads after graph assignment | 0 |
| Invalidation calls / invalidations of empty state | 235 / 235 |
| Hook registrations or watched async-resource accesses | 0 |

The 533 responses also equal the independently recorded parent-side fetch
and warm-fetch request count. No response or transport Promise is left
pending. All observed `tmp` accesses have call sites in the pinned Vitest
loader: checking for a temporary file, reading its contents, or spreading
the response into a new object. The `invalidate` field is copied there and
checked by Vite's `getModuleInformation`; every ensuing invalidation has
no previous state to clear. The zero-read result from the earlier probe
therefore applied only to metadata already stored in the graph, not its
whole lifetime.

These reported call sites match the inspected source; this probe does not
independently authenticate the executable script identity of each read.
Earlier IPC listeners, retained raw aliases and custom transport callbacks
remain explicit ownership gaps. Three native arguments are still unknown,
and the OpenAPI test worker still invokes three subprocesses. Actual module
byte equality was established in a separate run and has not been combined
here with configuration and complete non-code input coverage.

The next implementation step must combine these observations and reject
unaccounted channels before removing shared metadata dependencies. This is
still a diagnostic, with `replayEligible: false`; the Action and savings
claims are unchanged.
[Independent counts, call sites and artifact digests](worker-transport-metadata.json)
preserve the evidence.

## Executable identity of metadata readers

[Gate 42188370043](https://github.com/pango07/quod/actions/runs/42188370043)
at `0179811` passes 62 regressions and both complete runs. Independent
artifact analysis again confirms 120 file/project instances and 4,131
matching test states. Every one of the 6,442 observed metadata operations
resolves to the actual V8 script and exact UTF-16 source digest of one of
the two pinned Vitest/Vite loader modules. There are no unmatched reader
sources. All 530 module RPC requests have responses, and all 236
invalidations clear empty state. No stored-field reads or watched hook
entries appear.

The lookup uses the native call site's script identity, exposed by
[Node's call-site API](https://nodejs.org/download/release/v24.4.0/docs/api/util.html#utilgetcallsitesframecount-options),
and retrieves the corresponding source from the existing Inspector
session. It does not trust a displayed filename or `Function#toString`.
Five regressions cover actual ESM source, identical filenames with different
bodies, forged function text, source-URL annotations, exact UTF-16 code
units, closed sessions and unavailable source evidence. The failed
predecessor `42188179118` had an incorrect fixture expectation: Node's
native call-site API retains the original resource name despite a
source-URL annotation. The corrected fixture checks that behavior and the
Inspector annotation flag.

This establishes executable identity for the operations observed through
the metadata proxies. It does not establish complete object ownership or
exclude earlier raw aliases, and source equality alone does not establish
callback closure state or parent-side effect equivalence. Configuration
and consumed-code observations still need a combined run before they can
be used together. No replay or savings claim changes.
[Independent source comparison and artifact digests](metadata-reader-source.json)
retain the complete result.

## Combined worker observations

[Gate 42189187106](https://github.com/pango07/quod/actions/runs/42189187106)
at `e98f671` passes 73 regressions and three full runs: an unobserved
baseline, an observed run, and another observed run after the same 39
timestamp-only edits. Independent raw-state comparison confirms all three
have the same 120 file/project instances and 4,131 test states. Runtime and
runner image remain unchanged.

Configuration reads, evaluated module inputs, compiled JS/Wasm, metadata
delivery, executable reader identity and native access now coexist in the
same observed workers. One Inspector session records compilation and
watches native entries. The evaluator can read metadata through an
accessor installed by the diagnostic only while that exact getter and
setter remain installed; a replacement getter is rejected without calling
it. Two new regressions cover this ownership check and combined JS/Wasm
capture without an additional Inspector connection.

| Observation | Before metadata edit | After metadata edit |
| --- | ---: | ---: |
| Inline / external evaluations | 645 / 562 | 645 / 562 |
| Module RPC requests / responses | 521 / 521 | 516 / 516 |
| Observed metadata operations matching pinned executable source | 6,451 / 6,451 | 6,456 / 6,456 |
| Invalidations / invalidations of empty state | 235 / 235 | 235 / 235 |
| Observed stored-field / configuration seed operations | 0 / 0 | 0 / 0 |
| JS / Wasm script events | 108,354 / 117 | 107,985 / 117 |
| Configuration instances with a non-plain snapshot environment gap | 119 | 119 |
| Unknown native arguments / synchronous subprocess calls | 3 / 3 | 3 / 3 |

All **120 paired workers have identical evaluated code/identity
sequences**. Every per-worker compiled identity/body record appears on
both sides. The complete compiled sequences agree in only 27 workers;
their multisets agree in 48. All occurrence-count differences have the
exact body of `(function(other){return this===other})`, used by the
diagnostic's Inspector identity checks. This is evidence consistent with
instrumentation counts varying, not independently established ownership
of every such compilation. The differing order and counts remain in the
evidence and are not normalized into an equality claim.

The configuration gap is `snapshotOptions.snapshotEnvironment`. Raw
aliases, callback ownership, full filesystem/environment inputs, opaque
channels and parent effects still prevent this from being a replay
certificate. In particular, zero seed reads through a proxy cannot alone
exclude reads through an earlier raw alias. The next implementation must
close those paths while preserving object behavior before it can remove
the shared metadata dependencies. Production replay and savings claims
remain unchanged.
[Independent phase counts, raw comparisons and artifact digests](worker-combined-boundary.json)
retain the complete combined result.

## Scalar aliases and corrected native arguments

[Gate 42192515306](https://github.com/pango07/quod/actions/runs/42192515306)
at `e35bbb1` passes 82 regressions and three full runs. Independent
comparison confirms the same 120 file/project instances and 4,131 test
states in the baseline and both observed phases. All 120 workers retain
the installed seed accessor and record zero scalar seed operations in
both phases. The runtime and runner image are unchanged.

The new scalar observer installs an accessor on the original sequence
object instead of relying solely on a proxy. A regression reads through
an earlier object alias and through a class holding that alias; both reads
are observed without replacing the sequence object's identity. Ordinary
own writes and restoration preserve the tested value behavior. Six
regressions also retain the limits explicitly: a primitive copied before
installation is invisible, descriptors/native inspection can distinguish
the accessor, intermediate descriptor replacement can evade a final
intact check, and alternate write receivers or object-valued replacement
are unsupported. A zero-read result still grants no replay permission.

The native-argument correction is separate and necessary. A getter can
return a proxy, replace its property with an ordinary value, and finish
before the native-call breakpoint. Inspecting the later own data property
then misidentifies what was passed. The revised observer rejects property
arguments instead of reconstructing them from that later state. Three
new regressions cover replacement of the getter, replacement of its owner
binding, and an ordinary property argument that must also remain unknown.

In each full observed phase, **6,028 calls from Node's CommonJS loader
remain unknown across all 120 workers**. The earlier reports' smaller
unknown counts cannot support exclusion of native proxy observations.
Direct supported bindings still have their actual runtime identity checked.

This suggests observing metadata fields on their original objects, with
separate checks for descriptor access, rather than depending on proxy
identity remaining invisible. Such a check must account for native
descriptor queries, property mutation and values copied before observation;
the accessor alone is not a complete solution. No Action or production key
changes, and no new savings measurement is claimed.
[Independent scalar, argument and combined-run evidence](worker-scalar-seed.json)
preserve the corrected counts and remaining gaps.

## Original-object metadata and descriptor access

Commit `2de3436` adds an accessor mode that observes existing `tmp` and
`invalidate` scalar fields on the original metadata object. References
retained before observation now reach the observer; `isProxy` and object
identity retain their original values. Absent properties stay absent.
The evaluator obtains a fingerprint snapshot only from owned, intact
accessors. Replaced or deleted accessors make that snapshot unavailable.
Earlier primitive copies, intermediate deletion/restoration, descriptor
inspection, and other native channels remain explicit gaps.

The same combined diagnostic watches 19 native descriptor-related entry
functions, including cached references to Object/Reflect reads and
mutations. Argument recovery permits additional arguments only when all
are supported, side-effect-free lexical reads, `this`, or primitive
literals. Calls whose arguments can mutate earlier bindings remain unknown.
This finite API set does not cover syntax deletion or every native channel.

[Gate 42195814149](https://github.com/pango07/quod/actions/runs/42195814149)
passes **104 regressions**, but the full comparison fails: its first
observed phase reaches the existing 420-second timeout. The unobserved
baseline completed all 120 file/project instances and 4,131 states. Only
15 observed worker reports completed; they contain 700 instrumented
metadata scalars and 145,981 descriptor calls. Those partial records have
no reported observation errors or overflows. They are **not** a complete
corpus comparison or evidence of restored replay.

Follow-up `7be1dfe` removes redundant per-handle cleanup only on pinned
Node 24.18.0. The [protocol](https://chromedevtools.github.io/devtools-protocol/tot/Runtime/#method-getProperties)
specifies inherited object groups, and the pinned
[Node implementation](https://github.com/nodejs/node/blob/v24.18.0/deps/v8/src/inspector/v8-debugger-agent-impl.cc#L1486)
releases the paused backtrace group on resume. Other runtime versions keep
individual cleanup. Unsupported argument syntax is now rejected before
resolving any bindings, with the same unknown classification.

[Fixture gate 42197573577](https://github.com/pango07/quod/actions/runs/42197573577)
passes **69 regressions**, including a new lifetime check: all temporary
argument-property handles become invalid after resume, while the watched
intrinsic remains valid. It makes no full-workload speed claim.
The optimized full comparison,
[42197790745](https://github.com/pango07/quod/actions/runs/42197790745),
passes 105 regressions but also reaches the 420-second limit in its first
observed phase. Again only 15 workers finish. Their 145,443 descriptor
calls and 676 metadata scalars have no reported observation errors or
overflows. The cleanup optimization alone did not make full coverage
practical; the incomplete result must not be promoted to a corpus pass.

Commit `7392bcd` recognizes later argument construction that cannot replace
an earlier target binding: bounded object/array literals, function creation,
and unary operations without user conversion hooks. Computed effects,
spreads, property reads and calls remain unknown. Ten regressions include
self-changing bindings, getter side effects, conversion hooks and default
parameters that must not execute. [42198435527](https://github.com/pango07/quod/actions/runs/42198435527)
passes 79 fixture tests. Commit `122ae5c` caches syntax lookup by call
location, while re-reading current runtime targets. Its same-location,
alternating-target regression and the other 78 fixtures pass in
[42198746560](https://github.com/pango07/quod/actions/runs/42198746560).

Commit `1e14732` also writes bounded progress checkpoints for unfinished
workers, so a timeout can identify the APIs and callers still consuming
time. Checkpoints retain cumulative counts; no native events are skipped.
The combined [42199994352](https://github.com/pango07/quod/actions/runs/42199994352)
passes 116 regressions but reaches the same 420-second limit. Twenty-one
workers finish. The unfinished `stream.property.test.ts` worker alone
reaches 200,000 descriptor calls; its last checkpoint is in Vitest's
`setState`. This identifies assertion bookkeeping as a major source of
observer work. The calls remain observable and cannot simply be ignored.
No timeout was increased, and no action pin, production key, or savings
claim changes.
[Partial evidence and artifact digests](worker-metadata-accessors.json)
retain full-run failures and passing fixture gates separately.

## Descriptor-observer overhead

Commit `11f9f4` compares actual native-function identities with nonzero V8
heap IDs instead of compiling a JavaScript comparison for each call.
[42201238868](https://github.com/pango07/quod/actions/runs/42201238868)
passes 82 fixtures, including aliases, distinct closures, cleared IDs and
bound callees. One hundred comparisons create zero scripts, versus 100
with the old comparison. No heap snapshots are requested. Heap tracking
and GC remain observable, so this is a diagnostic, not a reuse certificate.

The full attempt at `09efe9f`,
[42201706092](https://github.com/pango07/quod/actions/runs/42201706092),
passes 118 regressions but still times out in the first observed phase.
Only nine workers finish, versus 21 in the preceding attempt. The
unfinished property-test worker reaches 150,000 descriptor calls, with
451 compiled-script events and no compilation overflow at that checkpoint.
The small fixture's faster comparisons therefore do **not** establish a
full-workload improvement. Both baselines complete 120 instances and
4,131 states; neither observed run completes. Both use Node 24.18.0 on
`ubuntu-26.04/20261101.186.1`.

Commit `2092649` next caches only an overapproximation of names declared
in the actual ES module source. When a name cannot be declared there, it
can avoid serializing the module's entire scope. All actual values,
targets and native identities are still read afresh. Local/closure/script
scopes and unknown scope locations keep descriptor reads. Counters record
descriptor request and result volume by scope kind. Live Inspector tests
cover mutable module aliases, live imports, destructuring, shadowing and
dynamic local bindings. The first fixture run recovered the changed values
but failed its descriptor-volume assertion: V8 had omitted unused ballast
declarations. Commit `e412386` exports those bindings to retain the large
scope. [42204149648](https://github.com/pango07/quod/actions/runs/42204149648)
then passes all 86 fixture tests. The full comparison,
[42204247885](https://github.com/pango07/quod/actions/runs/42204247885),
passes 122 regressions but reaches the same 420-second timeout, with only
19 worker reports complete. The property-test worker reaches 370,000
descriptor calls, versus 150,000 in the preceding heap-identity attempt.
Its checkpoint records 183,619 module-scope descriptor requests returning
23,949,683 entries, after avoiding 91,322 absent-name lookups. These partial
counts locate the remaining cost; they are not a complete workload timing.

Commit `48910d1` stores compiled-script records with lossless run-length
encoding (`rle-v1`). Every field, occurrence count and sequence position
remains significant; only adjacent identical records merge. The existing
20,000-entry cap now bounds runs, and any overflow still fails the
diagnostic. Weighted comparisons and independent verification count
occurrences rather than compressed rows. [42205036879](https://github.com/pango07/quod/actions/runs/42205036879)
passes 88 fixtures, including 20,005 repeated live compilations followed by
a different body and the original body. All occurrences and ordering
survive, without overflow. This avoids dropping evidence from repeated
Inspector evaluations; it does not change Inspector's coverage limits.

Commit `beffe3b` adds optional lookup of a bare identifier declared directly
in the actual module source. It first inspects preceding supported scopes,
then uses `Debugger.evaluateOnCallFrame` with side-effect checking and the
backtrace object group on pinned Node 24.18.0. Global/member expressions,
unknown scopes and lookup errors remain unknown. Values are never cached;
descriptor reuse stops after evaluation. [Module binding semantics](https://tc39.es/ecma262/multipage/executable-code-and-execution-contexts.html#sec-module-environment-records-getbindingvalue-n-s)
and the pinned [Inspector implementation](https://github.com/nodejs/node/blob/v24.18.0/deps/v8/src/inspector/v8-debugger-agent-impl.cc#L1573)
inform the experiment, without providing a general replay certificate.

[42205726797](https://github.com/pango07/quod/actions/runs/42205726797)
passes 93 fixtures. Changed module aliases, live imports, destructuring,
shadowed/dynamic locals and target getters retain their checked behavior.
The module-alias fixture performs three current-value lookups with zero
module-scope enumerations. A queued microtask that replaces the target
does not run during the native pause and runs normally afterward.
Commit `6751d1e` enables the path in the combined diagnostic;
[42205868477](https://github.com/pango07/quod/actions/runs/42205868477)
is running at this revision. No production replay behavior or savings
claim changes.
[Raw checkpoint evidence and artifact digests](worker-descriptor-overhead.json)
preserve partial results separately from complete worker reports.

## Callee provenance correction

[42206709275](https://github.com/pango07/quod/actions/runs/42206709275)
at `2d0f3dd` passes 96 fixtures. Two new cases return a bound native
descriptor function from a getter and replace either the property or the
owner before the native pause. The original call returns the bound target's
descriptor, while the member now contains the unbound intrinsic. Both stay
unknown, as does an ordinary member call that cannot be distinguished from
this history at the pause. Supported lexical callees still resolve their
current identities. The earlier argument-property restrictions remain.

Commit `aac46f7` captures `isProxy` in lexical bindings inside Quod's own
diagnostic helpers. This prevents later public-property replacement from
changing those helpers' inspection and avoids reconstructing their callees
from a mutable member. [42206893389](https://github.com/pango07/quod/actions/runs/42206893389)
passes 97 fixtures, including a public replacement that throws if called.
Fingerprinting still identifies ordinary objects and rejects proxies;
neither the replacement nor a proxy trap runs. Native-entry observations
retain the actual captured calls. This does not certify initial runtime
provenance, complete native coverage or replay safety.

## Fresh module scope lookup replaces the blocked identity cache

The lexical-evaluation full run [42205868477](https://github.com/pango07/quod/actions/runs/42205868477)
finished with a 420-second timeout in the first observed phase. All 129
regressions passed, and plain completed 120 instances / 4,131 states, but
only 11 observed workers completed. The unfinished stream property worker
reached 130,000 descriptor calls and 63,407 lexical evaluations. Transferring
fewer module descriptors did not make this run fast enough. It also predates
the member-callee correction, so its classifications cannot certify identity.

The private-callee cache introduced at `a06d464` is removed. Its live positive
fixture never passed: the guard correctly refused reuse because two native
Connection-shaped objects appeared at startup. Neither was a prototype;
only one matched a Session's private connection. The last failing gate was
[42209923638](https://github.com/pango07/quod/actions/runs/42209923638) at `d413886`.
[V8's template cache](https://github.com/nodejs/node/blob/v24.18.0/deps/v8/src/api/api-natives.cc#L261-L331)
provides a plausible source of the extra object, not an identity proof.
Increasing the allowed count would not resolve that uncertainty.

The replacement experiment reads one binding from V8's fresh module scope
object on every native pause, then compares its actual native identity.
[ScopeObject](https://github.com/nodejs/node/blob/v24.18.0/deps/v8/src/debug/debug-scopes.cc#L579-L621)
materializes a null-prototype object with lexical values and throwing native
accessors for unavailable bindings. Only a declared module binding on the
pinned runtime qualifies; prior scopes are checked first. A side-effect-checked
lookup uses that diagnostic scope object, never an application object. Values
are not retained across pauses, and exceptions remain unknown. Mutation, live
imports, shadowing, queued microtasks, foreign Inspector connections and
same-source cross-context modules are retained as regressions. This candidate
passes 109 regressions in [42211683490](https://github.com/pango07/quod/actions/runs/42211683490)
at `b025766`. It is not enabled in the full workload or product. A bounded
300-call comparison will measure each approach with identical values and
one native identity comparison per call before another full workload run.

The first equal-work fixture failed in [42211891164](https://github.com/pango07/quod/actions/runs/42211891164):
reading the clock lazily initialized `perf_hooks`, adding ten descriptor calls.
`1242d87` moves clock initialization before observation. Its
[gate 42212082012](https://github.com/pango07/quod/actions/runs/42212082012) passes
110 tests. For 300 calls, transferring all module descriptors took 1,516 ms;
lexical evaluation took 520 ms; fresh scope lookup took 495 ms. All methods
made 300 identity checks and returned the same values without invoking getters.
The lookup improvement over lexical evaluation is small, not sufficient
evidence for another full-workload run.

[42212429550](https://github.com/pango07/quod/actions/runs/42212429550) at `c813670`
also passes 110 tests and compares fresh compiled identity checks with heap
tracking. In that run the same three methods took 2,061 / 718 / 683 ms;
lookup with compiled identity checks took 702 ms. Removing heap tracking
did not improve the bounded measurement. These fixtures had compilation
logging disabled, unlike the full diagnostic. The next comparison enables
that logging and measures Inspector commands to identify the remaining cost.
These are diagnostic timings, not CI savings or a production replay change.

## Pause construction is now the dominant measured cost

Full compilation logging and inclusive Inspector command timing pass all
110 regressions in [42212940071](https://github.com/pango07/quod/actions/runs/42212940071),
commit `7704825`. The follow-up
[42213337485](https://github.com/pango07/quod/actions/runs/42213337485), commit
`7bc8cfb`, also passes 110 tests and times the entire pause callback. Every
mode observes all 300 calls, performs 300 identity comparisons, returns
identical values and invokes no application getter.

| Method, full compilation logging | 300 calls, ms | Inside pause callback, ms | Outside callback, ms |
| --- | ---: | ---: | ---: |
| Full module descriptors + heap identity | 2,691.9 | 1,940.5 | 751.3 |
| Lexical evaluation + heap identity | 960.9 | 259.8 | 701.0 |
| Fresh scope lookup + heap identity | 931.4 | 236.0 | 695.4 |
| Fresh scope lookup + compiled identity | 977.9 | 281.9 | 696.0 |

For the fastest mode, 74.7% lies outside the diagnostic pause callback.
This remainder includes runtime pause preparation, notification delivery,
resumption and the application calls; the measurement does not individually
attribute those components. V8's
[currentCallFrames](https://github.com/nodejs/node/blob/v24.18.0/deps/v8/src/inspector/v8-debugger-agent-impl.cc#L1641-L1717)
builds scope chains for every frame before emitting a pause. That is a
source-backed lead for the large remainder, not a measured breakdown.
Blackbox-pattern configuration does not add a frame filter in that loop.

Further binding lookup tuning cannot remove most of this measured cost.
The next investigation must reduce repeated pause construction or safely
reduce the number of observed pauses while preserving coverage, including
unknown calls. None of these experiments authorizes ignoring an input. The
fresh lookup remains disabled in the combined workload, whose latest full
attempt still failed. The packaged Action and the latest full 12-commit
product result remain unchanged. No new savings are claimed, and another
full workload run is not justified solely by the approximately 3% improvement
over lexical evaluation in this bounded full-logging fixture.

## Native entry arguments without full debugger pauses

Pinned V8 [evaluates native function-entry conditions with the topmost
arguments](https://github.com/nodejs/node/blob/v24.18.0/deps/v8/src/debug/debug.cc#L852-L894),
using an [extension object containing the actual arguments and receiver](https://github.com/nodejs/node/blob/v24.18.0/deps/v8/src/debug/debug-evaluate.cc#L117-L156).
A thrown condition exception is cleared from the application and
[reported to Inspector](https://github.com/nodejs/node/blob/v24.18.0/deps/v8/src/inspector/v8-debugger.cc#L674-L694).
The diagnostic uses this path to observe actual native values, including
bound arguments, without reconstructing the caller expression or creating
all lexical scopes for a full pause. It installs no public callback or
global bridge. Each condition carries a private nonce; reports contain no
raw application arguments.

[42214239789](https://github.com/pango07/quod/actions/runs/42214239789),
commit `df1e12f`, passes 112 tests. Wrapped, bound, replacing-getter,
apply and foreign-context calls preserve their actual arguments. Caught
condition exceptions produce zero debugger pauses and do not reach the
application's uncaught-exception handler.

[42215330256](https://github.com/pango07/quod/actions/runs/42215330256),
commit `02d9bb0`, passes 116 tests. The helper clears its own exception
wrapper and releases temporary Inspector handles. V8 console storage
otherwise retains those wrappers; clearing only remote handles is
insufficient. A WeakRef regression confirms an ephemeral argument is
collectible after observation. Proxy traps and target getters do not run;
queued microtasks remain queued; native call-site records retain the actual
caller. The synthetic exception is visible to other Inspector sessions and
can affect console history. This remains diagnostic instrumentation.

[42216738359](https://github.com/pango07/quod/actions/runs/42216738359),
commit `92abaaa`, passes 122 tests and integrates the helper as an opt-in
observer. All 19 listed descriptor APIs are observed with zero full pauses;
receiver-based APIs, bound replacing callees, later argument mutations,
native errors and proxy targets retain their checked behavior. The default
observer remains unchanged. A bounded 300-call fixture with 2,000 module
bindings took 585 ms for an empty pause handler and 81 ms for native capture;
the preceding gate measured 1,020 ms and 214 ms respectively. These are
diagnostic microbenchmarks, not full-job savings.

Alternating condition and cleanup compilations defeat adjacent run-length
compression. `dictionary-v1` stores complete records once and an ordered
run index, preserving every source digest, event and occurrence count.
Regression checks cover 40,002 alternating/changing records, malformed
indices, differing event order and capacity overflow. No source, including
nonce-bearing diagnostic source, is masked or dropped.

Commit `9f6dab7` enables this gated path in the combined diagnostic and
requires all native events to be attributed and every wrapper cleared.
[42216898878](https://github.com/pango07/quod/actions/runs/42216898878)
passes 158 regressions and completes plain's 120 instances / 4,131 states,
but the first observed phase again reaches its 420-second timeout.
It completes 43 worker reports, versus 11 in the previous full attempt.
Those reports contain 420,394 native events, with every wrapper cleared,
but five descriptor logs overflow; 8,005 descriptor occurrences are absent
from their retained entries. No complete observed phase or metadata-only
before/after comparison is established. The stream property-test worker's
last checkpoint reaches 430,000 descriptor calls, with no capture errors
or compilation-log overflow. These cumulative checkpoints must not be
added to final reports for the same worker.

The completed reports issued 17,192,026 individual handle releases.
The pinned [Inspector runtime implementation](https://github.com/nodejs/node/blob/v24.18.0/deps/v8/src/inspector/v8-runtime-agent-impl.cc#L569-L637)
assigns property, internal-property and private-property handles to their
parent handle's group. The next bounded experiment gives each exception
event a private group, clears the wrapper as before, and releases that
group together. It must leave unrelated handles usable and allow argument
objects and descriptor closures to be collected. Exact stack compression
also needs to resolve the detailed-entry cap without discarding frames.
[Raw counts, checkpoints and artifact hashes](worker-native-entry-overhead.json)
preserve this failed attempt. Complete native coverage, framework-object
ownership and production replay safety remain unproven; the packaged
Action and 12-commit savings result are unchanged.

The private-group experiment at `6902679` passes 125 regressions in
[42218887590](https://github.com/pango07/quod/actions/runs/42218887590).
Actual arguments, receivers, native errors and queued microtasks retain
their behavior. An argument object and its accessor's captured object are
collectible after cleanup, while an unrelated Inspector handle remains
usable. With full compilation logging and 80 function-valued target
properties, 300 calls take 661 ms with individual releases and 571 ms with
private groups. Release commands fall from 26,700 to 600, but the extra
group-transfer evaluation adds 300 compilations. The 13.7% measured gain
alone does not justify another full run. This mode remains disabled in
the combined workload. `04a797c` next measures requesting only the target
accessors that the diagnostic records, avoiding unused data-property
handles without discarding any retained accessor evidence.

[42219141965](https://github.com/pango07/quod/actions/runs/42219141965)
at `04a797c` passes 126 regressions. Accessor-only target inspection retains
both own getter and setter evidence, excludes inherited accessors and data
fields, and invokes no accessor. All 300 calls in each measured mode retain
the same target evidence and application result with complete compilation
logging:

| Target inspection | Cleanup | 300 calls, ms | Release commands | Compilation events |
| --- | --- | ---: | ---: | ---: |
| All properties | Individual | 637.7 | 26,700 | 609 |
| All properties | Private group | 484.0 | 600 | 909 |
| Accessors only | Individual | 320.5 | 2,400 | 609 |
| Accessors only | Private group | 343.5 | 600 | 909 |

Accessor-only inspection with individual cleanup reduces this diagnostic
microbenchmark's elapsed time by 49.7%. It is still disabled in the combined
workload pending integration with lossless stack-frame compression to
resolve the five overflowing logs. Do not translate this bounded diagnostic
result into a product savings percentage. No new production replay or
12-commit measurement has run.

## Lossless stack dictionaries and the next combined attempt

`0a42b0f` integrates accessor-only target inspection with individual handle
cleanup and passes 130 regressions in
[42219612107](https://github.com/pango07/quod/actions/runs/42219612107).
Native-entry reports now store complete frame records once and retain the
ordered frame indices on each grouped event. Frame fields, script identity,
stack order, argument evidence and occurrence counts remain significant.
The bounded capacity is 100,000 distinct event records and 20,000 distinct
frames per log; overflow or invalid references still reject the evidence.
The default full-pause observer retains its existing report shape.

The regression includes 12,001 distinct stacks / 24,002 occurrences, beyond
the previous entry cap. A separate Python check reconstructs every retained
record from worker 5277 in the failed run, preserving all fields and counts.
Its 10,000 retained records use 2,870 distinct frames: the serialized payload
falls from 53,968,352 to 4,156,190 bytes. This checks compression only; it
cannot recover the occurrences already lost by that old overflowing log.

`8dd2505` gives the native diagnostic a bounded four-million-run compilation
index, because the prior partial stream worker already recorded 861,284
compilations. It adds inclusive Inspector command timings to final worker
reports and complete current stacks to progress checkpoints. The new full
attempt is [42220018925](https://github.com/pango07/quod/actions/runs/42220018925).
It passes 166 regressions and plain completes 120 instances / 4,131 states,
but the first observed phase again reaches the 420-second timeout. It
completes 87 worker reports, versus 43 in the preceding attempt. Every one
of their 822,188 descriptor calls is retained, with zero log overflows or
capture errors. All 866,853 native events have cleared exception wrappers.
The unfinished stream worker reaches 680,000 descriptor calls, represented
by 2,871 event records and 1,817 distinct frames; its 1,361,372 compilation
events also fit without overflow. This confirms the trace-capacity fix,
not full-suite agreement or new production savings.

Across the 87 completed workers, inclusive Inspector command times are
251.4 s for 2,568,595 property requests, 149.8 s for 20,550,724 individual
handle releases, 101.5 s for 868,593 function calls and 33.3 s for 1,779,436
script-source requests. These sums span concurrent workers, may overlap
through synchronous notifications, and exclude the unfinished stream
worker; they are not a job wall-time breakdown. The busiest checkpoint's
actual caller is Vitest's `setState`, reached from `expect` in the stream
property test. [Raw evidence and artifact digests](worker-native-accessor-overhead.json)
preserve the partial observation and timings.

The next bounded experiment should combine owned-wrapper validation,
argument inspection and wrapper cleanup in one private Inspector
operation, returning only sanitized summary data. It must use captured
intrinsics, avoid application getters/proxy traps and public global
bridges, preserve actual arguments and receivers, and retain unrelated
exception/handle and collection checks. No full rerun is justified until
that path is gated and its complete-logging cost is measured. Production
replay eligibility and the 12-commit savings result remain unchanged.

## Private summary rejected; direct argument fixture passed

The private summary helper at `f9d1ef3` passed all 140 fixture tests in
[gate 42222713382](https://github.com/pango07/quod/actions/runs/42222713382),
but made the diagnostic slower. For 300 calls with complete compilation
logging, the existing accessor-only protocol path took 226 ms versus
690 ms for the helper on a narrow target. With 80 extra function fields,
the times were 322 ms versus 4,293 ms. The helper reduced protocol calls,
but its own descriptor operations caused additional native breakpoint
condition compilations: 3,911 / 27,911 events versus 609 in either existing
case. The passing tests do not turn that overhead into a useful improvement.
[Measurements and log digest](worker-native-private-summary.json) preserve
the rejected experiment.

The helper is removed. The next candidate places the actual first native
argument directly in the existing private exception wrapper, alongside
the argument count. Receiver-based APIs carry the actual receiver instead.
It retains Inspector accessor inspection and avoids a separate request
for the synthetic arguments array. Zero-argument calls avoid reading an
inherited array element. The existing array format remains the default;
the direct format remains an opt-in diagnostic.
Neither format establishes input ownership or authorizes production replay.

Direct transport at `2af69c7` passes all 141 tests in
[gate 42224572165](https://github.com/pango07/quod/actions/runs/42224572165),
including absent versus explicit undefined arguments, receiver-based calls,
replacing bound callees, accessor preservation, proxy traps, queued work,
foreign exceptions, modified public intrinsics and garbage collection.
For each 300-call measurement, property requests fall from 900 to 600 and
individual handle releases from 2,400 to 900. Both modes retain all 609
compilation events, all 300 arguments and cleared wrappers. Narrow-target
time changes from 226 to 218 ms; wide-target time changes from 316 to
255 ms. These are single fixture samples, not CI savings.
[Measurements and log digest](worker-native-direct-arguments.json) retain
the exact samples. The next step is integration into the combined observer,
with explicit mode assertions, followed by the full 120-worker measurement.

Commit `56deee7` enables the direct format in the combined diagnostic and
requires it in the integration assertions. Full workload gate
[42224978054](https://github.com/pango07/quod/actions/runs/42224978054)
passed 177 regression tests, then hit the 420-second `before` phase limit.
It completed 104 worker reports, up from 87 in the preceding run. Across
those completed reports, all 985,778 descriptor calls and 51,615 proxy
calls are retained, all 1,037,393 exception wrappers are cleared, and the
independent artifact check finds no overflow, capture error, count mismatch
or empty captured stack. The baseline completes all 120 instances and
4,131 states, but there is no complete observed phase or `after` comparison.

The unfinished stream property worker reaches 850,000 descriptor calls
(previously 680,000) with 2,871 distinct records and 1,817 frames. It has no
reported overflow. Its checkpoint is taken inside the current event, so
one not-yet-cleared wrapper there does not imply a cleanup failure.

The completed reports contain 10,727,435 enumerated accessor properties.
The largest target has 2,247 own accessors, with a caller named `__export`
in TypeScript. The observer makes 2,038,354 property requests
and 17,378,027 individual handle releases. Their aggregate inclusive
timings are 181 and 128 seconds respectively; these can overlap and omit
the unfinished worker, so they are not job elapsed times or product savings.
[Counts, checkpoints and artifact digests](worker-native-direct-overhead.json)
retain the partial evidence.

The next experiment should inspect only the three watched names (`seed`,
`tmp`, `invalidate`) using captured intrinsics and actual native arguments,
while preserving proxy and getter protections, wrapper cleanup and complete
compilation accounting. The current all-accessor scan is doing substantial
work unrelated to the watched fields. A narrower inspection must explicitly
report its scope, preserve the same watched-field evidence, and pass narrow
and accessor-heavy regression measurements before another full run. This
does not authorize dropping any production input or replay check.

## Actual source evidence for watched descriptor callers

The prior 87-worker partial trace has 35,551 descriptor calls on objects
with watched own accessors. All their apparent immediate callers use
diagnostic filenames. This is an observation of reported names, not an
identity proof: code can impersonate a filename.

Commit `66d5b78` resolves the V8 script IDs of every retained stack frame
for those calls and stores the actual source digest in UTF-16, using the
existing source reader and cache. It makes no filename-based exemption.
[Fixture gate 42225425259](https://github.com/pango07/quod/actions/runs/42225425259)
passes 142 tests, including foreign VM code named as our scalar observer:
the report records the foreign code's hash and actual script ID. The full
driver requires source evidence for every relevant frame. This closes an
attribution gap in the diagnostic; other unobserved channels still prevent
a replay certificate. [Evidence](worker-native-caller-sources.json) records
the gate and its scope separately from the full workload.

## Scoped target inspection and remaining protocol cost

Commit `4d3a3e0` adds an optional private Inspector helper that uses captured
intrinsics to inspect only own accessors named `seed`, `tmp` and `invalidate`.
It returns a primitive mask without reading their values or exporting a
public bridge. Proxy targets remain outside descriptor inspection. Reports
declare whether they inspected all own accessors or only the watched fields;
a scoped report never claims an all-accessor count. Adaptive selection is a
performance hint keyed by call site. Both paths inspect the current target
again on every call, including after target replacement or property edits.

[Fixture gate 42227725928](https://github.com/pango07/quod/actions/runs/42227725928)
passes 154 tests. For 300 calls on a target with 2,248 own accessors, protocol
inspection takes 8,935 ms and adaptive inspection 264 ms. On a one-accessor
target, the times are 139 and 148 ms. A target with 81 accessors also favors
scoped inspection (280 versus 449 ms), so integration commit `084a381`
selects the scoped path after a site observes at least 64 accessors. This
threshold changes implementation choice, not which watched fields are checked.
The fixture keeps complete compilation logging, including the additional
condition evaluations triggered by the helper's captured descriptor calls.

[Full gate 42228027307](https://github.com/pango07/quod/actions/runs/42228027307)
passes 191 regressions, including target garbage collection while the helper
remains active, but again hits the 420-second `before` phase limit. It retains
93 completed workers, 872,179 descriptor calls and 47,389 proxy calls, with
all 919,568 wrappers cleared and no reported capture error or dropped record
in those completed reports. The unfinished stream worker reaches 620,000
descriptor calls without overflow. There is no complete observed phase or
metadata-only before/after comparison.

The same 93 file/project instances in the preceding partial run required
15,906,657 handle releases, versus 3,680,889 with adaptive inspection. Their
native call counts differ (917,260 versus 919,568), so this is a comparison
of command volume, not an elapsed-time speedup. The plain stream property
file itself took 101,415 ms on the latest runner, versus 68,139 ms previously.
Its pinned source defaults to fixed seeds and 24 cases; the diagnostic has
not changed those tests or seed settings. This variation prevents treating
93 versus 104 completed workers as an isolated measure of the code change.

An independent source check validates all 1,940 source records used by
watched-property stacks in the 93 completed workers. All 37,594 watched calls
have immediate caller source bytes matching the checked-in diagnostic at
the measured commit, with the actual V8 script ID, module/sourceURL flags
and synchronous stack shape checked. This is stronger than matching names;
it still does not establish initial runtime integrity, closure ownership,
unobserved native channels or coverage of the missing workers.

The next experiment is a pinned-Node native helper that validates the owned
exception wrapper, inspects actual arguments and clears the wrapper in one
private Inspector call. It must preserve side-effect, proxy, receiver,
foreign-exception and lifetime checks, and retain complete compilation
evidence. The existing JavaScript helper triggers additional traced calls
inside Inspector; a native implementation may avoid that cost. It must be
built and tested remotely before use. No production input keys or replay
eligibility have changed. [Samples, source verification and artifact
digests](worker-native-scoped-inspection.json) retain the exact evidence.

## Private native capture fixture

The pinned Node 24.18.0 addon at `28278be`, corrected at `f4f0e4d`, combines
wrapper validation, argument classification, watched own-accessor inspection
and wrapper cleanup in one private Inspector call. It uses V8 APIs without
calling application getters or proxy traps. Interceptor targets are rejected;
their owned wrappers are still cleared. No public global holds the capture
function, and active-helper garbage-collection regressions check that argument
objects and accessor closures are not retained.

The first two gates failed because a generated condition lacked its final
block brace. Inspector accepted the malformed condition, producing zero
captured calls. The fixture now parses every generated condition before
installing it. [Gate 42233436439](https://github.com/pango07/quod/actions/runs/42233436439)
passes all 168 regressions after the correction. For 300 calls with complete
compilation logging, the measured times are:

| Target own accessors | Protocol | Adaptive JavaScript | Native |
| --- | ---: | ---: | ---: |
| 1 | 160 ms | 151 ms | 129 ms |
| 81 | 406 ms | 285 ms | 120 ms |
| 2,248 | 8,905 ms | 271 ms | 96 ms |

Every native sample captures and clears all 300 wrappers, with 300 private
calls and 300 handle releases. It retains 614 compilation events; adaptive
wide inspection retains 2,106. No breakpoint suppression or source filtering
produces that difference. These are fixture measurements, not CI savings.

Commit `b74197c` selects the helper in the combined worker diagnostic, passes
its path explicitly through Vitest's worker environment, and requires the
native inspection mode in the driver. The full gate is tracked separately
from the fixture. The first integration gate caught its obsolete zero-pause
setup assertion: loading the addon correctly emits one observed `dlopen`
pause. Correction `99a9310` requires that exact setup event and no additional
pauses for the 19 descriptor operations; it does not hide the load.
Initial runtime integrity, unsupported native channels and
complete workload coverage still require evidence. [Exact measurements and
failed-gate history](worker-native-capture.json) remain diagnostic-only.

Source review then identified a native-callback gap in the first helper.
V8's descriptor implementation can fetch a native data property's value,
invoking its host callback. Correction `98de5e8` uses
`HasRealNamedCallbackProperty` instead: it checks own callback presence
without fetching the value, and conservatively includes native callbacks
and empty JavaScript accessors. Owned wrapper fields reject callbacks before
requesting their descriptors. API wrappers and interceptors are rejected
before target property inspection. The implementation follows the pinned
[V8 API entry](https://github.com/nodejs/node/blob/v24.18.0/deps/v8/src/api/api.cc)
and [object lookup](https://github.com/nodejs/node/blob/v24.18.0/deps/v8/src/objects/js-objects.cc).

[Gate 42235088537](https://github.com/pango07/quod/actions/runs/42235088537)
at `21c3531` passes 171 regressions. A native data callback on an ordinary
object remains uncalled during capture, then runs during ordinary descriptor
inspection. A foreign marker callback remains uncalled too. A separate API
wrapper fixture proves that capture rejects the object without invoking
either host callback, while an ordinary cross-context read exercises both
the access check and getter. The wide 300-call fixture measures 126 ms native
versus 358 ms adaptive with complete compilation logging. The earlier full
gate runs the pre-correction helper and cannot establish native-callback
exclusion; a complete run of the corrected helper is still required.

The pre-correction [full gate 42234007135](https://github.com/pango07/quod/actions/runs/42234007135)
at `99a9310` passes 205 regressions and the plain 120-instance/4,131-state
baseline, then reaches the 420-second `before` limit. It retains 119 worker
reports, with 1,255,379 captured native calls and matching wrapper cleanup
counts. The remaining stream-property worker reaches 1,030,000 descriptor
calls without log overflow. No complete observed test result or `after`
phase exists.

Unlike the preceding adaptive run, 116 completed reports contain capture
errors and are explicitly unavailable. Their retained calls cannot establish
complete coverage. Each stores the maximum 20 error samples; that cap does
not establish the total error count. All 48,117 retained watched-property
calls have immediate caller bytes matching the diagnostic at the measured
commit, across 2,476 verified source records, but the failed captures remain
unaccounted for. The next investigation isolates exception handling across
execution contexts before another full gate. The [evidence file](worker-native-capture.json)
records the timeout, unavailable reports, artifact digest and scoped checks.

A focused [foreign-context fixture](https://github.com/pango07/quod/actions/runs/42236331469)
reproduces a protocol failure when the private native function receives a
foreign-context Error handle. The same-context case succeeds. V8's
[argument resolver](https://github.com/nodejs/node/blob/v24.18.0/deps/v8/src/inspector/injected-script.cc)
rejects handles from another execution context. This explains the focused
failure, but a corrected full run must still establish whether it explains
the 116 unavailable reports.

Candidate `4d18bf2` excludes genuine native Error exceptions before inspecting
them. Their subtype comes from `IsNativeError()` in the pinned
[value mirror](https://github.com/nodejs/node/blob/v24.18.0/deps/v8/src/inspector/value-mirror.cc);
the pinned [Node Inspector client](https://github.com/nodejs/node/blob/v24.18.0/src/inspector_agent.cc)
does not override value subtypes. A fresh null-prototype capture literal
cannot be a native Error. The added forged-Error fixture keeps a foreign
plain object unknown even when its name and `Symbol.toStringTag` say Error.
The candidate does not enumerate foreign properties, execute their getters,
or silently accept an unknown capture.

[Gate 42237047476](https://github.com/pango07/quod/actions/runs/42237047476)
passes all 174 regressions at `4d18bf2`, including both native Error contexts,
the forged foreign object and native callback/access-check fixtures. Corrected
[full gate 42237280902](https://github.com/pango07/quod/actions/runs/42237280902)
ran that commit. It passed 210 regressions and the plain baseline, then
again timed out at 420 seconds in `before`. All 119 retained worker reports
are unavailable: native Error exclusion did not resolve the repeated protocol
failures, and stricter target checks introduce additional rejected captures.
The diagnostic captures and clears 1,255,776 wrappers but retains only
1,217,683 attributed native events, a gap of 38,093 rejected owned targets.
The unfinished stream worker reaches 1,030,000 descriptor calls. No complete
observed test result or `after` phase exists. The source check validates
48,306 retained watched calls across 2,476 source records, but cannot account
for the failed captures.

Candidate `1921fe2` preserves sanitized metadata and call sites for unavailable
owned captures, classifies their failure stage, and counts every failure
separately from bounded samples. Foreign protocol failures retain bounded
call stacks and context IDs. These additions expose the remaining cases;
they do not make unknown captures available or permit reuse.
[Gate 42240010882](https://github.com/pango07/quod/actions/runs/42240010882)
passes 179 regressions. It also reproduces a foreign-intrinsic condition
that fails protocol transport, confirming that a foreign capture failure
cannot generally be dismissed as an unrelated application exception.

## Exact script metadata index

The observer previously kept a separate metadata object for every parsed
script ID. Millions of generated conditions share the same metadata. The
index at `d82ce60` interns the complete metadata tuple, uses bounded typed
array chunks for canonical numeric IDs, and keeps every other ID as an exact
string key. It preserves later updates, undefined versus empty URLs, language
and module/sourceURL flags, and UTF-16 strings. Capacity exhaustion makes the
observation unavailable. This changes the lookup table only: no compilation
event, code digest, order or occurrence count is removed.

[Integration gate 42238375600](https://github.com/pango07/quod/actions/runs/42238375600)
at `acf478a` passes 178 regressions. For 250,000 entries, isolated heap plus
ArrayBuffer growth falls from 43.46 MB to 1.18 MB. Every lookup is independently
checked. Insertion takes 183 ms versus 112 ms for the ordinary Map, while
lookup takes 32 ms versus 144 ms. These are synthetic samples, not process RSS
or CI savings. Native capture's actual source-evidence checks still pass,
and the driver requires index event counts to agree with all parsed-script
events. Progress checkpoints will record memory use in the next full run.
[Both fixture samples and invariants](worker-script-metadata-index.json) are
retained separately from full-workload evidence.

[Full gate 42240251280](https://github.com/pango07/quod/actions/runs/42240251280)
at `1921fe2` passed 215 regressions and the 120-file / 4,131-state baseline,
then timed out at 420 seconds in `before`. Its 119 retained worker reports
capture, attribute and clear all 1,252,907 owned native calls. They remain
unavailable: 38,093 targets are rejected as API wrappers, and 16,704 events
fail cross-context protocol transport. The latter's bounded stacks point
to Edge Runtime setup in context 2. Most rejected target calls come from
Vitest's global-environment setup. These call sites are investigation leads,
not ownership exemptions.

The unfinished stream property test reaches 1,880,000 descriptor calls and
3,761,885 compilation events, versus 1,030,000 descriptor calls previously.
Its checkpoint RSS is 2,557,988,864 bytes. There is still no complete observed
phase, so this progress is not a workload-speed result. One retained worker
also records 20 capped script-index errors; their cause remains unresolved.
Index event totals alone cannot establish successful indexing. The source
check validates all 48,278 retained watched calls against actual checked-in
diagnostic bytes across 2,476 source records, without certifying failed calls.

## Private native capture across execution contexts

Candidate `5350601` creates a private holder in an affected Inspector context,
finds exactly one holder by a fresh random own property name, then creates the
native capture function in that holder's creation context. The bounded heap
predicate checks property presence only; it reads no values or descriptors.
Full descriptor validation and function allocation occur after heap iteration.
No public global bridge or persistent application reference is installed.
Missing, duplicate, malformed or inaccessible holders remain unavailable.
Each context is attempted once, with a 32-context cap; close releases the
private Inspector object group.

[Gate 42241986122](https://github.com/pango07/quod/actions/runs/42241986122)
passes 183 regressions. Six foreign-native calls are captured and cleared
with one context query. Getter-backed holders, duplicate holders, proxy
traps, realm identity, wrapper cleanup and unchanged global keys are covered.
Foreign Error-like plain objects receive native inspection; their labels do
not grant an exemption. Native API-wrapper targets still fail closed.

The pinned V8 [heap query implementation](https://github.com/nodejs/node/blob/v24.18.0/deps/v8/src/profiler/heap-profiler.cc)
collects garbage and clears feedback vectors. Its full-workload cost remains
unmeasured. Resolve the rejected targets and script-index errors before the
next full boundary gate. This remains diagnostic work with replay eligibility
false; no production key or savings claim changes.

## Guarded owners and dense run storage

The index correction at `9f886ac` passes
[187 regressions](https://github.com/pango07/quod/actions/runs/42243803259).
The new regression deliberately re-enters insertion while freezing metadata.
The earlier order reserved a dictionary slot before freezing; a nested
insertion could reserve that same slot. The correction completes descriptor
validation and freezing before committing, then rechecks membership and
capacity. Captured intrinsics and length-prefixed UTF-16 keys avoid later
application replacements and inherited `toJSON` hooks. Failure stages are
counted and required to be zero. The earlier full-run errors remain
unexplained until a new workload measurement; this fix is not that evidence.

Candidate `b3006e8` requires a native ordinary-object guard before installing
watched scalar accessors on seed or module metadata. Rejected owners create
explicit gaps. Within this construction rule, API-wrapper/interceptor targets
cannot be those watched owners: capture retains the call as
`excluded-owner-type`, without inspecting host properties or inventing an
accessor count. Unrestricted capture keeps its previous rejection behavior.
The full-workload verifier requires the native owner policy and passing
guards on every watched scalar. This does not certify initial runtime
integrity or arbitrary native capabilities, and still authorizes no replay.
[All 190 regressions pass](https://github.com/pango07/quod/actions/runs/42244547700),
including host access-check exclusion, ordinary owners, strict-mode rejection,
and scope preservation through the private foreign-context bridge.

At `941d2d3`, the compilation log stores its exact ordered runs in dense
Float64Array chunks. Every safe-integer ID and occurrence count remains exact;
reports retain the existing `dictionary-v1` JSON format. No script, count,
ordering, field, or capacity is removed. The
[192-regression gate](https://github.com/pango07/quod/actions/runs/42245067969)
passes. For 250,000 alternating runs, isolated heap plus ArrayBuffer growth
falls from 18.07 MB to 4.10 MB, with every serialized entry checked. This
measures run storage before report materialization, not process RSS or CI
savings. [Measurement details](worker-dense-compilation-runs.json) retain that
distinction and the chunk-boundary and snapshot-mutation regressions.

[Full diagnostic 42245249422](https://github.com/pango07/quod/actions/runs/42245249422)
at `941d2d3` passed 228 regressions and the 120-file / 4,131-state baseline,
then timed out at 420 seconds in `before`. All 98 retained worker reports
pass the scoped observation checks: 1,047,507 owned native calls are
attributed and cleared, with zero capture failures or index failures.
The 95 foreign-context bridges each require one query. All 3,268 watched
metadata scalars and 98 seeds pass the native owner guard; 32,072 API-wrapper
calls remain explicitly classified as excluded owner types. The formerly
failing zod-4.2 worker completes with zero index errors. Source verification
checks all 40,468 retained watched calls across 2,042 source records.

The unfinished stream worker reaches 700,000 descriptor calls, with
1,401,387 compilation events and 1,011,503,104 bytes RSS. Its run index uses
22,478,848 bytes. The plain baseline took 101.42 seconds versus 48.26 in the
preceding full attempt, so worker progress alone cannot isolate performance
effects. Twenty-two required worker reports and the complete observed
test-state result are missing. This is not full coverage or a wrong=0
comparison for the observed phase.

## Bounded retired-script source cache

The pinned V8 [Debugger implementation](https://github.com/nodejs/node/blob/v24.18.0/deps/v8/src/inspector/v8-debugger-agent-impl.cc)
defaults `maxScriptsCacheSize` to an effectively unlimited value, retaining
retired script source text outside the JavaScript lookup tables. Candidate
`32256db` sets that cache to zero for the combined native compilation
diagnostic. Every compilation is still recorded during `scriptParsed`.
Watched caller sources are now captured synchronously at the native entry,
while their frames are live, then retained as exact source hashes and
metadata. Missing source makes observation unavailable; deferred reports
do not depend on retired Inspector source text.

[Gate 42246820101](https://github.com/pango07/quod/actions/runs/42246820101)
passes 194 regressions. A watched temporary function is garbage-collected,
then its source evidence is verified with all late source requests forced
to fail; no late request occurs. In an isolated 25,000-compilation check,
every source matches under both cache settings. RSS growth is 112.06 MB
with the default cache and 88.32 MB with zero retired-source caching.
These are synthetic memory samples, not full-workload or CI savings.

[Full diagnostic 42247258542](https://github.com/pango07/quod/actions/runs/42247258542)
at `32256db` passed 230 regressions and the 120-file / 4,131-state baseline,
then timed out at 420 seconds in `before`. All 103 retained worker reports
pass the scoped checks: 1,103,506 native calls attributed, cleared and
released; 3,428 guarded metadata scalars; 103 guarded seeds; zero native or
index failures. All 42,265 retained watched calls match the actual checked-in
caller sources. Seventeen worker reports and the complete observed test-state
result are missing, so this is not a full before/after or wrong=0 result.

The stream worker reaches 740,000 descriptor calls and 1,481,406 compilation
events at 435,458,048 bytes RSS, versus 700,000 calls at 1,011,503,104 bytes in
the previous attempt. Plain baseline spans are 98.56 and 101.42 seconds.
Memory retention improves, but complete workload time remains unmeasured.
The retained workers still issue 2,263,648 source-fetch commands, taking
67.23 seconds inclusively across workers. These timings overlap other
Inspector commands and cannot be added to wall time.

## Exact source reuse for repeated diagnostic compilations

Candidate `68308eb` avoids fetching identical diagnostic source on every
compilation, while preserving every ordered event. Native hashes alone are
unsafe: the pinned V8 [script hash](https://github.com/nodejs/node/blob/v24.18.0/deps/v8/src/objects/objects.cc)
uses [String::ToCString](https://github.com/nodejs/node/blob/v24.18.0/deps/v8/src/objects/string.cc),
which replaces NUL with space. The cache admits a source only after fetching
its complete text, checking its length and native hash, and establishing
that every code unit is ASCII, excluding NUL and space. Within that subset,
the conversion has a unique preimage, subject to the existing SHA-256
collision assumption. The retained digest remains SHA-256 of UTF-16LE.

All other sources, live edits and WebAssembly keep full source reads.
Capacity exhaustion also falls back to full reads. Metadata, failed-parse
state, occurrence counts and order remain independently recorded. The
diagnostic's condition and private invocation strings use equivalent
space-free syntax so their repeated compilations can qualify.

The first fixture gate passed the exact-record and actual native-hash
collision regressions, then rejected the measurement because a second
Inspector session rediscovered scripts from the first measurement.
Correction `dfccfbe` gives the two measurements distinct source identities
and requires zero initial events. The corrected
[gate 42250490937](https://github.com/pango07/quod/actions/runs/42250490937)
passes 199 regressions. Both isolated measurements retain all 20,000 exact
compilation records. Full source reads fall from 20,000 to two; measured
capture time falls from 1,230.39 ms to 426.28 ms. These are diagnostic
capture timings, not full-workload or CI savings.

`3db47e4` also rechecks cache membership and capacity after source acquisition
so a nested script event cannot double-count an admitted source. A regression
exercises that path. The
[full gate 42250940190](https://github.com/pango07/quod/actions/runs/42250940190)
passed 236 regressions and the complete baseline, then timed out in `before`
at 420 seconds. All 115 retained workers pass scoped checks: 1,240,967 native
calls are attributed, cleared and released; zero capture or index failures;
3,776 guarded metadata scalars and 115 guarded seeds. All 47,464 retained
watched calls have matching actual source evidence.

The retained compilation logs preserve 2,523,643 events with 2,463,733 cache
hits and 59,910 full-source reads. Including separate caller-source lookups,
63,569 source-fetch commands took 13.16 seconds inclusively across workers.
The unfinished stream worker reaches 880,000 descriptor calls and 1,761,382
compilation events at 442,912,768 bytes RSS. The plain baseline span is
92.12 seconds. Five worker reports and all complete observed test-state
comparisons remain missing; this is not a full wrong=0 or CI savings result.

`567e91e` extends only the combined diagnostic's observed-phase collection
limit to 900 seconds, with a 35-minute workflow cap. The original Vitest
test timeouts, test scope and product targets remain unchanged. Phase
duration/status and source-cache checkpoint counts are now saved explicitly.
The [extended full gate 42253196341](https://github.com/pango07/quod/actions/runs/42253196341)
passed 236 regressions and the complete 120-file / 4,131-state baseline,
then timed out after 900 seconds in `before`. All 119 retained worker
reports pass scoped checks: 1,269,076 native calls cleared and released,
zero capture/index failures, 3,892 guarded metadata scalars and 119 guarded
seeds. All 48,292 retained watched calls match actual checked-in source.

The unfinished stream worker reaches 5,450,000 descriptor calls and
10,902,958 compilation events. Its ordered compilation log reaches the
4,000,000-run limit and reports overflow; complete coverage cannot be
claimed. Its checkpoint RSS is 554,151,936 bytes. The baseline phase took
48.72 seconds; observed collection reached 900.08 seconds. No complete
observed test-state result or after comparison exists. Extending collection
alone did not complete this diagnostic, and the log-capacity issue must be
resolved before a complete comparison can pass. No production replay
authority or savings claim changes.
[Machine-readable evidence](worker-compiled-source-cache.json) retains the
cache restrictions, measured counts and gate scope.

## Native payload data lookup

Separately, `5750e67` removes descriptor-object
allocation from owned native payload/holder validation. The helper first
requires an ordinary object with a null prototype and no host wrapper or
interceptor, then rejects callback properties before checking attributes and
reading data. The pinned [V8 lookup implementation](https://github.com/nodejs/node/blob/v24.18.0/deps/v8/src/api/api.cc)
can traverse prototypes and invoke accessors in general; the explicit owner,
null-prototype and callback checks are required before using it here.

[Gate 42255029030](https://github.com/pango07/quod/actions/runs/42255029030)
passes 202 regressions. A separately compiled descriptor baseline produces
the same capture summaries for 29 wrapper shapes, including missing and
read-only fields, getters, empty accessors, frozen wrappers, inherited fields
and proxies. No getter or proxy trap runs. Existing native callback,
host access-check, foreign-context and lifetime cases also pass.

Four isolated 100,000-call measurements retain 100,000 matching summaries
each. Descriptor validation takes 721.42 / 567.43 ms; direct data validation
takes 409.21 / 532.67 ms. These measure the native helper alone, excluding
Inspector transport and full-workload time. The extended full run above used
the earlier helper. No production savings or replay authority changes.


## Lossless compression beyond the observed log capacity

`4b584f2` stores repeated blocks of compilation records once, retaining an
ordered block-run index and the exact unfinished tail. The four-million-pair
limit now bounds stored data; repeated occurrences do not consume an entry
each. Physical capacity exhaustion still makes the report incomplete.
Every serialized record field, count and sequence position remains present.
[Gate 42257945410](https://github.com/pango07/quod/actions/runs/42257945410)
passes 210 regressions, including reconstruction of 12 million alternating
compilation events, interruptions in repeated patterns, corrupt references,
count overflow and sequence reorderings with equal totals.

`199e978` also stores consecutive numeric script IDs with identical metadata
as exact intervals. Gaps, noncanonical string IDs and overwritten metadata
remain distinct. The index bounds intervals and sparse entries at 65,536
each. Captured native operations and typed temporary storage keep inherited
array setters out of the commit path. Existing nested insertion, immutable
metadata and changed-intrinsic regressions run against both storage modes.
[Combined gate 42258901740](https://github.com/pango07/quod/actions/runs/42258901740)
passes 224 regressions. All 12 million ID lookups match expected metadata;
20,000 irregular range updates also agree with an independent Map.

In that gate, the 12-million-event fixture serializes to 2,249 bytes and
uses 513 stored pairs; appending takes 2.00 seconds. The 12-million-ID fixture
uses one interval and 98,304 bytes of allocated range storage; insertion
takes 6.63 seconds. These isolated storage measurements do not establish
full-workload speed or CI savings.

The [full diagnostic 42259109908](https://github.com/pango07/quod/actions/runs/42259109908)
at `199e978`, including the earlier `5750e67` native data-field helper,
passes 260 regressions and the 120-file / 4,131-state baseline, then times
out at 900 seconds in `before`. All 119 retained worker reports pass the
scoped checks: 1,271,348 native calls cleared and released, 3,876 guarded
metadata scalars and 119 seeds; zero capture, index or log failures. All
48,236 retained watched calls match actual checked-in caller source.

The unfinished stream worker reaches 3,040,000 descriptor calls and
6,082,144 compilation events without overflow. It uses 96,750 stored
compilation pairs and 1,523 script metadata intervals, with 447,094,784
bytes RSS. Compression fixes the previous capacity failure but does not
complete collection. The baseline takes 68.38 seconds. Complete observed
test states and the after phase remain missing, so there is no full
wrong=0 comparison or new CI savings result. No production replay is authorized.
[Machine-readable evidence](worker-compressed-diagnostic-storage.json)
records the gates, storage limits, measurements and remaining proof gaps.

## Compact native summaries and repeated caller lookups

`4299ddd` encodes the native capture result in four fields while retaining
every ownership, validity, cleanup, kind, mask, failure, rejection and owner
classification. The initial gate caught array destructuring of the native
null-prototype array. `1e51c85` reads owned slots directly and tests that an
iterator is never invoked. The corrected
[gate 42260710701](https://github.com/pango07/quod/actions/runs/42260710701)
passes 229 regressions. Four isolated 5,000-call comparisons preserve equal
capture and cleanup results plus 10,013 exact compilation events each.
Inspector response bytes fall from 1,455,000 to 405,000; object-format
measurements take 1,710.67 / 1,748.35 ms versus 1,534.47 / 1,521.95 ms packed.
These are transport measurements, not workload or CI savings.
[Evidence](worker-native-packed-summary.json) includes the failed first gate.

`6c7d5df` caches at most 2,048 successful caller-metadata lookups. Only
public reads fill it, so millions of compilation insertions cannot displace
frequently inspected caller IDs. Cached records are immutable owned values;
successful overwrites update the cache after commit. Failed updates keep
the earlier value, and nested validation activity rechecks the index state.
[Gate 42262070111](https://github.com/pango07/quod/actions/runs/42262070111)
passes 237 regressions. Four isolated two-million-lookup measurements check
all metadata fields while inserting new script IDs. Uncached times are
525.41 / 531.45 ms; cached times are 65.74 / 80.23 ms, retaining 128 entries.
[Evidence](worker-hot-caller-metadata.json) records the exact counts and
correctness cases. Full-workload time remains unmeasured for these changes.

`e9255f0` also reuses immutable mapped native frames instead of repeatedly
serializing identical caller fields. Only private, registered script metadata
can enter this cache. Unknown/mutable metadata and cache exhaustion keep
fresh mapping; metadata replacements, missing IDs, exact UTF-16 fields and
frame order remain significant. Cached frames and locations are frozen,
owned serialization reads only checked primitives, and cache capacity is
rechecked after freezing in case of nested Inspector activity.

[Gate 42263280155](https://github.com/pango07/quod/actions/runs/42263280155)
passes 243 regressions. Four 100,000-call measurements with 24 frames per
call produce byte-identical 302,475-byte reports containing all 100,000
occurrences, 1,536 frames and 64 distinct entries. Fresh mapping/logging
takes 2,618.42 / 2,684.75 ms; cached mapping takes 1,254.62 / 1,377.50 ms.
The cache retains 1,536 frames and 24 metadata identities, below its 20,000
limit. These are isolated mapping/logging timings, not new product gains.
[Evidence](worker-native-frame-cache.json) records the complete comparison.

The combined [full diagnostic 42263493464](https://github.com/pango07/quod/actions/runs/42263493464)
at `e9255f0` passes 279 regressions and the complete 120-file / 4,131-state
baseline, then times out at 900 seconds in `before`. All 119 retained
workers pass scoped checks: 1,272,645 native calls cleared/released,
3,858 guarded metadata scalars, 119 guarded seeds and no capture/index/log
failures. All 48,173 retained watched calls match checked-in caller sources.

The unfinished stream worker reaches 2,370,000 descriptor calls and
4,741,945 compilation events without overflow. It retains 117 hot metadata
lookups and 2,639 cached frames, with 434,249,728 bytes RSS. The baseline
takes 99.50 seconds; complete observed states and the after phase are missing.
This does not establish a workload speedup or a full wrong=0 result. Scope,
native coverage, collection limits and original Vitest timeouts are unchanged.
Detailed [full-run evidence](worker-native-frame-cache.json) records the
partial verification and artifact hashes. Production replay authority and
savings claims remain unchanged.

## Incremental test progress and owned compilation-record keys

`5b32fc4` adds parent-side ready/result events using the documented Vitest
[reporter lifecycle](https://vitest.dev/api/advanced/reporters). Each event
is written immediately, so a phase timeout retains completed test identities
and the last reported starts. The complete JSON test report remains the
state-comparison authority. Write failures, unknown callback shapes and the
40,000-event limit explicitly make progress evidence incomplete.

[Gate 42265335788](https://github.com/pango07/quod/actions/runs/42265335788)
passes seven regressions and both complete plain runs around 39 metadata-only
edits. Each run retains 8,263 contiguous events: 4,131 ready, 4,131 result
and one run-end. Independent verification matches task identities, event
order, monotonic receipt times and 4,130 passed / one skipped state across
120 file/project instances. Every case has exactly one ready and one result.
The complete per-test state lists agree before and after. This validates
reporting, not replay ownership or performance savings.
[Evidence](worker-test-progress.json) includes all artifact file hashes.

`b719412` reuses exact serialized keys for compilation records already owned
and deeply frozen by the source recorder. All identity/code/language/kind
fields remain present. Mutable or unrelated records keep full serialization,
including their later field changes. Independently freezing a record cannot
grant ownership; private lookup never invokes inherited serialization hooks.

[Gate 42265857321](https://github.com/pango07/quod/actions/runs/42265857321)
passes 249 regressions. Four two-million-event measurements produce identical
2,239-byte reports and exact ordered counts. Full serialization/logging takes
3,979.73 / 3,680.49 ms; owned-key reuse takes 199.85 / 191.69 ms. These isolated
times do not establish workload or CI savings. Neither this change nor the
progress reporter is included in the `e9255f0` full diagnostic above.
[Evidence](worker-compiled-record-keys.json) records the complete comparisons.

`aaed633` adds inclusive command timings and native exception-callback
events/completions/time to the existing descriptor checkpoints. The in-flight
callback is explicit, and each snapshot owns its counter values. These
durations overlap and cannot be summed as job elapsed time.
[Gate 42266816888](https://github.com/pango07/quod/actions/runs/42266816888)
passes 249 regressions, including all 19 descriptor operations, settled
callback counts and per-checkpoint in-flight/release accounting.

The [full diagnostic 42267064963](https://github.com/pango07/quod/actions/runs/42267064963)
at `aaed633` timed out after 900 seconds. Its incremental journal retains
two test-state differences and 23 missing baseline states; the planned
39-file metadata mutation was not reached. See the
[complete retained-evidence analysis](worker-native-progress.md). This is
not a passing whole-suite comparison or a product savings measurement.

## Primitive native-summary transport

The private native Inspector experiment could compile but could not start:
the pinned Node binary does not export `v8::debug::GetInspector`. The
[recorded failure and symbol inventory](worker-native-inspector-experiment.json)
rule out that approach on the current runner.

`c91d123` instead encodes the existing validated capture summary as a
primitive string, using V8's JSON serializer over a null-prototype array
of validated primitives. The payload is cleared before serialization.
[Gate 42274440392](https://github.com/pango07/quod/actions/runs/42274440392)
passes 251 regressions, including poisoned JavaScript serialization hooks,
foreign contexts and exact equivalence with object and packed summaries.

`b996649` calls this helper inside the native breakpoint condition through
a realm-local lexical binding. Only the sanitized string reaches Inspector.
[Gate 42275119640](https://github.com/pango07/quod/actions/runs/42275119640)
passes seven tests. Its protocol/string/string/protocol 5,000-call comparison
takes 571.20 / 262.71 / 268.06 / 635.15 ms. Each run retains all observations,
cleanup confirmations and bounded stacks. The string loops issue zero
Inspector commands, compared with 5,000 captures plus 5,000 releases for
the protocol loops; compilation events fall from 10,000 to 5,000.

These are isolated transport timings, not workload or CI savings. Global
object property names remain unchanged, but the new lexical bindings are
observable through debugger capabilities and persist until realm destruction.
Foreign realms still require a one-time native heap-query bridge. Neither
transport proves initial ownership, complete observer coverage or replay
eligibility. [Machine-readable evidence](worker-native-string-transport.json)
records the measurements. The optional integrated observer at `2d1b1ed`
passes [49 transport/integration checks](https://github.com/pango07/quod/actions/runs/42276529577)
and [251 existing regressions](https://github.com/pango07/quod/actions/runs/42276533939).
All 19 watched descriptor operations retain their actual targets and caller
sources. The helper's own native load remains observed; unknown native
owners and failed context installation remain explicit failures. A garbage
collection check confirms that primitive exceptions do not retain their
application argument object. The repeated isolated timing comparison is
1,330.89 / 614.18 / 613.53 / 1,322.78 ms, again with every observation retained.

The full worker-boundary gate can now select `native_transport=string`.
The default stays on the existing protocol transport. Both retain the
120-file / 4,131-state workload, original test timeouts, seeds and 39-file
metadata mutation. The phase artifact identifies the chosen transport;
primitive captures must account for every summary without claiming object
handle releases. Whole-suite agreement and runtime remain unmeasured for
the integrated string transport.

The full diagnostic [42276956134](https://github.com/pango07/quod/actions/runs/42276956134),
at `0c65084` with `native_transport=string`, failed. All 333 preceding
regressions passed. The observed phase completes within its collection
limit, but six tests exceed their unchanged 30-second timeouts.

| Measurement | Plain baseline | Observed before phase |
| --- | ---: | ---: |
| Elapsed seconds | 46.691 | 538.419 |
| Retained file/project instances | 120 | 120 |
| Retained test states | 4,131 | 4,131 |
| Passed / skipped / failed | 4,130 / 1 / 0 | 4,124 / 1 / 6 |
| Test-state differences from baseline | — | 6 |

All six failures are the composed-stream adversarial-cursor tests, both
directions for seeds 1, 12648430 and 3735928559. Their actual JSON failure
messages identify test timeouts; this is no longer inferred from receipt
timings. No replay occurred. The driver stops before the 39-file metadata
mutation and after phase, so the full comparison remains incomplete.

All 120 retained worker reports pass the scoped boundary checks. They
retain 7,099,799 validated/cleared primitive captures, 7,155,077 compilation
events and zero per-entry object-handle releases or transport failures.
All 48,670 watched-accessor calls match checked-in diagnostic caller source
after independently verifying the new seven-frame transport prefix.
The reporter journal independently matches every complete JSON test state.
These checks do not establish initial input ownership or complete coverage.
[Detailed evidence](worker-native-string-full.json) records all 171 artifact
digests, failure identities and overlapping command/callback timings.

The run also exposes a regression in diagnostic source caching: the space
after `throw` makes repeated conditions ineligible for the existing exact-
source cache. It performs 7,153,171 complete-source reads. `ae49c7f` uses a
parenthesized throw expression to remove that space without weakening any
cache rule. [Gate 42279318158](https://github.com/pango07/quod/actions/runs/42279318158)
passes 50 tests, including 1,000 captures with one source fetch and 999 exact
cache hits. This fix has no new full-workload measurement. Global native
observation remains too expensive; no product savings claim follows.

## Native data-property candidate

[Gate 42277546954](https://github.com/pango07/quod/actions/runs/42277546954)
at `ce3fb40` inventories 44 cases using the existing native-data fixture:
23 reads/shape queries and 21 mutations. Every observed result, error type
and final descriptor/shape matches the corresponding ordinary data property.
The 19 value-reading cases each invoke the native callback once, including
descriptor reflection, cloning, JSON, V8 serialization, `util.inspect`,
inherited/proxy receivers and a foreign realm. Getter/setter lookup, keys
and presence checks do not invoke it or expose a JavaScript getter.

This may allow direct scalar-read observation without intercepting unrelated
descriptor operations globally. It is not yet an application observer or
a replay proof. Ordinary writes and partial property redefinitions can
remove the callback. A future observer must detect that loss and reject
incomplete evidence; it cannot assume tracking survives a successful write.
The fixture covers one string field on a null-prototype object. General
values/attributes, replacement/restoration, native callback identity, earlier
copies, error/microtask isolation and lifetime still need implementation
and checks. [Machine-readable evidence](worker-native-data-property.json)
retains every comparison and callback count. No performance measurement or
product change is attached to this candidate.

`079526e` prototypes a native scalar-read observer on existing writable,
configurable own data fields. It preserves the original scalar and flags,
keeps its state in collectible V8 objects, and checks that its own native
callback still supplies the field. Replacing the field with identical data
does not restore tracking. Checks do not call replacement JavaScript
getters. Sink errors/reentrancy mark observation failed while preserving
the application's value; queued microtasks do not advance at the new
native-to-JavaScript boundary.

The first [gate 42278886610](https://github.com/pango07/quod/actions/runs/42278886610)
compiled both helper variants and passed eight of ten tests. Two fixture
assumptions were wrong: its default parameter changed an explicit undefined
test value to 17, and redefining a native property itself reads the prior
value before the later integrity check. `ae49c7f` corrects those baselines.
[Gate 42279314082](https://github.com/pango07/quod/actions/runs/42279314082)
passes all nine observer regressions plus the 44-case inventory, including
56 scalar/value/descriptor/prototype combinations, foreign realms,
replacement/restoration, callback failure and garbage collection.
[The shared-helper gate](https://github.com/pango07/quod/actions/runs/42279322159)
also passes all 251 existing regressions.

Closing a frozen owner cannot remove its nonconfigurable callback. That
case is reported explicitly: the closed callback returns the scalar but
releases the owner and JavaScript sink. Inspector/native/heap capabilities,
initial ownership and values copied earlier remain unproven. This observer
is not integrated into the workload and grants no replay authority. The
next step is a separately gated worker diagnostic using its direct read
evidence, with callback loss rejecting incomplete observations and the
same complete test scope and original timeouts.

## Integrated direct scalar diagnostic

`52a201e` integrates native scalar reads into the worker diagnostic behind
`scalar_observation=native-data`. It retains metadata identity and snapshots
without counting private integrity checks as application reads. Every native
read must reconcile with a recorded event and the exact source of each
retained caller frame. Missing source, callback loss or added unobserved
fields make the evidence unavailable. This mode explicitly disables the
global descriptor census and retains the proxy, hook, opaque capability,
Inspector, transport and compilation checks. It does not enable replay.

The combined regression exposed a startup SIGSEGV before scalar installation.
[The trace](https://github.com/pango07/quod/actions/runs/42284696816) ends at
`Runtime.evaluate` for an old foreign context after heap queries; GDB shows
`Context::Enter` inside Inspector's `InjectedScript::Scope::initialize`.
This matches the failure described in
[Node issue 67138](https://github.com/nodejs/node/issues/67138): collection can
empty a context before the asynchronous destruction notification arrives.

`e3b30de` guards foreign-context setup with V8's public context-ID API and a
heap lookup that retains at most one matching object. Its native handle
scope keeps the context alive throughout the synchronous callback, including
nested collection. Collected contexts are counted as retired and never sent
an evaluation; active foreign contexts still receive the helper. Destruction
notifications remove retired contexts, and a registry-clear event remains an
explicit observation failure. No context or application object is retained
after the callback. The first attempt used an unavailable private Inspector
accessor and failed its gate; the final guard uses the public exported API.

The corrected integration passes 22 focused tests, 54 transport/descriptor
tests and 251 existing shared regressions. The tests include collection before
setup, collection inside the guarded callback, subsequent release, errors,
and a live realm after a retired one. [Detailed gate evidence](worker-native-scalar-integration.json)
records commits and run URLs. The complete workload is dispatched at the
same code revision, with 120 file/project instances, 4,131 test states,
39 metadata-only edits and unchanged test timeouts.

[Full run 42285754294](https://github.com/pango07/quod/actions/runs/42285754294)
passes all 351 preflight tests and all three workload phases. The plain
baseline takes 68.028 seconds; direct scalar observation takes 93.532 seconds
before the metadata edit and 92.078 seconds afterward (1.375x and 1.354x the
baseline). Each phase has 4,130 passed tests and one skipped test, with the
same semantic fingerprint. There are no test timeouts. The earlier diagnostic
used a different observer, lacked the exact-source cache fix, and had a
46.691-second baseline, so its 538.419-second observed phase is not a paired
estimate of product savings.

Each observed phase reconciles all 2,581 native scalar reads with recorded
events and caller source evidence. The 120 workers report no scalar gaps or
transport failures. Proxy captures total 50,016 before and 49,914 afterward;
the global descriptor census is explicitly disabled. Evaluator code sequences
match in all 120 workers. Full compilation sequences and RPC observations
still differ in every worker, including a changed configuration seed.

An exploratory compilation breakdown preserves exact records, multiplicity
and order within each kind. File, builtin and named sequences match in all
120 workers; WebAssembly sequences match in all 117 workers that have them.
Anonymous scripts differ in all 120. This classification does not certify
interleaving, anonymous script ownership or complete inputs, and it changes
no replay predicate. The next work is to identify those differences without
silently discarding them. [Full evidence](worker-native-scalar-full.json)
includes all 261 artifact digests and phase timings. There are no product
hits, lineage promotions or audits in this diagnostic; the product savings
bar remains unmet.
