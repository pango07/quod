# Replay architecture review, 2026-11-07

The founder requested a first-principles rethink. The original target stays
unchanged: original cold 12-commit window at no more than 0.5x plain
runner-seconds, a genuine hit at floor + at most 8 seconds, wrong = 0,
one-line integration, and no hidden change to the tests' meaning. This
document is a design review, not approval of a new execution policy.

The current measured implementation has 1,538 runner-seconds against
1,735 plain. It needs to remove another 670.5 seconds. It replays no tests
or units in that window. Dependency installation saves 471 seconds; test
work costs an extra 193 and other work an extra 81. The
[verified window](bench/convex-helpers-current-cloud-window.md) includes
Cloud costs and retains the per-commit evidence.

The latest [isolated feasibility run](bench/wasi-feasibility.md) confirms
the concentration of useful work. `stream.property.test.ts` took 98.575
seconds in a 99.777-second native suite command, then 98.064 seconds in
the 99.128-second native control. Other tests overlap; these figures must
not be added or turned into a predicted whole-job saving. The
[exact test source](https://github.com/get-convex/convex-helpers/blob/5eaa7e5160736054d53f8b71e2c522835eb427e0/packages/convex-helpers/server/stream.property.test.ts#L21)
uses fixed default seeds `1`, `0xc0ffee`, `0xdeadbeef` and 24 cases, with
explicit environment overrides. This does not prove the rest of its runtime
deterministic, but a new random seed on every invocation is not its stated
default. Do not reduce cases, seeds, assertions or coverage to win the bench.

## What a cache hit must establish

A previous passing result is reusable only when the computation sees
equivalent relevant inputs and its required outputs/effects can be
reproduced. Source contents alone are insufficient. Configuration, runtime,
dependencies, environment, negative filesystem lookups, directory contents,
metadata, external services, time and randomness may all matter.

Past observations are useful only with a complete observation boundary and
an appropriate determinism contract. Two matching results or a sampled
audit do not prove that boundary complete. E1/E2 and their audits do not
replace input tracking; this review does not silently change their switches
or the founder's accepted witness policy.

The current design treats main-process reads as shared inputs of every test.
Conservative metadata keys therefore invalidate expensive test execution
because the framework inspected files while preparing the run. Refining a
key requires accounting for all relevant readers, which has led into native
addons, promise observers, heap inspection and worker transport. More
agreement samples cannot resolve missing causal coverage.

## Candidate direction: execute preparation, cache a bounded execution

Evaluate a boundary between framework preparation and isolated test
execution. Continue running discovery, configuration and transformation;
validate the actual module bytes, configuration and runtime inputs delivered
to a worker. Cache execution only when all ways that worker can observe the
outside world are accounted for. Unsupported behavior runs normally.

This is a hypothesis with substantial unresolved requirements:

1. The execution manifest must include every relevant part of the worker
   protocol, callbacks, provided context, dynamic imports, environment and
   runtime identity. Hashing transformed source alone is insufficient.
2. The runtime must enforce or completely observe the filesystem, native
   code, processes, network, clocks and randomness it exposes. A manifest
   is not enforcement. A sandbox alone does not make a test deterministic.
3. Preparation may not introduce unrepresented state or effects into the
   worker. Existing worker diagnostics have not established a complete,
   stable boundary; worker IDs and default sequence seeds cannot simply
   be masked because they vary.
4. Tests that observe real timestamps must keep those timestamps as inputs
   or run normally. Canonical timestamps, frozen clocks, substituted random
   seeds or different framework behavior would change the execution contract.
   Such a managed mode would require an explicit product decision; it may
   not quietly replace the original plain benchmark or become the default.
5. WASI is an implementation candidate, not the boundary itself. Its
   observed compatibility and timing do not remove its host capabilities.

The principle of explicit inputs and controlled tool identity is established
in [Bazel's hermeticity model](https://bazel.build/basics/hermeticity).
That does not establish that a transparent Vitest adapter can achieve it.
The integration and compatibility costs must be measured for Quod.

## Evidence that would justify building further

First write a complete boundary specification and concrete counterexamples
it must reject. Reuse existing failed/provisional worker evidence; do not
restart the same source-hash or field-masking approach under a new name.

The first implementation gate must show a real replay of the expensive
unchanged test on a fresh runner, and rerun on relevant code, configuration,
environment and observable metadata changes. It must preserve the existing
test count, assertions and results. Include deliberate hidden-input and
result-observer cases, not only the known passing repository.

Then restore units and whole-job hits to the same standard, retain lineage
audits and Cloud publication, and run the original cold 12-commit window.
Replaying one expensive file is a milestone, not completion: units, the
floor + 8-second hit and the window cost target remain required.

Stop extending a candidate when its required input boundary cannot be
closed without changing supported test behavior, or when measured miss and
validation costs make the window target unattainable. Report that result
and the concrete compatibility tradeoff rather than adding another informal
exception. No universal savings guarantee follows from one successful repo;
one-line installation can coexist with explicit support limits and plain
fallback.
