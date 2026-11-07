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

## Admission stays closed

Every result still says `replayEligible: false` and the host says
`cacheEnabled: false`. These changes are not a complete soundness proof. Native
Date parsing, locale and timezone semantics need an explicit policy. Vitest's
mutable worker, matcher, configuration and task state remain in the test realm;
public intrinsic protection alone cannot certify their integrity. Resource
timeouts and unresolved effects must remain incomplete. No ledger is allowed
until those boundaries are enforceable and reviewed.

The next performance evidence must compare uncached managed execution with
actual replay on separate fresh runners and retain relevant-mutation misses,
unrelated-change hits, outputs and complete job/transfer costs. The ordinary
twelve-commit result remains separate: 1,538 versus 1,735 runner-seconds, with
zero test/unit replays. Its 50% bar is still unmet.

Remote validation for this follow-up is pending. Do not quote the old 44.995 s
managed execution as a result for the hardened candidate.
