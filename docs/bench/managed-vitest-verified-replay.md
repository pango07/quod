# Managed Vitest: verified fresh-runner replay

The unchanged expensive test now replays on a fresh GitHub runner under the
explicit managed contract. All 47 test names/states, file results and required
console effects match a separate uncached managed execution. The consumer
executes no managed tests; it recompiles, validates the trusted receipt and
returns the recorded result. Production Action and Cloud behavior are unchanged.

[Gate 42376989108](https://github.com/pango07/quod/actions/runs/42376989108)
passes at `60fb521ec9e8cf6ce2f5062d557ed8978a71b610`. Separate hosted jobs
record, execute an uncached control and consume the receipt. The independent
artifact audits pass after verifying every archive digest, compiled identity,
runtime invocation, receipt byte digest, execution count and compared effect.
Full identities, all retained evidence-file hashes, step/job timings and audit
results are in the [evidence JSON](managed-vitest-verified-replay.json).

| Measurement | Record | Uncached managed control | Fresh-runner replay |
| --- | ---: | ---: | ---: |
| Command, including preflight/preparation/compilation | 164.477 s | 111.192 s | 1.857 s |
| Complete job, including setup and artifact transfers | 197 s | 143 s | 28 s |
| Modeled rounded minutes, per job | 4 | 3 | 1 |
| Expensive-file executions | 1 | 1 | 0 |

The observed replay job is 115 runner-seconds, or 80.4%, shorter than its
uncached managed control. The two execute jobs have substantially different
CPU performance despite sharing the same native software class. This is one
observed reuse comparison, not a whole-window savings forecast. The public
repository's actual GitHub runner charge is zero; rounded minutes above are
the model `ceil(job seconds / 60)`, not a claimed charge reduction.

## What passed

The hardening job passes 40 compiler checks, 31 capability checks in each of
two fresh processes, 18 isolated boundary cases and 107 admission checks. It
also compares 12 actual Vitest fixture states and seven module-fixture states
with ordinary controls. Its expensive managed/ordinary pair preserves all 47
states. Managed completion seals with zero pending timers, microtasks, event
callbacks or asynchronous errors, without exempting framework timers.

Relevant real test bytes, dependency values, declared environment,
configuration seed and glob-module names change the recompiled identity and
miss the receipt. An unrelated file outside the input set preserves the key
and hits. That small compiler-to-ledger submatrix uses an explicitly constructed
passing response; it is admission integration evidence, separate from the real
47-state execution and fresh-runner replay above. The seed's separate 91-check
matrix rejects failures, incomplete results, missing policies, obsolete records,
altered checksums and coherent receipt tampering without the trusted job digest.

The fixed legacy-handler regressions pass: global handlers cannot suppress
private failure observers; local legacy handlers and changing getters are
declined before invocation; AbortSignal.onabort is declined while registered
abort listeners work. Asynchronous failures remain failures. The
[earlier hit](managed-vitest-first-replay.json) was provisional while that gap
was being closed; this run validates the fixed policy.

## Scope and remaining product work

The workload remains original c11, with its original three property-test seeds,
24 cases per seed, assertions and test count. The contract declares clock,
randomness and environment. Exact configuration sources, Vitest 5, native
binary/flags, supervisor, capability fingerprint, image/kernel/architecture/libc
and trusted receipt bytes are keyed. Preparation and compilation always run
before lookup. A different native class misses. The compatibility probe retains
ordinary controls and labels unsupported fallback; the research receipt gate
fails when the pinned candidate is unsupported. General Action fallback dispatch
is part of the remaining product integration.

This is an opt-in research harness for the pinned configuration. It is not yet
the distributed Action's managed input, broad Vitest compatibility or production
admission. Recording remains slower than ordinary in the paired diagnostic:
71.450 s managed versus 37.120 s ordinary in this gate. Cold/miss economics and
the original twelve-commit window still need the actual expanded experiment.

The original ordinary window remains 1,538 versus 1,735 runner-seconds, or
11.4% saved. Its 50% target is unmet. No floor job was measured in this batch,
so it supplies no new floor-plus-N claim. No production deployment, main push,
Action pin or test reduction was performed.
