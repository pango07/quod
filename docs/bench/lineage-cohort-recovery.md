# Recover the whole affected lineage batch in the detecting job

The old packaged engine could detect a lineage audit disagreement and revoke
the batch for future jobs while still reporting a stale passing result from
another member in the current job. Comparing only process exits missed this:
both Quod and plain exited 1, but one per-test state differed.

[Baseline gate](https://github.com/pango07/quod/actions/runs/42367339583)
reproduces that recovery defect with packaged engine `c12a7fe` and Action
`beb12a8c9c32500ad2d19a87b10ee7cc7a95c197`. The fixture deliberately removes
one input dependency from each of two genuinely promoted records, then
changes that input so both tests fail. This is synthetic fault injection,
not evidence that the recorder naturally omits that dependency. Only those
two dependency vectors change; all other ledger bytes remain identical.

One member has already passed its scheduled plain audit. Its retained
promotion stamp must still associate it with the faulty batch.

| Detecting job | Wrong per-test states | Revoked records | Final results |
| --- | ---: | ---: | --- |
| Old packaged engine | 1 | 2 | One stale pass, one failure, independent control passes |
| Corrected release engine | 0 | 2 | Both affected tests fail; independent control still replays |
| Corrected packaged Action | 0 | 2 | Same actual results and independent replay as the release fixture |
| Recovery process deliberately interrupted | Not reported | 2 | Exit 1, summary exit 1, combined report removed |

The correction saves revocation before launching recovery, reruns affected
replays plain in the detecting job, and merges the actual results before
publishing the combined report. It preserves the independent control's
trust. Recovery grants no new witness for the faulty read set, and a new
record cannot inherit trust from the revoked promotion stamp. Both failing
cohort members are quarantined after successful recovery.

If recovery cannot finish, the command returns failure and removes the
potentially stale report. This failure case has no final per-test comparison;
it must not be described as `wrong = 0`.

[Source and release gate](https://github.com/pango07/quod/actions/runs/42368026811)
passed for `024b9290a708e0e4e7d9c5dd1133664b8cd9a0a7`: 128 Rust tests,
92 JavaScript tests, Clippy, existing release regressions, and both new
recovery scenarios. The fixture uses the binary's embedded recorder, with
no source override. All scenarios use Node 24.18.0 and runner image
`ubuntu26/20261101.186.1`.

[Action gate](https://github.com/pango07/quod/actions/runs/42368399833)
passed for Action source `1892bdbe79f1a594ae0eb799ec0b3e9ba80372e4`,
including its 14 unit tests, existing packaged regressions and both new
recovery scenarios. The binary was loaded from the shipped archive. Its
stripped ELF hash matches the independently verified release archive.
The remotely compiled bundle is committed as Action
`7a84de3363d18226080f37a9eea9ebd103210c5d`; benchmark commit
`9fde6f331a426113cfbc191f75ec1214e7aa6cfb` changes only the Action,
version and archive pins. No full window was launched for this safety fix.

The [evidence manifest](lineage-cohort-recovery.json) retains artifact,
report, ledger and binary hashes. Independent analysis compares raw test
names/states and multiplicity, verifies byte-preserving injection, checks
that revocation includes the already-audited peer, and checks that unrelated
trust survives. The release fixture uses the unstripped binary; packaging
has a separate stripped binary hash and requires its own Action gate.
The [independent verifier](../../scripts/bench-vitest/audit-lineage-cohort.py)
accepts an extracted scenario directory and `stale`, `correct` or
`recovery-failure`. It was run against the baseline, release and packaged
Action artifacts. The packaged scenarios reproduce both the corrected
per-test results and the report-suppression failure path.

The earlier [signature-only fixture](lineage-revocation.md) remains useful
for next-invocation revocation. This fixture adds recovery of changed actual
results within the detecting invocation. Fast rendering, narrowing and
opportunistic audits are disabled to isolate this E1 path. It is not a
fresh-runner performance experiment or a universal correctness proof.

The original convex-helpers window and its savings target remain unmet.
This change closes an identified safety defect; it authorizes no additional
replay and changes neither the default lineage switch nor the benchmark.
