# Adaptive recording admission (experimental)

`QUOD_VITEST_RECORDING=adaptive` opts into backing off repeated recordings
when changing shared inputs already force every file to execute. The default
remains ordinary recording. This is a miss-cost optimization, not a new
replay rule or evidence of the original 50% window target.

After two consecutive eligible shared-input changes, run subsequent misses
through the existing unrecorded driver path. That path executes every test
without recorder hooks or coverage. It creates no witness, promotes no
record and refreshes none of the ledger's input/result/audit evidence.
Only a separate economic hint is updated. A relevant mutation still executes
and can fail the job; a previous pass cannot replace it.

Try recording after at most seven skipped recordings. If the current shared
fingerprint matches the previous observation, resume recording immediately
and reset the miss streak. This permits recovery when a previously unstable
environment becomes reusable. The fingerprint includes shared key names and
states, and is never used as replay evidence.

Admission is disabled for cold/incompatible ledgers, changed command
arguments, `--full`, explicit `--no-record`, `--expect-report`, explicit
positive audit counts, any pending lineage audit, and any run whose shared
inputs permit normal selection. An adaptive plain run is additionally
required to report zero replays before its hint can be saved. Explicit
`--no-record` retains its existing promise to leave the ledger untouched.

The summary labels `recording_policy` as `adaptive-plain`, `no-record`, or
`record` on the corresponding normal execution paths. Full replay retains
its existing summary. The Cloud report uses the ordinary executed-test
report; no recording savings are added to replay savings.

Four Rust regressions exercise cold admission, repeated misses, bounded
retry, stable recovery, override/incomplete-state behavior, counter
saturation and fingerprint identity. The remote
[binary fixture](../../scripts/bench-vitest/recording-admission-audit.mjs)
must establish actual replay before inducing misses, compare a changed
failing test with plain execution, verify unchanged proof
data during backoff, exercise periodic retry and recovered replay, and
check full/off/no-record/pending-lineage overrides. Its synthetic hint and
pending marker are guard fixtures, not a measured lineage audit.

Updating the admission hint can require a ledger upload. Any performance
claim must include that cost. The existing profile suggests removable
recording overhead, but a passing fixture alone cannot establish a net
improvement. Keep this experimental until a bounded real-workload comparison
shows its cost and results, then gate the packaged Action before changing
the benchmark pin.

## Gated implementation

[Gate 42365187132](https://github.com/pango07/quod/actions/runs/42365187132)
passed at `45393a3776d9007f84bc5c463127a1836fe7c82e`: 126 Rust tests,
92 JavaScript tests, Clippy and the release-binary regressions. The new
fixture completed 23 invocations, including eight adaptive plain executions.
Its changed input failed in both Quod and plain, with zero replay during
backoff. Recording resumed after seven skips and immediately when shared
inputs stabilized; the recovered state then replayed both fixture files.
Full/off/no-record/pending-lineage guards passed. Existing environment,
descriptor and realpath correctness regressions also passed.

The [evidence](recording-admission.json) retains the verified artifact digest
and hashes of all 23 reports. Independent parsing checked every report's
test names and states, the changed failure, zero backoff replays, periodic
retry, stable recovery and override labels. Proof preservation is asserted
inside the release fixture; it is not inferred from equal test output.

The source-only follow-up `f8cada8` applies the gate's formatting and adds a
separate bounded full-suite cost probe. It explicitly primes the economic
hint to avoid three extra full-suite training runs. Its measured path must
preserve every proof and every test result; it is not a naturally warmed
window or an end-to-end cost measurement. The Action and original benchmark
pins remain unchanged pending evidence.
