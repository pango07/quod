# Managed Vitest investigation: explicit contract, version 0

The founder authorized investigating an opt-in managed mode, with unsupported
work running normally. The ordinary twelve-commit benchmark remains separate.
This document specifies the first executable investigation; it does not
declare a complete runtime, a replay guarantee, or a new savings result.

## Candidate and enforcement

Run a single test file in a fresh local workerd process, with the full Node
compatibility layer disabled. Enable only `nodejs_als`: convex-test needs
actual asynchronous context propagation, which a Promise.then patch cannot
provide. Use Vitest 5's actual collection, runner, assertions and spies.
Do not reduce the expensive test's seeds, cases, assertions or test count.

Preparation runs normally in Node and produces a self-contained bundle.
The prototype compiles before every execution/replay decision. A future key
must include the resulting bundle, resolved configuration, module-name sets,
declared environment, runtime binary, compatibility flags and runtime adapter.
Preparation effects cannot be skipped. No reusable ledger is produced by v0.

| Capability | Initial policy and enforcement |
| --- | --- |
| Module loading | Bundle static dependencies. Only node:async_hooks can remain external. Unsupported imports reject the managed attempt. |
| Lazy glob modules | Preserve the actual name set. Loading a module not materialized in the bundle marks unsupported; it cannot silently return a fake module. |
| Filesystem/process/native addons | No disk service, process binding, native addon loader or full Node compatibility. |
| Network | Runtime outbound service is a deny worker, in addition to the JS API guard. No external service binding. |
| Environment | Explicit copied values only. No host environment or OIDC credentials passed into the worker. |
| Time | Fixed declared epoch plus virtual timer advancement. Original Date constructor remains private; prototype.constructor must not expose it. |
| Randomness | Declared deterministic PRNG seed. Cryptographic randomness is unsupported. |
| Scheduling | Promise microtasks and one ordered virtual timer queue. No shared memory, worker threads, native asynchronous crypto or WebAssembly. |
| Results/effects | Capture console output and test states. No published files or external effects. Snapshot writes and unsupported RPCs reject eligibility. |
| Limits | Supervisor timeout/crash is an incomplete managed attempt, never a reusable pass. |

The initialization module runs before Vitest or test dependencies. Unsupported
operations set a private sticky flag before throwing, so catching the error
does not make an attempt eligible. Every attempt uses a fresh process.

Workerd has other native Web APIs, prototypes and implicit behaviors. Version 0
must inventory and test its exposed surface before any cache can be enabled.
The initial guards are a candidate boundary, not a complete soundness claim.

## Verification and fallback

First gate: native async-context separation, seeded clock/random agreement
between fresh processes, ordered timers, caught unsupported operations,
network rejection, absence of Node filesystem/process capabilities and
constructor aliases. Then attempt the unchanged expensive convex test with
the real Vitest runner. Compare all names/states with ordinary Vitest, retain
raw reports, and measure preparation and execution separately.

An unsupported or incomplete managed attempt runs ordinary Vitest. A managed
test failure remains a test failure; fallback must not overwrite it with a
passing result merely because the environments differ. In v0 ordinary Vitest
is always run as a control; the evidence explicitly labels unsupported cases.

Next gate, only if the boundary and compatibility checks pass: an uncached
managed run and a real replay on separate fresh GitHub runners; relevant
source/configuration/environment/module-name changes must miss, unrelated
changes must hit, and unsupported work must execute plain. Measure all setup,
compilation, validation, transfer and execution costs. Only then consider a
managed twelve-commit window alongside the original one.

Primary references: workerd's
[configuration schema](https://github.com/cloudflare/workerd/blob/main/src/workerd/server/workerd.capnp),
[ALS-only compatibility flag](https://developers.cloudflare.com/workers/configuration/compatibility-flags/),
and the pinned [Vitest 5 runner](https://github.com/vitest-dev/vitest/blob/v5.0.0/packages/vitest/src/runtime/runner/run.ts).
