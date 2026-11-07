# Replay architecture review, 2026-11-07

**Decision update:** the founder has authorized investigating explicit managed
Vitest execution, with unsupported work running normally. The
[managed contract](managed-vitest-contract.md) specifies the separate,
opt-in prototype. It enables no cache or production change. The original
benchmark and acceptance target remain separate; the decision request below
records the reasoning that led to this authorization.

The founder requested a first-principles rethink. The original target stays
unchanged: original cold 12-commit window at no more than 0.5x plain
runner-seconds, a genuine hit at floor + at most 8 seconds, wrong = 0,
one-line integration, and no hidden change to the tests' meaning. This
document is a design review; the later authorization above covers an
investigation, not a production execution-policy change.

## Decision in one page

The product's job is to avoid repeated computation while preserving the
answer and required effects. Its economic test is:

`execution avoided > recording + validation + transfer + audit + integration`

The current full window saves 11.4% of runner time and 11.1% of modeled
rounded minutes, with zero test or unit replays. It validates dependency
installation savings, not the central test-reuse proposition. Agreement
with plain when everything executes cannot establish correct reuse.

The recurring failure is architectural: observing more of ordinary Node
makes reuse safer but exposes runner-specific inputs that invalidate it.
Ignoring those inputs without an equivalence argument creates stale results.
Prior agreements and lineage audits cannot resolve that conflict. The
[same-job cohort recovery regression](bench/lineage-cohort-recovery.md)
also demonstrates why a detected audit fault must invalidate current results,
not only future records.

Keep the one-line installation goal. Separate it from eligibility: replay
requires an explicit supported execution contract; unsupported work runs
normally. Recommend a bounded investigation of a managed Vitest execution
contract with immutable inputs, explicit environment and controlled external
effects. Name the concrete enforcement mechanism and unsupported features
before implementing it. Do not treat a container, manifest or passing
compatibility test as proof of that contract.

The next performance gate is one unchanged expensive test on a fresh runner,
with required effects preserved, relevant source/configuration/environment
and metadata mutations forcing execution, unrelated changes permitting
reuse, and cold/miss/hit overhead measured. Stop the candidate if the boundary
cannot be enforced or its costs consume the execution saved. Do not extend
it through an indefinite sequence of observer exceptions.

Only after that gate passes should it expand to the original twelve-commit
window and whole-job target. A managed runtime that changes observable
semantics needs an explicit product decision and its own uncached control;
it cannot replace the original benchmark silently. No managed runtime is
enabled by this review. The original target remains unmet.

The founder has approved investigating that narrower, explicit support
contract. There is no evidence-based commitment today to universal drop-in
40–50% savings. Rewriting the runner in Rust does not answer the input-boundary
question, and fixed-overhead improvements cannot close the measured gap.

## Reset: choose the execution contract before extending the observer

Stop treating another Inspector/metadata patch as the default next step.
Keep the evidence and regression cases. The latest
[callback-request diagnostic](bench/native-stat-fields.md#native-callback-requests-and-inode-consumers)
passes 52 regressions and preserves the expensive test's observed results,
but still supplies no fresh-runner replay or complete input boundary.
Mechanism progress alone does not justify another full-window benchmark.

The product must satisfy three separate requirements:

1. **Equivalent results:** replay preserves the supported execution's
   results and required effects. Agreement and sampled audits cannot
   establish that an unobserved input does not matter.
2. **Useful reuse:** equivalent inputs recur on fresh runners and normal
   commits. A complete key that changes every time is safe but ineffective.
3. **Positive economics:** avoided execution exceeds recording, validation,
   transfer, audit and integration costs across the complete cold window.

The current window fails the second requirement. Its `wrong = 0` comes
with zero test and unit replays; it does not demonstrate correct reuse.
Faster tracing alone cannot make an invalidated record reusable.

| Direction | Promise | Unresolved work |
| --- | --- | --- |
| Transparent acceleration of ordinary Node/Vitest | Preserve the existing environment and behavior; run plain when reuse cannot be established | Complete, stable, affordable observation for this benchmark |
| An explicitly managed test environment | Reuse under a versioned, enforced input/effect contract; run unsupported work outside the cache | A product decision, enforcement, compatibility, economics and an uncached control in the same environment |

Prefer evaluating the managed direction over extending broad transparent
observation indefinitely. One-line installation can provision an
environment without making every workload eligible for replay. This is a
proposal, not an enabled policy or a demonstrated performance result.

The contract needs immutable input files and a defined metadata view,
pinned tools/dependencies, explicit environment/configuration, and ownership
of outputs. Clocks, randomness, scheduling, native code, processes and
external services each need a supported policy. A container or Node
permission flags alone do not establish deterministic execution or complete
mediation. The isolation/input-identity principle is described by
[Bazel](https://bazel.build/basics/hermeticity); Node documents the limits of
its [permission model](https://nodejs.org/api/permissions.html).

Canonical metadata or controlled clocks/seeds change observable behavior
and must be disclosed. Keep test cases and assertions unchanged. Compare
cached and uncached execution within the same managed contract, while
retaining the original ordinary-GitHub baseline separately. Managed results
cannot silently satisfy the original 12-commit goal; that goal remains
unmet unless its own acceptance test passes.

Before implementation, name the enforcement mechanism and unsupported
capabilities. The first execution gate is an expensive unchanged
convex-helpers test replaying on a fresh runner, with relevant mutations
forcing execution, required effects preserved, and cold/miss/hit costs
measured. Another passing observer fixture is insufficient. Expand to
units, whole-job replay and the complete window only after that gate passes.
Reject candidates that need another open-ended chain of observation
exceptions. Rewriting tests in Rust or reducing property-test cases does
not resolve these requirements.

The [existing-runtime source review](bench/runtime-boundary-review.md) checks
Cloudflare's current Vitest/workerd integration against this requirement.
Its source peer range supports Vitest 5, but it changes filesystem behavior,
retains local clocks and configurable external access, and still runs
preparation in Node. It is not an off-the-shelf solution to the original
transparent benchmark. The benchmark's existing `edge-runtime` environment
is a different implementation and its name does not establish a closed
boundary either. No replacement runtime was installed or enabled. Any managed
prototype must specify enforcement before another compatibility-only run.

## Measured starting point

The [content opportunity audit](bench/convex-content-opportunity.md) adds a
second constraint. Five tracked modules observed for the expensive test are
unchanged across all eleven transitions, but a coarse manifest/configuration
comparison is unchanged on only four. Removing every measured Quod test-step
second on those four transitions would still leave 1,166 runner-seconds in
an arithmetic scenario, above the 867.5-second target. Useful reuse needs
both stable host observations and a precise dependency boundary. Merely
moving an all-repository key into a managed environment is insufficient.

The [real unit descriptor audit](bench/unit-descriptor-origin.md) rules out
a simple pathname-recovery fix for build/release fallback. Actual first
witnesses observe anonymous pipe and Unix-socket metadata, including the
wrapper's captured stdout/stderr. Each of four commands succeeds but
correctly declines caching. E2 cannot help until these observations have
a justified input representation; dropping them is not an optimization.

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

The [dependency time/randomness review](bench/convex-test-entropy-review.md)
confirms that mock document IDs use counters and the test already controls
the creation clock. The library also generates random action request IDs;
their effect on this test's assertions is neither established nor excluded
by the source review. Do not infer a complete determinism contract from the
test's seeded generator, or discard those IDs merely because outputs agree.

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

The [environment descriptor regression](bench/environment-descriptor-contract.md)
is a concrete example: the old recorder tracked presence while a test
could obtain the variable's value through its descriptor. Two stale passes
became two ordinary failures after the value changed. The corrected source,
release binary and packaged Action now pass that regression. This fixes the
identified path; it does not certify all possible host observations.

The current design treats main-process reads as shared inputs of every test.
Conservative metadata keys therefore invalidate expensive test execution
because the framework inspected files while preparing the run. Refining a
key requires accounting for all relevant readers, which has led into native
addons, promise observers, heap inspection and worker transport. More
agreement samples cannot resolve missing causal coverage.

The [filesystem-descriptor regression](bench/filesystem-descriptor-contract.md)
adds a second concrete input omission. The previously packaged engine can
replay three stale passes after only a descriptor-observed timestamp changes.
The corrected source runs those descriptor-dependent tests normally while
retaining replay for an ordinary content-reading control, and invalidates
the older ledger. The latest full-window agreement does not erase this
separate reproduced defect or establish a universal correctness guarantee.

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

## Economics before another optimization

The product earns its keep only when avoided execution costs more than
input validation, recording, audits, transfers and integration overhead.
The relevant hit rate is the fraction of execution time avoided, not the
fraction of test files marked as hits. A hundred tiny hits can be worth less
than one expensive test.

Using the measured default window of 1,538 runner-seconds and the same
1,735-second plain baseline:

| Desired runner-time saving | Maximum Quod window | Further reduction needed |
| --- | ---: | ---: |
| 40% | 1,041 s | 497 s |
| 45% | 954.25 s | 583.75 s |
| 50% | 867.5 s | 670.5 s |

These are arithmetic thresholds, not forecasts or billed-minute targets.
Per-job minute rounding must be computed separately. The 50% gap is about
67% of the measured 1,003 seconds of plain test steps. That is a scale
comparison, not a prediction: miss costs, overlap, audits and replay
validation change the achievable gain.

The next performance experiment must therefore demonstrate substantial
avoided test execution. Do not repeat the full window merely because a
small fixed cost improved. The original cold window remains the final
acceptance test once the prototype clears its correctness and useful-reuse
gate.

The measured default build, Action and post steps total only 82 seconds
(23 + 29 + 30). Even eliminating those steps would not close the
670.5-second gap. Units remain necessary for whole-job replay, but optimizing
them alone is not a path to the window target.

A separate economic safeguard is [adaptive recording admission](bench/recording-admission.md).
When repeated shared-input changes already require every test to execute,
the opt-in policy can run those tests without recording, retry periodically,
and resume recording when inputs stabilize. Its release regression preserves
all proof data, executes a changed failing test, and recovers to ordinary
replay. This reduces avoidable miss work if the real cost check supports it;
it does not complete the input boundary or authorize any additional replay.

The bounded real-suite check now preserves all 4,131 test states and proof
data: 70.554 seconds recorded versus 68.114 adaptive plain, between plain
controls of 68.039 and 67.316 seconds. That 2.440-second difference is small,
uses a primed admission hint and excludes job/storage costs. It supports
keeping the safeguard experimental; it does not replace the fresh-runner
reuse milestone or justify another full-window run by itself.

## Product recommendation and decision boundary

Keep one-line installation, but state compatibility and replay eligibility
separately. Installation can be automatic while unsupported execution runs
plain. A previous result agreeing with another run is evidence about those
runs; it does not establish that all possible inputs have been accounted
for.

Prefer a narrow, versioned Vitest integration with an enforceable input and
effect contract over more broad observation exceptions. This follows the
explicit action-input model described in
[Bazel's remote cache documentation](https://bazel.build/remote/caching),
but its feasibility for unchanged Vitest execution remains to be proved.
Do not call a list of observed inputs an enforceable contract, or call an
unsupported fallback path a demonstrated acceleration.

The prototype must retain the unmodified expensive test and ordinary plain
control. Its acceptance matrix includes unchanged inputs on a fresh runner,
relevant source/configuration/environment/metadata mutations, an unrelated
file change, external effects, and an older ledger. Each relevant mutation
must rerun or reject replay. Each comparison must retain the full test
names, states and expected effects. Unsupported effects must be identified
before replay eligibility is granted, including effects hidden by native
code or a different execution path.

If safe reuse requires canonical metadata, a controlled clock, selected
random seeds, restricted host APIs or test changes, document those as a
separate managed execution mode. It would need the founder's product
decision and a matching managed plain baseline. It cannot satisfy the
original transparent benchmark by silently changing that baseline.

The immediate decision is technical: can this pinned runtime expose a
complete, stable execution boundary cheaply enough? If not, stop extending
that candidate and bring back the concrete support restriction. Rewriting
the test runner in Rust does not answer this question and is not the next
milestone.

The [native Stats field experiment](bench/native-stat-fields.md) tests one
alternative mechanism against the current metadata bottleneck. Its field
callbacks preserve the tested native Stats semantics, but its earlier-
consumer case proves why late installation cannot authorize key refinement.
Any further work on that candidate must establish a pre-publication boundary
and retain the existing observer counterexamples. It has enabled no product
replay and does not replace the fresh-runner or full-window acceptance gates.

The [constructor-entry workload follow-up](bench/native-stat-fields.md#unchanged-expensive-test-2026-11-07)
now passes 48 regressions and the expensive test's 47 unchanged test states
between ordinary controls. Its observed command takes 58.211 seconds against
57.531 and 56.759 seconds plain. It records 291 main-process Stats objects,
but 210 have no query attribution and 16 read device/inode values. This
changes the mechanism's feasibility evidence; it still establishes no
replay, complete input boundary or window savings. Resolve those specific
coverage questions before enabling narrower keys.

The [callback-request follow-up](bench/native-stat-fields.md#native-callback-requests-and-inode-consumers)
reduces unmapped objects from 210 to 14 in another unchanged-test comparison.
It locates all 16 observed device/inode reads in Node's `fs.realpath` handling
of the repository's README and changelog symlinks. That makes a native
operation/result boundary a concrete candidate to evaluate. It does not
make the existing keys redundant: provenance, other observers, error cases,
arguments and effects remain part of the correctness argument.

The [realpath counterexample](bench/realpath-contract.md) demonstrates why
the operation's actual result needs its own input representation. Changing
only a parent symlink produced four stale per-test passes while Quod and
plain both exited with failure. Shared directory metadata masked the defect
until the fixture moved the mutation outside the project. The correction
adds a bounded path-resolution footprint and retains the prior keys; it
does not yet authorize dropping internal metadata observations or establish
fresh-runner test reuse. Preserve this counterexample in any future
operation-level refinement.

The bounded [descriptor-caller probe](bench/unit-descriptor-stacks.md) adds
another concrete constraint. Only 29 of build's 44 anonymous metadata calls
reach `uv_guess_handle`; Node's shutdown path also compares descriptor
identity before restoring flags. The partial release trace reaches Git and
libc, and the diagnostic exceeds its 120-second release limit. This rejects
a blanket type-only shortcut and supplies no replay or savings result.
Equivalent behavior may depend on relationships and effects, not equality
of every raw field. Any such abstraction must prove ownership, all consumers
and descriptor lifecycle; stack names alone cannot authorize it. Do not
extend this into another open-ended tracing project.
