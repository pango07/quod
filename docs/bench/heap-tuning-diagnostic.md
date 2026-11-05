# Heap tuning did not improve the convex-helpers recording cost

[Run 42129211176](https://github.com/pango07/quod/actions/runs/42129211176),
probe `c99bd32`, ran the complete c4 suite four times per mode, in the order
default / 64 MiB semi-space / 64 MiB / default. Plain and recorded modes had
separate runners; compare settings within a mode, not their absolute times
across runners. All recorder observations were enabled, every execution was
fresh, and no reusable ledger was uploaded.

| Mode | Default mean seconds | 64 MiB mean seconds | Change |
|---|---:|---:|---:|
| Plain | 98.350 | 98.980 | 0.6% slower |
| Recorded | 103.297 | 103.059 | 0.2% faster |

There is no useful measured gain. The flag is not adopted. These timings
are test-command timings, not job totals, and add no savings to the window.

Both runners used Node 24.18.0, four CPUs, approximately 15.6 GiB RAM, AMD
EPYC 7763, and image `ubuntu26/20261101.186.1`. The pinned corpus was
`6b17e1514f5222e9c019e177e36384df95a3ff25`, binary `ae52750`, packaged Action
`8b0ad91`. GNU time's maximum process RSS ranged from 452–460 MiB plain
default to 471–550 MiB plain tuned, and 492–524 MiB recorded default to
518–526 MiB recorded tuned. This is not aggregate peak worker memory.

All eight executions' 120 file/project instances and 4,129 individual test
states agree after normalizing plain ancestor titles into Quod's full test
titles, retaining duplicate instances. This establishes agreement for this
experiment, not a general correctness guarantee. A separate
[metadata regression](stat-metadata-regression.md) found a real stale replay
in the same packaged binary and takes priority over performance work.

The experiment follows Node's documented workload-dependent tradeoff:
[a larger semi-space may improve throughput at a memory cost](https://nodejs.org/download/release/v24.4.0/docs/api/cli.html#--max-semi-space-sizesize-in-mib).
