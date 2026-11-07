# Adjacent full-fence batch diagnostic

This unwired compiler experiment changes no dispatcher, cache lookup or receipt
admission. Its remote gate must pass before any integration is considered.
The original compiler remains byte-exact `a87f1f8fafea2e0f1b4a1e299199f5581172a2fe90f59882d3c64eb50f68acaf`.

`compileAndCaptureCertifiedBatch({project, adapterRoot, requestsJSON})` accepts
primitive root strings and a canonical JSON string containing at most 512
`{entry: 'runner.mjs', options, testFile}` requests. The request string has a
16 MiB cap. Caller accessors, callbacks and unknown request fields decline.
The existing strict single compile/capture/validation APIs remain available.

The batch reserves the certified process before its first await. Any public
initialization, compile, capture, validation or proof read already in flight
excludes the reservation; those public operations cannot enter a reserved
batch. There is no caller hook, guard-disable option or public reset.
Malformed input rejected before reservation does not poison the process.
A reserved failure poisons subsequent certified use and returns no pending
build, certificate or private provenance.

Each internal compile and capture is surrounded by a complete tool/API/Node/
environment guard. Only adjacent guards inside the closed sequence are shared:

1. Before the raw constructor.
2. After construction, before compile 0.
3. After each compile, before its capture.
4. After each capture, before the next compile or final publication.

This is `2N+2` full fences including construction: 26 for twelve requests and
218 for 108. Initial bootstrap and initialization observations are additional.
Every fence also checks the complete current CJS module/exports roster against
the reserved roster. The last capture rechecks every earlier full source-byte
observation and glob set. All results and WeakMap build issuance stay private
until the final full guard; no await or caller callback precedes publication.

The trust boundary is the reviewed builtin bootstrap, running Node, pinned
compiler/esbuild/TypeScript implementations and trusted controller. Workspace
observations are sampled before and after work. Active adversarial edits,
arbitrary same-process interference and complete filesystem atomicity are not
claimed. No project test source runs inside the producer interval. Later
receipt reuse still requires independent authenticated receipt authority and
fresh current-input validation.

The manual `gate.yml` selector `benchmark-managed-batch-certificate` preserves
all 55 compiler, 213 strict certificate, 13 dispatcher cases/142 assertions,
18 proof and 152 ledger controls. It executes all original native 120 instances
and current managed 108, compares original project-labelled assertion and API
file states with the independently verified ff baseline, and retains both raw
sides of the twelve representative and full 108 bundle/identity comparisons.
It validates all 108 current certificates through the fresh resolver and checks
their opaque proofs against complete execution identities. Fresh-process
negative cases cover cold unknown tools, prior operations, reentry, late
failure, API/environment/module-cache mutations and source/resolution/glob
changes. Failed controls remain failed; ordinary execution cannot erase them.

The diagnostic records actual strict12, batch12, batch108 and shared validation
times separately. Additional proof, recapture and negative-control verification
work is outside those batch timers. Existing source-tree caching was already
enabled by default. No timing saving, production acceleration, complete-command
hit or twelve-commit benchmark result is inferred from static fence counts.
Three separate evidence roles retain finite per-role limits of 500,000,000
expanded bytes, 32 MiB per file and 20,000 total paths, including the inventory.
