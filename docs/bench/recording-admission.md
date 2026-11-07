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

## Unchanged real-suite cost check

[Gate 42365479228](https://github.com/pango07/quod/actions/runs/42365479228)
passed at `f8cada81cf3dfdba4273cc83d6fc127d76338194`, using original
convex-helpers c11, Node 24.18.0 and image `ubuntu26/20261101.186.1`.

| Phase, in execution order | Command seconds |
| --- | ---: |
| Plain before | 68.039 |
| Recorded | 70.554 |
| Adaptive plain | 68.114 |
| Plain after | 67.316 |

All four phases executed 120 file/project instances and produced 4,131
matching test states: 4,130 passed and one skipped. Both Quod phases
executed every file and replayed zero. The adaptive phase retained exactly
the previous proof data; independently checked canonical proof snapshots
are byte-identical, SHA-256
`549b166cc3000a64ac539cd2dbafa419af46863ea2dc7a8503dd70a6bbc69217`.
The actual binary SHA-256 was
`6aafd60f655dd902cc4cc25bcd743d63678141e0afdc31f85dd82fc0415015a8`.

Before the adaptive phase, 38 recorded source paths had only their mtimes
advanced by 60 seconds. Their content hashes were checked unchanged, and
the final tracked-source diff was empty. The economic hint was explicitly
primed; this is not natural training or the original cold window. No input,
witness or result evidence was primed or relaxed. Full raw reports retain
duplicate file instances, and independent parsing checked all names/states.

Adaptive execution was 2.440 seconds (3.46%) shorter than the recorded phase
and 0.436 seconds above the mean of its plain controls. This is one
fixed-order comparison, not a causal population estimate or whole-job cost
saving. Installation, Cloud and upload costs are excluded. The verified
[evidence](recording-admission-cost.json) retains those limits, report hashes,
source mutations, proof hash and artifact identity (`13571965337`, 625,630
bytes, SHA-256 `9bf1f7fe049c7308b75fed52f9f3cfc6cd43237e12d23004a95a084828d303bc`).

This is a small measured miss-path improvement. It does not justify claiming
40–50% savings or spending another full-window run on recording admission
alone. Keep the feature opt-in in source; the Action and benchmark pins
remain unchanged. Useful fresh-runner test replay remains the main unmet
requirement.
