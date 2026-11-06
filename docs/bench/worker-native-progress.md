# Full native diagnostic: retained test progress and timing

[Run 42267064963](https://github.com/pango07/quod/actions/runs/42267064963),
commit `aaed633`, failed after the observed phase reached its 900-second
limit. All 285 preceding regressions passed. The observer is not suitable
for production replay: this run has two received test-state differences
as well as missing results. The product performance target remains unmet.

| Measurement | Plain baseline | Observed phase |
| --- | ---: | ---: |
| Phase elapsed seconds | 97.853 | 900.090, timed out |
| Complete JSON test-state report | 120 modules / 4,131 states | Missing |
| Incremental results received | 4,131 | 4,108 |
| Passed / skipped / failed received | 4,130 / 1 / 0 | 4,105 / 1 / 2 |
| Received states differing from baseline | — | 2 |
| Baseline states absent from journal | — | 23 |

Both differing results belong to seed 1's “composed streams preserve scope
under adversarial cursors” tests, ascending and descending. Their failure
messages were not retained. The observation cost makes timeouts a plausible
cause, but that cause is not established by this artifact. These are
diagnostic execution differences; no test was replayed.

The last unmatched ready event names seed 12648430's descending stale-cursor
test. Reporter callbacks are delivered in batches: this names an unresolved
journal entry, not necessarily the exact test executing at termination.
For example, a baseline stale-cursor test took 42 ms in Vitest's JSON report
while its reporter receipt span included a later expensive test. Receipt
spans must not be presented as actual test durations.

The stream worker's last native checkpoint retained 2,450,341 captures and
4,901,969 compilation events without log/index overflow or capture errors.
It measured 481.741 seconds inside completed native callbacks, including
293.985 seconds in `Runtime.callFunctionOn` and 50.594 seconds in object
release commands. These inclusive measurements overlap and must not be
added. The checkpoint was taken inside one active callback, before its
handle release. The native Inspector bridge is a major measured cost.

All 119 completed worker reports pass the scoped metadata-owner, transport,
compilation, native cleanup and caller-source checks. Their 1,272,788
captures were cleared and released. All 48,285 watched-accessor calls match
the checked-in diagnostic source. This partial evidence does not establish
whole-suite agreement, the after phase, or complete input ownership. The
planned 39-file metadata mutation was not reached.

The separate `changes` job failed to resolve the short input ref `aaed633`.
The measured job checked out the full run head successfully. Use full SHAs
for future dispatch inputs; do not describe this workflow as a passing gate.

[Machine-readable evidence](worker-native-progress.json) includes the
artifact digest, all 167 file digests, checked progress, native counts and
timings. The independent checker is
[`verify-test-progress.py`](../../scripts/bench-vitest/verify-test-progress.py).
It checks event order, unique task identities, ready/result pairing, and the
complete JSON report when available. It also matched both completed plain
runs from [42265335788](https://github.com/pango07/quod/actions/runs/42265335788).
Checks against deliberately corrupted journals rejected duplicated sequence
numbers, changed result names, an empty log, and a passed end with an
unresolved ready event. A valid partial journal remains explicitly partial.

The next experiment should remove the per-operation Inspector request, not
repeat this full diagnostic with another small serialization change. Pinned
[V8's breakpoint-condition path](https://github.com/nodejs/node/blob/v24.18.0/deps/v8/src/debug/debug.cc)
passes the actual exception value to `DebugDelegate::BreakpointConditionEvaluated`.
The [native interface](https://github.com/nodejs/node/blob/v24.18.0/deps/v8/src/debug/debug-interface.h)
exposes that callback and function breakpoints. A direct native delegate is
only a candidate: it must be measured in an isolated process, preserve
argument validation and cleanup, and account for Node's existing debugger
ownership. Installing a second delegate blindly would be unsound. No such
replacement is implemented or enabled by this evidence commit.
