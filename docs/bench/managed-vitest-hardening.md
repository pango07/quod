# Managed Vitest hardening follow-up

This research branch continues the [first execution gate](managed-vitest-feasibility.md).
The saved handoff changes were restored from `ac17261` and independently hashed
before further work. Production Action and Cloud behavior remain unchanged.

**Current result:** the fixed policy now passes independently audited
[fresh-runner replay](managed-vitest-verified-replay.md). The history below
retains the earlier gates and the defect closed before that result.

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

The first actual experiment subsequently passed at `edc7c04` in
[gate 42376167015](https://github.com/pango07/quod/actions/runs/42376167015).
Its independent [receipt audit](managed-vitest-first-replay.json) verifies all
47 states and console effects across separate seed, uncached control and
consumer jobs. The consumer executed zero managed tests. Commands took
109.742 s seed, 162.873 s control and 2.718 s replay; complete jobs took 141,
195 and 33 s respectively. This is one observed workload on a matching native
class, not the original twelve-commit benchmark or a general cost forecast.

That earlier admission policy was provisional: pinned-source review then identified
legacy workerd `on<type>` handlers running before registered error listeners.
The verified revision freezes global error/rejection/fetch handler slots, declines
AbortSignal.onabort, and rejects local legacy descriptors before invoking any
getter. It installs an immutable null slot before local native dispatch so a
second property read cannot evade the check. Eighteen isolated boundary modes
now cover suppression, async failure, pending work and the changing-getter case.
The fixed-policy batch now passes in
[gate 42376989108](https://github.com/pango07/quod/actions/runs/42376989108),
including its independent artifact audits.

Local events support registered listeners. Legacy handler attributes are outside
this managed contract. A dispatch installs a non-enumerable immutable null
`on<type>` slot on the target; reflective code can observe that managed metadata.
An existing legacy descriptor, an accessor, or a target that cannot accept the
guard declines managed admission. Registered listener order and once behavior
remain covered by the capability and actual Vitest gates.

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

The 31-check preflight, symbol inventory and shared supervisor pass in
`42376167015`; the 18-mode legacy-handler revision and actual compiler-to-receipt
mutation matrix subsequently pass in `42376989108`. Production remains off.
