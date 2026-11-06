# Inspector observation of native Stats

This is diagnostic evidence for the async size proof, **not a replay
certificate or a new savings result**. Production still records complete
metadata for async stat. The packaged engine remains `7ff2b2f`.

## What the regression demonstrates

The local consumer is accepted by `certifyAsyncStatSizeReads`: it awaits
native stat in a private const, then immediately sends only `s.size` to a
function, with an empty catch. That function can nevertheless use Inspector
to find the live Stats in the heap and read its timestamp. It does not
receive the Stats reference. Both promise-hook and async-hook registries
remain empty, and the Inspector network listener remains closed.

The new diagnostic watches calls to the actual native `Connection`,
`MainThreadConnection`, and `open` functions through function-call
breakpoints. The regressions establish detection of local public sessions
and a direct native constructor cached before the diagnostic starts. The
recorded event survives disconnection. See Node's
[Session implementation](https://github.com/nodejs/node/blob/v24.18.0/lib/inspector.js)
and [native Inspector bindings](https://github.com/nodejs/node/blob/v24.18.0/src/inspector_js_api.cc).

| Isolated case | Startup native objects | New local connection events | Timestamp observed |
|---|---:|---:|---:|
| No foreign observer | 2 | 0 | No |
| Public session connected before observation | 3 | 0 | Yes |
| Public session connected afterward | 2 | 1 | Yes |
| Cached native constructor called afterward | 2 | 1 | Yes |

**Native object counts are not live-session counts.** The initial fixture
expected one object for the diagnostic's own connection. Both
[42156066614](https://github.com/pango07/quod/actions/runs/42156066614) and
[42156199578](https://github.com/pango07/quod/actions/runs/42156199578) failed
that expectation. Excluding the queried prototype by identity did not
explain the second object; no queried object was that prototype. The final
test checks the additional pre-existing session against an isolated
baseline. It does not use the baseline count to certify absence of observers.

[Final regression gate 42156860657](https://github.com/pango07/quod/actions/runs/42156860657)
at `ecedde1` passes all 15 observer fixtures and 58 production proof tests.
It explicitly verifies that the regression's exact local consumer is
accepted by the current AST certificate. An earlier passing diagnostic
at `953b110` used a different sink form; this final gate is the evidence for
the accepted form.

## Real planner, driver and workers

[Driver gate 42156552821](https://github.com/pango07/quod/actions/runs/42156552821)
at `0cf659e` passes with 120 file/project instances and 4,131 unchanged test
states. Its full fingerprint is
`09ebb9e3a722220d3a21ddfe9f63992a8a074b0fcaa95764e54c37be90d469cd`.

The observer records two new native local connections. Their actual V8
frames lead to Quod's `createBooleanProof` and the lazy `names-proof`
startup. No additional entries into the watched functions were observed.
This supports feasibility on the benchmark; it is not proof against every
observation channel. The startup census is still unclassified, and a
connection made in another worker or native addon is not excluded by this
main-isolate diagnostic.

All 120 test-file size consumers still have an inspected native size
result, only owned promise hooks, unchanged observed native bindings, and
no inherited constructor fields. The ledger remains 299 `sm:`, 257 `sl:`,
and 104 `st:` keys, with zero replays. There is no new hit-job, cost, or
12-commit-window measurement. See [machine-readable evidence](inspector-stat-observers.json).

The next implementation dependency is an early startup boundary that can
attest the original native functions before user preloads run, then keep
unsupported observer channels conservative. An initial snapshot or a count
of two native objects must not substitute for that boundary. Production
size-key integration and fresh-runner validation remain outstanding.
