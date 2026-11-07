# Frozen-slot managed Vitest gate

The [candidate product gate](https://github.com/pango07/quod/actions/runs/42395278059)
passed at [`eb2864eb`](https://github.com/pango07/quod/commit/eb2864ebb9c49a1bf7939a47e8f62d59a717112e).
All four uploaded archives matched their GitHub-reported SHA-256 digests and
byte counts before extraction. Independent data audits passed for the boundary,
receipt policy, dispatcher, native remainder and same-runner comparison.

This is the candidate defined by the [experimental Action contract](../managed-vitest-action-v0.md).
Its boundary contract is `quod-managed-v0-protectors-frozen-slots-investigation`
and its intrinsic policy is `quod-managed-frozen-array-promise-slots-v1-probe`.
Both appear in returned boundary evidence and the keyed runtime policy. Earlier
accessor-policy receipts cannot be admitted under this policy.

The candidate preserves seven original Array/Promise slots before freezing
their four constructor/prototype objects. This explicitly changes writes to
inherited nonwritable data properties: strict assignment throws and
`Reflect.set` may return false, including on instances or subclasses. Explicit
own-property definition on public instances remains possible. A caught native
write failure is allowed under this policy; guarded descriptor-mutation calls
remain unsupported. The other ambient-input and completion guards remain in
force. This profile does not claim arbitrary ordinary-Node equivalence.

## Unchanged checks and added policy checks

The gate retained all 40 compiler checks, 31 capability checks in each of two
fresh processes, 18 adversarial boundary modes, 12 ordinary fixture states,
seven module fixture states and 107 receipt admission checks. The original
stream property file passed all 47 tests in both managed and ordinary execution,
with identical names and states. Its three property seeds and 24 cases per seed
were unchanged. Managed completion was sealed, with no unsupported operations,
async errors or pending timers, timer callbacks, microtasks or event callbacks.

The added protector fixture passed 51 checks in each of two fresh processes.
It covers the seven preserved descriptors, failed prototype and instance
writes, explicit own-property definitions, species/subclass behavior and the
remaining guards. Sixteen additional receipt checks reject missing, altered
and earlier policy evidence, including externally digest-authorized receipts
constructed under the earlier policy. These constructed admission fixtures are
separate from actual worker execution.

All 12 dispatcher cases and 45 checks passed, including one receipt hit and
ordinary fallback controls. Two mixed managed/native-remainder controls passed
20 checks: ordinary success produced one reusable receipt; ordinary failure
preserved exit 1 and produced none. These controls verified exact native
selection, merged assertions and console-marker multiplicity.

## Fixed-order worker diagnostic

| Actual execution | Seconds |
| --- | ---: |
| Candidate managed worker | 126.612 |
| Earlier-policy managed control | 161.249 |
| Candidate's ordinary comparison | 85.553 |

The candidate full hardening run executed first; the earlier-policy control
executed afterward on the same runner. Both used the same Node and workerd
binary bytes, host class, native capabilities, declared epoch, seed and
environment, and unchanged source. All 47 assertion results, file results,
console effects and virtual time agreed. The earlier control used frozen
source [`2a4d6f2f`](https://github.com/pango07/quod/commit/2a4d6f2f7547dd00e720a32d5b451229784ba8fb).

The candidate/earlier-worker ratio was 0.785194, or 21.5% lower elapsed worker
time in this observation. This fixed-order diagnostic does not establish a
causal or general speedup. The candidate still took 1.480 times the ordinary
comparison. It establishes no candidate twelve-commit job savings or fresh-job
cache result. The earlier-policy window remains separate evidence.

## Provenance and run accounting

| Completed job | Runner seconds | Modeled rounded minutes |
| --- | ---: | ---: |
| [Boundary and worker comparison](https://github.com/pango07/quod/actions/runs/42395278059/job/128471204782) | 464 | 8 |
| [Dispatcher and native remainder](https://github.com/pango07/quod/actions/runs/42395278059/job/128471205057) | 78 | 2 |
| Total | 542 | 10 |

These durations include each job's setup, uploads and post steps. Modeled
minutes sum the ceiling of each complete job's seconds divided by 60; they are
not invoice charges. No Action build or candidate full-window run is included.

| Archive | Artifact ID | Bytes | SHA-256 |
| --- | ---: | ---: | --- |
| managed-runtime | 13582854874 | 3,591,675 | `132ded0ec65be7002ba960cedbe5972bd22be0509444cd3c8ca93339b404ae9a` |
| managed-protector-ab | 13583587039 | 369,519 | `175271b4f4d7c63b764112fa2531b2e683c2d9b57e317ba14b0af9d955e80cd9` |
| managed-dispatcher | 13583500764 | 1,892,007 | `defc195d1a20db1974017ed46dec8d4f7f8ab9bde9e69ded720c987bca2e4fe9` |
| managed-dispatcher-remainder | 13583580489 | 464,373 | `1c2a991042c84d4ba9ca95e364360c7dec56ebf0d09cbd62c184c15703aa6b72` |

The [JSON evidence](managed-vitest-protector-gate.json) retains exact archive
metadata, raw response/build/config identities, auditor identities, complete
audited file hashes, step timings and comparison data. The later packaging
source [`293afb71`](https://github.com/pango07/quod/commit/293afb713a1ceab141f290db8a002728298cb5c0)
adds independent data-audit code; all nine runtime source files match the tested
commit byte for byte. The separate [Action build](https://github.com/pango07/quod-action/actions/runs/42396142682)
passed 22 checks and produced Action
[`c77d8656`](https://github.com/pango07/quod-action/commit/c77d86564686c439aa8e5b9d42a1f8573da771b8).
Its manifest identifies that source and runtime archive SHA-256
`52025e46885ee48ba7cbdc81cb407fbead633fedbbb5f015794ec5e9703dd0fa`.
The candidate window auditor separately pins those identities and harness
`73c786a58cbc48597517b99b3e8eec47c898165f`; this gate record supplies no result
for that window. The optional Action mode remains off by default.
