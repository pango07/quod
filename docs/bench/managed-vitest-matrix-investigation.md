# Managed Vitest matrix investigation

The [passing corrected gate](https://github.com/pango07/quod/actions/runs/42406036816) executes **108 of the original 120 project/file instances** inside fresh managed processes. Their **3,947 original test names/states (95.55% of 4,131)** agree exactly with the complete native control. The independent data-only audit passes with zero issues. All 81 Zod instances pass; 27 of 37 root instances pass. This correctness probe enables no receipts or cache hits and establishes no benchmark savings.

Frozen source: `ff279ea5b59b359d26985f08f22e0749f9eccbde`. Artifact `13587034973`: 36,186,785 bytes, `sha256:f8e20f3a68cc3fbec7ce01a5ea90e8b50a1d558cbf08146a7fb049e242adc282`. The [machine-readable report](managed-vitest-matrix-investigation.json) retains the complete supported roster, decline reasons, original fingerprints, source provenance and independent audit.

| Actual project | Selected instances | Managed instances | Matching test states |
|---|---:|---:|---:|
| example-ui | 2 | 0 | 0 |
| zod-4.0 | 9 | 9 | 418 |
| zod-4.1 | 12 | 12 | 470 |
| zod-4.2 | 12 | 12 | 470 |
| zod-4.3 | 12 | 12 | 470 |
| zod-4.4 | 12 | 12 | 470 |
| zod-4.5 | 12 | 12 | 470 |
| zod-4.6 | 12 | 12 | 470 |
| convex-helpers | 37 | 27 | 709 |

All 120 native instances execute, with 4,130 passing and 1 skipped state. The original 47 property tests retain their source bytes. Every managed instance has a fresh workerd process, a closed compiled module graph, exact project alias/context and sealed completion. Supported managed failures remain sticky. Both 31-check boundary preflights, all 40 original compiler regressions plus 15 additional compiler controls, alias/context negatives, planner negatives and path controls pass.

The extension preserves every glob key and admits only the actual test module's own lazy loader. Foreign targets remain sticky-denied. A wider [intermediate gate](https://github.com/pango07/quod/actions/runs/42405153156) correctly failed: bundling renamed Zod class constructors in eight instances, changing 32 assertions. Explicit `keepNames: true` plus genuinely colliding class/function fixtures fixed those observable names; all eight instances now agree with native.

The remaining 12 ordinary instances retain 184 native states: two DOM files, four native-dependent compile declines, two `reportError` cases, and one each requiring Request/foreign lazy loading, environment mutation, timer mutation and Blob. No test is omitted. Legitimate duplicate test names and the all-skipped file remain visible.

The sequential investigation took 474.216s, including 132.918s of successful compilation, 158.951s in all worker phases and 100.019s for the complete native control. The native control also took substantially longer than in the earlier 74-instance run, so cross-run changes cannot establish a compiler performance effect. A separate paired compiler experiment compares exact bundle bytes and complete identities on the same runner.

The [earlier corrected run](https://github.com/pango07/quod/actions/runs/42404007739) independently established 74 instances and 3,369 states. The initial preparation failure and the intermediate constructor-name failure remain in the evidence history.

The original twelve-commit cost target and genuine floor+8s full-job hit remain open. Separate gates cover the v2 receipt ledger, authenticated collection transport, finite npm installation equivalence and native-command isolation; each requires its own evidence before integration.
