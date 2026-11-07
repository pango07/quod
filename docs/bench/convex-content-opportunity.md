# Original window: content reuse opportunity

This is a Git-history and recorded-module audit, not a replay result. The
original cold-window target remains unmet. No production cache key or test
behavior changed.

The [full measured window](convex-helpers-current-cloud-window.md) contains
1,538 default Quod runner-seconds versus 1,735 plain, with no test or unit
replays. It needs another 670.5 seconds removed to reach half of plain.

## What actually changes

The verified worker-input artifact from
[run 42169609242](https://github.com/pango07/quod/actions/runs/42169609242)
contains five distinct tracked source paths served to the
`stream.property.test.ts` worker: the test itself, `stream.ts`,
`rowLevelSecurity.ts`, `filter.ts`, and `setup.test.ts`. Matching their
fingerprinted absolute paths against the public repository's tree resolves
every transformed module record for that worker. Each of those five files
has the same Git blob and mode at every commit in the original 12-commit
window. There is a real source-content stability opportunity.

This observation is deliberately limited: the worker capture is at c11,
not a dependency trace at each earlier commit. Its five external-module
records are retained but their transitive package internals are not expanded.
It is not a complete input closure or permission to replay.

| Transition | Changes relevant to a coarse manifest/lockfile key |
| --- | --- |
| c1 | None; changelog changes |
| c2 | Root scripts, including adding `test:release` to `npm test` |
| c3 | Root package version |
| c4 | Root package exports |
| c5 | Nested package manifest removed |
| c6 | Root version and lockfile root entry |
| c7 | None; OpenAPI implementation/test and changelog change |
| c8 | None; OpenAPI implementation/test and changelog change |
| c9 | None; CORS implementation and README change |
| c10 | Chalk update and associated nested dependency placement |
| c11 | Commander update |

The exact comparison set includes all tracked package manifests,
`package-lock.json`, `tsconfig.json`, Vite/Vitest configuration paths and
`scripts/zodVersions.mjs`, as well as the five observed source paths. That
set is unchanged on only c1, c7, c8 and c9. It is an explicit coarse analysis
scenario, not an implemented key or a claim that these are all relevant
inputs. Root-manifest field names are reported to explain changes; no fields
are excluded from production unit keys. The earlier decision against
per-field package.json keying for npm units remains intact.

## Why this changes the next step

The four transitions account for 306 seconds of plain test steps and 372
seconds of recorded Quod test steps. Even an arithmetic scenario removing
all 372 seconds, with every other observed duration fixed, leaves:

| Scenario | Runner-seconds | Ratio to plain | Modeled minutes |
| --- | ---: | ---: | ---: |
| Measured plain | 1,735 | 1.000 | 36 |
| Measured default Quod | 1,538 | 0.886 | 32 |
| Remove all four entire test steps | 1,166 | 0.672 | 27 |
| Required original runner-time target | 867.5 | 0.500 | Calculated per job |

Modeled minutes round each adjusted job separately. The scenario's 32.8%
runner-time reduction corresponds to 25% fewer modeled minutes, not 32.8%
less money. These public jobs have no GitHub runner charge.

The scenario omits warm-up, audit, validation and transfer costs. It is not
a performance forecast or a general upper bound: scheduling, overlap,
additional optimizations and runner variance can change whole-job timings.
It does establish that merely applying a broad source/manifest key and
expecting those four test steps to close the measured gap is insufficient.

A useful prototype must address both dimensions: safe handling of changing
host metadata, and a precise dependency boundary that can distinguish
unrelated manifest/tool changes from actual test inputs. A managed runtime
with an all-repository key would not automatically solve the latter.
Likewise, dropping manifest fields or metadata without that boundary would
weaken correctness. The next design must explain how relevant consumed
inputs are established before investing in another implementation.

## Reproduction

The [audit script](../../scripts/bench-vitest/audit-convex-content-opportunity.py)
reads Git trees/blobs and the existing worker artifact; it does not install
dependencies, build software or run tests. From the Quod root:

```sh
python3 scripts/bench-vitest/audit-convex-content-opportunity.py \
  /path/to/convex-helpers-clone \
  /path/to/verified-worker-artifact/before-inputs.json \
  /tmp/convex-content-opportunity.json
```

The input artifact SHA-256 is checked against the existing committed worker
evidence before use. [The generated evidence](convex-content-opportunity.json)
retains the twelve exact commits, all compared blob IDs/modes, changed paths,
module records, source digests, arithmetic and limitations. The Git clone
must contain the requested public objects; a partial clone may fetch them.
