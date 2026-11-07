# Managed Vitest matrix investigation

The [corrected gate](https://github.com/pango07/quod/actions/runs/42404007739) executes **74 of the original 120 project/file instances** inside fresh managed processes. Their **3,369 original test names/states (81.55% of 4,131)** agree exactly with the complete native control. The independent data-only audit passes with zero issues. This expands the previously admitted 47-test single instance; it does not enable new receipt admission or establish benchmark savings.

Frozen source: `166bd2de80bcbf1153094a06380e8c1e3675d0f1`. Artifact `13586299886`: 33,827,872 bytes, `sha256:8f54f906cf3cbecfcf0deb64a95b17168ad5116673552f697b0fa0e698f752e7`. The [machine-readable report](managed-vitest-matrix-investigation.json) retains the exact supported roster, decline reasons, project-labelled native fingerprints and audit result.

| Actual project | Selected instances | Managed instances | Matching test states |
|---|---:|---:|---:|
| example-ui | 2 | 0 | 0 |
| zod-4.1 | 12 | 8 | 400 |
| zod-4.0 | 9 | 6 | 352 |
| zod-4.2 | 12 | 8 | 400 |
| zod-4.3 | 12 | 8 | 400 |
| zod-4.4 | 12 | 8 | 400 |
| zod-4.5 | 12 | 8 | 400 |
| zod-4.6 | 12 | 8 | 400 |
| convex-helpers | 37 | 20 | 617 |

All 120 native instances still execute, with 4,130 passing and 1 skipped state. The original 47 property tests retain their source bytes. Every managed instance has its own workerd process, closed compiled module graph, exact project alias/context and sealed completion. Supported managed failures remain sticky. Both 31-check boundary preflights, 40 original compiler regressions, 5 alias/context negative checks, 2 planner negatives and 6 path controls pass.

The remaining 46 instances comprise 34 self-loader-only declines, 2 mixed lazy-loader/capability declines, 2 reportError declines, 1 timer-write decline, 1 Blob decline, 4 native-dependent compile declines and 2 DOM instances. Their complete native evidence remains visible.

The initial [failed preparation run](https://github.com/pango07/quod/actions/runs/42403530445) preserved the full native roster but admitted zero instances because the alias canonicalizer expected the wrong Vite filesystem-URL prefix. The correction keeps exact path boundaries and adds six positive/negative path controls.

The investigation took 246.164s sequentially, including 76.050s of successful compilation, 75.237s in worker phases and 45.861s for the complete native control. These are diagnostic phase measurements, not full-job acceleration claims.

The original twelve-commit cost target and genuine floor+8s full-job hit remain open. The next experiment admits only the actual test module’s own lazy loader, while retaining all other glob keys and denying their execution. Install equivalence and native-command closure are tested independently.
