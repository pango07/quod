# Native incremental execution

Experimental opt-in action: `.github/actions/incremental`. It preserves
TypeScript's dependency graph and optionally Next.js's build cache between
fresh runners. The native tools still inspect inputs, determine invalidation,
and execute every original command. No result replay or skipped test is claimed.

**Decision: functional milestone passed; keep opt-in and experimental.**
Native compiler/build work is reduced, but the full-job trials do not establish
dependable net savings. Do not enable it in routine CI or Class-management on
the strength of this experiment.

```yaml
- uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1
- uses: actions/setup-node@949feb2413d6458794dcd2491c4babbbce0c15c1
  with:
    node-version: '24.22.2'
- uses: pango07/quod/.github/actions/incremental@93119cf8fa9ed1575cea59d393652f2f8849137c
  with:
    working-directory: cloud
    next: 'true'
# Keep the existing install, lint, typecheck, test and build steps.
```

The TypeScript adapter supports an npm project with a JSON tsconfig,
`incremental: true`, `noEmit: true`, and the default build-info location.
Extends, project references, composite projects, custom output locations,
JSONC and other package managers bypass this first adapter. Next is explicit
opt-in and supports the default `.next` directory. Configuration files are
hashed as data, never imported. Only `tsconfig.tsbuildinfo` and optionally
`.next/cache` are transported, not node_modules or final build outputs.

The small planning step reads bounded package/lock/config descriptors. It
does not scan source files, rehash dependency trees, install Quod's sandbox,
or intercept execution. A versioned key separates Node, OS, architecture,
package lock, configuration, project and adapter selection. The commit suffix
permits a new state to be saved after source changes. A compatible older
cache is a native compiler hint, not authority to accept the previous result.
Preexisting state and symlinked destinations decline restoration.
The optional `generation` input defaults to the job's commit SHA; workflows
that check out a different revision can supply that commit or a content digest.
It changes the save key, not the compatible-state prefix or native validation.

Transport is GitHub Actions cache with its repository/branch access rules.
No Cloud signing, API, production deployment or Class-management change is
part of this milestone. Existing native caching should remain enabled in
both benchmark arms. Cache setup, downloads and post-job uploads count as
overhead; a warm command is not a whole-job saving.

Validation: dispatch `gate.yml` with `only=incremental`, at the reviewed
branch and exact source SHA. First, actual compiler controls compare restored
and clean state on unchanged inputs, implementation changes, changed imported
types, added erroneous files, deleted dependencies, generated type changes and
compiler configuration changes. Every diagnostic and exit status must agree.
Then three matched pairs run all real Cloud commands on independent runners:
cold state (which also seeds Quod's cache), warm state and changed root-layout
metadata. Both changed builds must contain the new metadata in their server
output, guarding against a stale production build. Cache namespace is unique
to that workflow run. No same-runner replay
or repeated warmup commands occur in the matched jobs. The workflow waits for
the seed job's post steps before starting consumers.

The final gate uses the externally downloaded action pinned at `93119cf`,
with byte-identical action sources in the validated checkout. Plain jobs have
separate step definitions and never download that action. The changed-source
case requests a distinct cache generation, so it restores compatible prior
state and pays for saving the updated state in post steps. This includes
distribution and refresh costs that a local-action, exact-key-only test misses.

Acceptance: all controls and full app checks pass; real cache transport works
on fresh runners; full-job measurements, including the initial learning/save
cost, justify further trials. Three pairs are an initial decision experiment,
not evidence of a general percentage improvement. A slow or inconclusive
result keeps the action experimental. Routine CI remains native.

## Results, 2026-11-08

The final [gate 42522891752](https://github.com/pango07/quod/actions/runs/42522891752)
passed at source `a13441cddc6cf55e696e93e33bb45c5d6d9e3c40`, attempt 1.
Its action is externally fetched at the exact pin in the example above;
the validated source contains byte-identical action files. Six planner
boundary tests and seven actual TypeScript differential controls passed.
Each of the six full pipeline jobs passed all 496 Cloud tests, lint, route
generation/typecheck and production build. Both changed-source builds
contained the new metadata. The changed Quod job restored the original cache
and successfully saved a distinct updated generation after completing.

Complete GitHub job lifetimes, including preparation and post steps:

| State | Plain | Quod | Observed difference |
| --- | ---: | ---: | ---: |
| Cold, first save | 126s | 126s | tied |
| Warm, independent runner | 102s | 75s | 27s shorter |
| Changed layout, updated save | 104s | 118s | 14s longer |

The sum is 332s plain versus 319s through the action. This small sequence is
not a causal estimate or a general savings rate. Controls and baseline jobs
are additional research compute, not part of a user's normal execution.
The cold pair used EPYC 7763 on both sides. The warm plain runner was EPYC
9V74 and Quod's was EPYC 9V45. Both changed runners reported Xeon 8573C, but
installation, lint and test execution also differed. All had four logical
cores, Node 24.22.2 and runner image 20261101.186.1. Matching a model name
does not eliminate hosted-runner variability.

Within the final warm pair, route generation/typecheck took 6.245s plain
versus 1.969s with restored state; production build took 14.273s versus
4.016s. In the changed pair the respective timings were 6.148s versus
3.012s and 13.816s versus 5.677s. These phase reductions are not substitutes
for the complete-job numbers above. The planning step read 295,118 bytes
of descriptors and took 5.2–10.8ms; that excludes Node/action startup and
transport. The native cache archive was approximately 57MiB.

Earlier validation evidence is retained, including negative outcomes:

| Gate / source | Cold plain → Quod | Warm plain → Quod | Changed plain → Quod |
| --- | ---: | ---: | ---: |
| [42521942786](https://github.com/pango07/quod/actions/runs/42521942786) / `3d0d1cc` | 135s → 88s | 132s → 89s | 126s → 90s |
| [42522275039](https://github.com/pango07/quod/actions/runs/42522275039) / `a5b16b6` | 131s → 138s | 114s → 122s | 124s → 112s |

Both passed the full checks. The first changed-source case added an unused
TypeScript file and lacked hardware metadata; the second checked a real
layout change. Both used the local action and an exact cache key for changed
source, so neither included external distribution or updated-cache publication.
The second three-pair sequence was 3s slower overall. The first's large cold
difference, before reuse could occur, is evidence against attributing its
whole observed advantage to caching. None of these results is discarded.

This establishes a working layer that avoids rebuilding native compiler
state across fresh runners without introducing Quod's observer preparation.
It does not establish generic task replay, test selection, cheaper dependency
installation or Cloud result transport. Installation still consumed roughly
29–49s in the final jobs, and tests roughly 24–35s. Those costs now dominate;
more optimization of this 5–11ms planner would not materially change them.
