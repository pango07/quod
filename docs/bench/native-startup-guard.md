# Native stat guard before preloads

The type-only recorder now requires a native snapshot from before user
preloads. Previously a preload could replace `Stats.prototype.isFile` with
a timestamp reader, or wrap `fs.statSync` to expose metadata, before the
proof took its initial snapshot. The recorder then classified the read as
`st:` instead of preserving `sm:`.

[Baseline gate 42159856708](https://github.com/pango07/quod/actions/runs/42159856708)
at `eed0373` fails both new regressions while the previous 58 production
proof tests and 28 observer tests pass. This demonstrates incorrect key
classification; the baseline gate itself does not measure a wrong replay.

The production fix in `c80077b`, finalized in `081f20a`, prepends a small
CommonJS bootstrap to recorded Node runs with proofs enabled. It restores
the original `NODE_OPTIONS` before user preloads, captures the native stat
binding and prototype chain, and binds the proof's ESM imports and language
intrinsics before user code. A later CJS cache replacement cannot replace
that captured origin. Unknown startup ordering or options retain metadata.
The recorder passes the actual wrapped function to the guard; checking the
public function after Quod has wrapped it would be insufficient.

The launcher changes only the spawned child's environment. The planner
continues to key the original environment. Plain witnesses, no-record
runs, and the full-replay fast path do not receive this bootstrap. Both new
JavaScript modules are embedded and included in the recorder code hash, so
older ledgers cannot be reused with the new recorder.

[Focused gate 42160284888](https://github.com/pango07/quod/actions/runs/42160284888)
at `c80077b` passes 28 observer tests and 65 production proof tests. These
include the two original regressions, missing origins, unsupported startup
flags, a replaced native binding, exact absence/empty environment handling,
and a poisoned CJS cache.

[Built-binary gate 42160288152](https://github.com/pango07/quod/actions/runs/42160288152)
at `c80077b` passes the two preload scenarios through record, confirmation,
and warm replay. A timestamp-only change then reruns the newly selected two
tests and matches plain Vitest in each case (`wrong = 0`). The existing
native type-only case still replays across a metadata-only change, and a
file-to-directory change still invalidates it. The final `081f20a` gate adds
an explicit no-record regression.

[Final binary gate 42160729489](https://github.com/pango07/quod/actions/runs/42160729489)
at `081f20a` passes 99 CLI unit tests, one unit integration test, 16 recorder
regressions, and 65 JavaScript proof tests (181 total), plus clippy, a clean
format check, and all built-binary metadata scenarios. The no-record
regression confirms that its configuration sees no proof bootstrap.
The binary archive SHA-256 is
`b31627739db6fe8ebdbed4d6aa54e7307e86665cefc677eca0bd7b3f93c95a9b`.
The audit and binary jobs both used Ubuntu 26.04 image `20261101.186.1`.

[Diagnostic compatibility gate 42160884373](https://github.com/pango07/quod/actions/runs/42160884373)
at `6a8c184` passes 29 observer tests and the same 65 production proof
tests. The additional fixture verifies that the existing startup diagnostic
can follow the production origin bootstrap before any user preload.

[Action gate 42161209539](https://github.com/pango07/quod/actions/runs/42161209539)
passes five Action tests, the recorder upgrade and configuration/Boolean
proof integrations, and the lineage switch regression: two promotions,
one plain audit, zero disagreements. Action `1aa08ca` contains the exact
remotely built bundle; its main bundle SHA-256 is
`76296a7e7e4c15431fe5667aee8bbb57faea8e7385db61a09ebeea66ea0dbebe`.
Harness `62a5c2d` pins that Action and `bin-081f20a`.

[Real driver gate 42161262009](https://github.com/pango07/quod/actions/runs/42161262009)
at `b486d42` uses the new binary and passes with 120 file/project instances,
4,131 states matching fingerprint
`09ebb9e3a722220d3a21ddfe9f63992a8a074b0fcaa95764e54c37be90d469cd`,
and zero replays. All 120 size consumers retain the diagnostic's native
outcome, owned-hook, constructor-field and startup-origin checks. The
ledger remains 299 `sm:`, 257 `sl:`, and 104 `st:` keys. The production
bootstrap runs first, followed by the diagnostic and the report preload.
The runner image is Ubuntu 26.04 `20261101.186.1`. This is compatibility and
correctness evidence, not a measured hit or cost window.

This guard does not enable async size keys or establish new savings. The
remaining async proof must still exclude other metadata observers, account
for the result's lifetime and consumers, and validate query equivalence.
The corrected full-window cost and hit-time targets remain unproven.
See [machine-readable evidence](native-startup-guard.json).
