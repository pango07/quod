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
