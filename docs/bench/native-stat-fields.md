# Native filesystem metadata fields: bounded feasibility

[Gate 42356300909](https://github.com/pango07/quod/actions/runs/42356300909)
passed 38 tests at `1519b8eb1ab796cef9f5bb4803017e9f1da55990`, using Node
24.18.0 and the existing remotely compiled native helper. Six new tests
exercise actual Node `Stats` and `BigIntStats`; the other tests cover the
helper's existing semantics, lifetimes and integration. No production
recorder policy changed, and no replay was enabled.

The new comparisons cover 11 consumers for each Stats representation.
Each observed value, prototype and final own descriptor agrees with the
ordinary object. This extends the earlier plain-object inventory to the
filesystem object at issue in convex-helpers' invalidations.

| Consumer | Observed fields |
| --- | --- |
| isFile | mode |
| size | size |
| mtimeMs, mtime Date, Object/Reflect descriptor value | mtimeMs |
| inode | ino |
| spread, inspect, structured clone, V8 serialization | All own metadata fields |

Normal Stats have 14 observed own fields, BigIntStats 18. A content-preserving
60-second timestamp mutation retains the same type and size while changing
the observed timestamp. Callback replacement is detected and rejects the
observation. Async stat retains the ordinary Stats prototype and supports
field observation after fulfillment.

The final case is deliberately disqualifying for late installation: an
earlier promise consumer copies a timestamp, then the observer is installed
and a later consumer reads only size. Its callbacks remain intact and report
only size, although application code already has the timestamp. Callback
integrity therefore cannot certify complete observation. This case is a
controlled ordering counterexample, not a demonstration of a bypass in a
proposed production factory.

The result justifies investigating observation **before a native Stats result
is published**, with provenance and constructor checks. It does not justify
wrapping a fulfilled promise and immediately using the narrower field set
as a cache key. The existing Inspector, native capability, constructor-setter
and earlier-alias counterexamples still apply; do not restart the worker IPC
masking approach or infer completeness from matching outputs.

A bounded next check is whether a pinned native constructor's return point
can install observation before synchronous callers and promise consumers
receive the object, while rejecting inherited constructor setters and
preserving microtask order. That is a mechanism experiment, not permission
to use it for replay. Startup integrity, unsupported observers, lifetime,
query equivalence and actual benchmark overhead remain required before any
production key refinement. If these cannot be established economically,
discard this candidate rather than growing another unbounded observer.

The [machine-readable evidence](native-stat-fields.json) contains the 22
consumer reports, the metadata mutation and earlier-consumer result. These
were parsed from the completed run log and independently checked. The log
SHA-256 is
`86d9164c00009a636c7867b447ccf5b3745e3e566633e10d874ef08a593766b9`.

## Constructor follow-up

The pre-publication probe has not passed. Run
[42356669903](https://github.com/pango07/quod/actions/runs/42356669903) at
`b0d5a9946dca85073046059b3ef87c52f0c11fc8` and run
[42356756999](https://github.com/pango07/quod/actions/runs/42356756999) at
`a76caf9da73aab6907d60e797899f0def2a61f65` each passed the existing 38 tests
and failed all five new constructor tests during Inspector setup.

The second run preserves the exact `Debugger.getPossibleBreakpoints`
error: `Inspector error -32000: Cannot retrive script context`. Narrowing
the query to the constructor body did not resolve it. The earlier diagnosis
of a declaration-boundary problem was therefore insufficient. The five
tests did not reach constructor observation, promise ordering, inherited
setter rejection or microtask-order assertions. Those behaviors remain
unverified, and no production input key was refined.

These failures establish a limitation of the attempted Inspector path,
not an impossibility result for every native mechanism. Further work must
first meet the [architecture review's execution-contract gate](../first-principles-replay.md),
rather than treating another debugger workaround as evidence of progress
toward useful replay.

## Constructor entry follow-up, 2026-11-07

[Gate 42359574023](https://github.com/pango07/quod/actions/runs/42359574023)
at `554f49e` passed all 46 tests, including eight constructor cases. The
revision resolves the actual constructors through the native promise-stat
function's lexical conversion helper; public `fs.Stats` is a deprecation
wrapper. Function-call breakpoints avoid the internal-script location
breakpoint failure described above.

At entry, the diagnostic accepts only primitive constructor arguments, an
empty extensible owner and no inherited metadata fields. Temporary setters
replace themselves with ordinary data fields plus the existing native read
observer as each pinned constructor assignment runs. The tested prototypes,
final descriptors, values and queued microtask order agree with ordinary
execution. The first promise consumer's `mtimeMs` read is observed. Inherited
setters, coercion side effects and nonextensible owners reject observation;
later writes retain their value and invalidate it.

The [evidence](stat-constructor-entry.json) retains the diagnostic reports.
This establishes the tested early-observation mechanism, not a complete
input boundary. Startup, debugger/native capabilities, query identity and
lifetime integrity remain unresolved. It enables no production replay and
supplies no savings claim. The next remote diagnostic measures this mechanism
around the unchanged expensive property test with ordinary controls; even a
passing result would still require a fresh-runner replay and mutation gate.

## Unchanged expensive test, 2026-11-07

[Gate 42359952265](https://github.com/pango07/quod/actions/runs/42359952265)
at `998b9091f434f8758b2ebfb4f7cdc1764c718550` passed 48 regressions and
three executions of c11's unchanged `stream.property.test.ts`. All 47 test
names and passed states agree. The test's cases, seeds and assertions were
not changed. These are commands on one runner, not whole-job measurements.

| Phase | Command seconds | Tests passed |
| --- | ---: | ---: |
| Ordinary before | 57.531 | 47 |
| Constructor observation | 58.211 | 47 |
| Ordinary after | 56.759 | 47 |

The observed command took 0.680–1.452 seconds longer than the two controls.
Fixed phase order and one sample do not establish causal overhead or a
stable percentage. This result is encouraging for feasibility but contains
zero replay and no fresh-runner or savings claim.

The main-process diagnostic observed 291 Stats constructions, all fully
initialized and intact at its reporting point, with no instrumentation
failure or rejected construction. Its source-frame query heuristic mapped
80 synchronous stats and one promise stat; 210 constructions remain unmapped.
It is diagnostic attribution, not a certified path identity.

| Observed field set | Objects |
| --- | ---: |
| Mode only | 235 |
| No field read observed | 39 |
| Device, inode and mode | 16 |
| Size only | 1 |

The size reader maps to the property-test source path. All 16 device/inode
readers remain unmapped. Their consumers and meaning must be established;
their values cannot be discarded because the final test results agree.
The observation window excludes workers, earlier startup and later exit
listeners. Query attribution and the complete observation boundary remain
required before production keys can be narrowed.

The added collection regression deliberately overwrites `size` with `99`,
reads it and collects the object. Its callback integrity becomes unknown.
This is **not** a demonstrated missed original metadata read: that program
uses a replacement constant. A possible simpler dependency argument would
track every consumption of the original immutable metadata value, including
copies and value-preserving descriptor changes. It would still need to
exclude unobserved earlier aliases/native or debugger reads and preserve
durable read/failure evidence after collection. No such certificate is
implemented or claimed here; retaining Stats objects to avoid collection
would also need an argument about observable lifetime behavior.

The [verified evidence](stat-constructor-workload.json) retains the exact
archive digest, report digests, independent name/state comparison, image and
field groups. No full window was repeated for this diagnostic.

## Prepared benchmark harness

`pango07/ci-bench` branch `bench/descriptor-safe-codex`, commit
`919b5358fce819aa6bbf018e14a7761d40d083f2`, starts from the original full-window
harness `8131a61c3c614497d5b48a775e499010fee5f192`. Its only changes are the
Action SHA, binary version and binary digest. It pins gated Action
`a1b9a37a4bf38e1883a4afc07d397616fa985942`, which includes deferred Cloud saves
and the descriptor-safe `bin-2bd297b` engine. Original comparison branches
remain intact. No new window was dispatched for this pin-only change.

The [last measured full window](convex-helpers-current-cloud-window.md)
remains 1,538 versus 1,735 runner-seconds for default Quod, with no test or
unit replays. This feasibility gate supplies no new speed or cost claim.
