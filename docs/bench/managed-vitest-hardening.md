# Managed Vitest hardening follow-up

This research branch continues the [first execution gate](managed-vitest-feasibility.md).
The saved handoff changes were restored from `ac17261` and independently hashed
before further work. No Action, Cloud deployment or reusable result is enabled.

The candidate compiler parses original modules and the final bundle, admits only
its reviewed adapter imports and a restricted AsyncLocalStorage facade, and
retains source, resolution, glob-set and configuration identities. Computed
imports, dynamic evaluation and ambient module metadata retain private rejection
flags when caught. Preparation always executes and currently accepts only the
pinned c11 workload's configuration sources. Only documented configuration path
fields are canonicalized; arbitrary string replacement is forbidden.

The runtime now attempts bounded completion of all managed timers, microtasks
and local event listeners. It captures detached errors, rejects global event
receiver aliases, protects core intrinsics before test import, and serializes
own result data without invoking user toJSON hooks. Date.now spies and the pinned
framework's stack formatting slots must be restored before completion seals.
There are no timer exemptions based on a callback's presumed origin.

The remote gate runs compiler mutations, repeated capability probes, isolated
completion/tampering fixtures, and actual Vitest failure/fallback/module-loading
fixtures before attempting the unchanged 47-state expensive file and ordinary
control. A compiler or small-fixture failure stops before the expensive pair.
Static syntax checks are not runtime evidence.

## Verified hardening gate

[Gate 42375428420](https://github.com/pango07/quod/actions/runs/42375428420)
passed at `441de9744baeab3a869e09581ce456781b37f0df`. The independently verified
artifact passes 40 compiler checks, 26 capability checks in each of two fresh
processes, all 13 isolated lifecycle/tampering cases, 12 ordinary-control Vitest
fixture states and seven module-fixture states. All 47 expensive-file states
match ordinary Vitest. Completion seals with zero pending timers, callbacks,
microtasks, event callbacks or asynchronous errors. The prior pending timer
is drained by the common policy, with no origin-based exemption.

Managed execution took 158.521 s versus 83.541 s ordinary. The full diagnostic
job took 303 s. This candidate's recording is slower; do not reuse the old
44.995 s result or claim a speedup for hardening. The
[evidence JSON](managed-vitest-hardening.json) retains the artifact digest,
independent audit and measured boundaries. There were zero replays.

## Bounded research admission

Runtime responses still say `replayEligible: false`, the compatibility probe
says `cacheEnabled: false`, and production admission remains disabled. A
separate manually dispatched research gate now prepares an exact-byte receipt,
an independent uncached managed control and a consumer on three fresh runners.
It has not passed yet.

The research key includes the complete compiler/preparation identities, actual
runtime binary, flags, deny service, supervisor/validator source, watchdog,
native capability fingerprint and declared UTC/locale environment. It also
requires the same image, kernel, architecture and libc class. Cross-image reuse
misses. Preparation and compilation execute before every lookup. The consumer
requires a receipt byte digest supplied through trusted seed-job outputs,
revalidates sealed passing results and compares all names, states and console
effects with the independent uncached managed control. Failures, pending work,
unsupported operations, older records and altered bytes cannot yield a hit.

The runtime review found no concrete additional input in mutable framework
state when the full bundle/configuration and native policy are keyed. That
state alone is not a reason for indefinite deferral. Native Date semantics are
bounded by the explicit same-image UTC/locale policy, with validation on each
consumer. These are narrow research semantics, not arbitrary Node equivalence
or proof of universal compatibility. Resource timeouts and unresolved effects
remain incomplete.

The next performance evidence must compare uncached managed execution with
actual replay on separate fresh runners and retain relevant-mutation misses,
unrelated-change hits, outputs and complete job/transfer costs. The ordinary
twelve-commit result remains separate: 1,538 versus 1,735 runner-seconds, with
zero test/unit replays. Its 50% bar is still unmet.

The new 31-check preflight, symbol inventory, shared supervisor and receipt
experiment require their own remote validation. The passing gate above applies
to the 26-check source at `441de97`.
