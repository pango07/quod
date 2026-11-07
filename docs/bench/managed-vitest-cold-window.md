# Managed Vitest: first complete cold window

The [48-job experiment](https://github.com/pango07/ci-bench/actions/runs/42392036476)
and its independent data audit passed. All 24 managed/control archives were
verified against GitHub's archive size and SHA-256 before extraction. The
[complete evidence](managed-vitest-cold-window.json) retains the artifact
provenance, byte checks, original state fingerprints, per-job times and gaps.
This is the earlier accessor policy; subsequent candidates need separate proof.

| Original cold twelve commits | Runner-seconds | Sum of rounded minutes |
| --- | ---: | ---: |
| Checkout/setup floor | 117 | 12 |
| Ordinary baseline | 1,851 | 36 |
| Uncached managed control | 2,167 | 42 |
| Managed with receipt reuse | 889 | 19 |

These are complete job durations, including install, build, test, post work and
retained diagnostic transfer. Queue and the benchmark coordinator are excluded.
Rounded minutes are `sum(ceil(each job's runner-seconds / 60))`, a model of paid
usage. The actual GitHub runner charge for these public repositories is **$0**.

Against the fresh ordinary baseline, replay saved **51.97% of runner time** and
**47.22% of modeled minutes**. Against the earlier 1,735-second ordinary window,
it saved **48.76% of runner time**. The conservative runner bar requires at most
half of both ordinary measurements: 867.5 seconds against that earlier reference.
The observed 889 seconds misses it by 21.5 seconds. The 18-minute modeled target
is also unmet. Excluding the cold first job gives 622 seconds/14 minutes for
c1–c11; that diagnostic does not replace the required cold-window result.

There were **11 authenticated partial candidate hits**, one cold managed
execution, zero fallbacks and zero full-job hits. The floor-plus-8-second
full-job goal remains unmet. C1 took 46 seconds versus its 8-second floor;
the native remainder alone took 22.215 seconds. Its complete 26-second test step
could hypothetically disappear and the measured outside-test work would still
be 20 seconds, above that 16-second target. Broader remaining-test/release effect
coverage and outside-test overhead need investigation; this profile proves no
such capability or future speedup.

The 12 uncached controls execute the same managed contract without any receipt
transport. Their complete candidate responses equal the selected replay responses
at every commit. All original Vitest full names and statuses match the original
manifest and fresh ordinary jobs: 120 file/project instances and 4,129 tests at
c0–c6, 4,130 at c7, and 4,131 at c8–c11, including the original skip. Across a
chain, all 49,557 states remain represented. The original property file's 47
tests, three seeds and 24 cases per seed remain unchanged. The later native
release-test command also runs and its results match ordinary execution.

The native adapter retains exactly 119 remaining project/file instances per
job, including repeated paths under the Zod project aliases. It never reruns the
managed candidate. Changes elsewhere can add tests without invalidating that
unchanged candidate: the c7/c8 state-count increases are preserved. The first
cold job records a receipt and receives positive SDK cache ID `10167458518`.
Every later hit's exact authenticated service key supplies the digest of those
same original worker response bytes. No cache-payload manifest supplies trust.

Reporter fidelity remains incomplete. Managed JSON recomputes suite counters
as file-instance counts (120), while native Vitest includes nested suites (802
for the full workload). It preserves the correct assertion counts, full names,
states and the complete native remainder rows, but flattens 36 managed assertion
ancestor lists and omits managed duration/file-time fields. The evidence retains
the concrete raw comparisons. Zero disagreements here means **test names and
states**, not complete JSON metadata compatibility.

The measured pins are Action
[`d79c8635`](https://github.com/pango07/quod-action/commit/d79c8635f4dbc93c842ec1da8bd5ee6bf8e72928),
runtime
[`2a4d6f2f`](https://github.com/pango07/quod/commit/2a4d6f2f7547dd00e720a32d5b451229784ba8fb),
and harness
[`82f59245`](https://github.com/pango07/ci-bench/commit/82f59245d12162ce1d998f0bc4eb068db38acc3d).
The native-remainder controls passed in
[42391903760](https://github.com/pango07/quod/actions/runs/42391903760).
The existing ordinary Quod result remains separate in
[its original evidence](convex-helpers-current-cloud-window.md). The optional
profile remains experimental and off by default; its [contract](../managed-vitest-action-v0.md)
does not assert general native Node equivalence.
