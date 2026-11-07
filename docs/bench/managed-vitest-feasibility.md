# Explicit managed Vitest: first execution gate

The authorized investigation has a positive compatibility result. The
unchanged expensive convex-helpers test executes in the candidate managed
runtime with Vitest's real collection, assertions and spies. All 47 names
and states agree with ordinary Vitest. The fixture's deliberate assertion
failure remains failed, and its caught unsupported operation selects ordinary
execution. No cache, ledger, Action integration or production change is enabled.

[Gate 42370932658](https://github.com/pango07/quod/actions/runs/42370932658)
passed at `a9604edcd9808f4c83cd97c3da2cf38a93092ed6` on image
`ubuntu26/20261101.186.1`, Node 24.18.0. The full diagnostic job took 146
seconds, including installs, fixtures, both expensive executions and artifact
upload. It is a combined investigation job, not an accelerated CI benchmark.

| Measurement | Result |
| --- | ---: |
| Managed expensive-file execution, uncached | 44.995 s |
| Separate compilation of that managed bundle | 0.133 s |
| Ordinary expensive-file command afterward | 66.360 s |
| Expensive-file names/states compared | 47 |
| Name/state disagreements | 0 |
| Boundary checks per fresh process | 17 / 17 |
| Fresh processes repeating the boundary checks | 2 |
| Actual Vitest fixture states compared | 12 |
| Managed replays / lineage audits | 0 / 0 |
| Best / median hit; floor + N | Not measured: no hits |

The managed execution includes workerd startup and its response, but excludes
dependency installation, Vitest preparation/configuration and compilation.
The ordinary command includes its own Vitest startup. The observed command
difference is 32.2%; a single runner and fixed execution order do not establish
a repeatable speedup. These are different execution contracts and command
boundaries, not a whole-job or billed-minute savings measurement. A future
replay comparison needs its own uncached managed control on fresh runners.

## Workload and runtime identity

The project is original c11, `5eaa7e5160736054d53f8b71e2c522835eb427e0`.
`packages/convex-helpers/server/stream.property.test.ts` retains SHA-256
`9798bab303233bca846a89ccb39ce28bd968329e0704f3e5d67adca934c11c75`, its three
default seeds and 24 cases. No test assertion or source file was reduced or
rewritten. Static compilation preserves the actual glob name sets and rejects
attempted lazy module loads not materialized in the bundle.

The candidate uses workerd 1.20261107.1, esbuild 0.28.2 and the project's
Vitest 5.0.0. Only workerd's `nodejs_als` compatibility flag is enabled, with
compatibility date 2026-07-01. The runtime executable SHA-256 is
`a4d954a9ec17a367cf7d5509a2282785e84dacca6fc13b69c532ab25d157ef48`.
The [contract](../managed-vitest-contract.md) declares the clock, random seed,
environment, runtime identity, timers, native capability restrictions and two
pinned framework adapters. Preparation executes normally in Node every time.

## Failure and fallback evidence

| Fixture | Managed outcome | Ordinary control | Selected result |
| --- | --- | --- | --- |
| Supported assertions, spies and async context | Four pass | Four pass, exit 0 | Managed pass |
| Deliberate assertion failure | Three pass, one fail | Same states, exit 1 | Managed failure retained |
| Fetch rejection caught by the test | Four pass, sticky `global fetch` flag | Four pass, exit 0 | Ordinary fallback |

The unsupported fixture uses a harmless data URL in ordinary Vitest; its
managed attempt cannot erase the rejection by catching it. Each ordinary run
is retained as a control, and the unsupported one is labeled as the fallback.
This is the prototype harness's behavior, not a shipped generic dispatcher.

A separate fixture deliberately omits the JavaScript guard and calls native
fetch. The configured outbound deny service receives and rejects it; the
raw runtime log contains `QUOD_DENIED_OUTBOUND`. This checks the native routing
policy independently of the JavaScript fetch rejection.

The two preflight processes agree on deterministic random values, native
AsyncLocalStorage separation, logical time and timer order. They also check
constructor/global-prototype aliases and retained rejection of fetch, crypto,
WeakRef, FinalizationRegistry, Event.timeStamp, WebAssembly and AbortSignal.timeout.
Neither process runs on a separate GitHub runner.

## Admission remains closed

Every managed response has `replayEligible: false`; the probe has
`cacheEnabled: false`. Its `managedSupported` compatibility field is not an
admission certificate. The successful expensive run still has **one pending
timer** when results are collected. Vitest's reporting throttle is a plausible
source from static inspection, but this run does not attribute that timer or
prove that ignoring it preserves all effects. Do not drop it from evidence or
admit this result based on matching states.

Before a replay gate, close module resolution including computed imports,
review all reachable native APIs and aliases, define complete async/effect
termination, and validate configuration/preparation/output ownership. The
contract lists the remaining work. Then use fresh GitHub runners to compare
uncached managed execution with actual replay, requiring relevant source,
configuration, environment and module-name changes to miss and unsupported
work to execute normally. Include all setup, validation and storage costs.

This is sufficient evidence to continue the managed investigation. It does
not meet the original founder's benchmark bar. The unchanged
[original twelve-commit window](convex-helpers-current-cloud-window.md) remains
1,538 versus 1,735 runner-seconds (11.4% reduction), with zero test/unit
replays. Managed execution cannot silently replace that baseline.

## Reproducible evidence

Artifact `13573518576` is 818,657 bytes with SHA-256
`7731d6a20e614d4fe9b0e4057688c1e0235e174796eb8dc0080dcadd1b9621ee`.
The archive was verified before extraction. The
[independent auditor](../../scripts/bench-vitest/managed/audit.py) checked the
raw reports, response/bundle hashes, runtime configuration, every compared
state, all fixture outcomes and cache-disabled flags. The
[evidence JSON](managed-vitest-feasibility.json) retains identities, all 47
state pairs, complete boundary checks, glob sets and all 51 extracted runtime
evidence-file hashes (the archive also contains the dependency lockfile).

Earlier gates are preserved in that JSON, including two failed fixture gates.
The earlier green gates only established boundary checks and ordinary fallback;
they did not execute the expensive managed test. They are not successful
managed compatibility or performance measurements.
