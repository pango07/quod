# Convex-helpers: Addendum E implementation and evidence

Experimental work on `exp/whole-job-e-codex`, starting from `c0f55ad` and
including the fail-closed audit fixes described in `convex-helpers-audit.md`.
The real 12-commit window remains pinned at
`5eaa7e5160736054d53f8b71e2c522835eb427e0`; cold-start jobs count.

## Replay changes

E1 is opt-in with `QUOD_VITEST_LINEAGE=1`, exposed as the action's
`lineage-witness` input (off by default, on in this benchmark). Changed
inputs still execute under recording. A plain-confirmed predecessor can
supply the extra witness only when the file's input names and per-test
states agree, the new result passes/skips, tools and arguments agree,
and the recording has no near-timeout, network use, or quarantine.
Shared inputs still force fresh execution; the comparison is of the
per-file record, as specified in Addendum E, rather than the tools'
per-run temporary-file bookkeeping.

A deterministic hash sample, rounded up to one in eight pending records,
runs plain on the next eligible job. Agreement clears its pending status.
Disagreement quarantines that file and revokes the entire promotion
stamp, including already-audited peers. Turning the switch off requires
plain confirmation of pending lineage records. Render shortcuts cannot
bypass pending audits. A plain run under changed toolchain facts drops
the old record rather than carrying its trust into the new toolchain.

Consecutive changed commits cannot inherit indefinitely: an E1-promoted
record has `plain_n=0`, so it cannot supply another lineage witness until
it has passed a plain audit. The original savings estimate assumed more
consecutive promotions than this rule guarantees. Measurement decides.

E2 lets traced units confirm other records with exactly the same input
names, across any changed input states. Exit, output files, stdout, and
stderr must agree under the existing pairwise masks. Version banners are
still significant; a changed read set never qualifies. Regression coverage
includes a real strace/bubblewrap execution across changed file contents.

## Fixed overhead

The old cold-job log contradicted the initial diagnosis: the ledger upload
was about 0.3 seconds; npm's download cache and node_modules were saved
sequentially and occupied most of the seven-second post step. The action
now awaits all independent cache saves concurrently, with separate failure
handling. There is no upload left running after the post step exits.

The dependency marker is written after successful npm installation, before
build scripts run. Fresh and restored node_modules trees therefore expose
the same marker filename. Previously post added the marker only after the
build, changing the directory listing seen by the next job.

## Short profiles

[Profile run](https://github.com/pango07/quod/actions/runs/42081702707),
`stream.property.test.ts` at c4, one plain/recorded pair per runner:

| recorder setting | plain s | recorded s | ratio |
|---|---:|---:|---:|
| all observers | 57.48 | 57.83 | 1.006 |
| preload only | 78.75 | 79.76 | 1.013 |
| environment only | 85.83 | 88.06 | 1.026 |

These are diagnostics, not cost benchmarks. Disabled observers never
produce reusable records. One pair per setting does not estimate a stable
mean. The isolated test does not reproduce a 25-second recorder penalty;
a full-suite CPU profile checks whether other workers' recording causes
contention.

## Verification

[Initial E gate](https://github.com/pango07/quod/actions/runs/42080266190)
and [corrected E gate](https://github.com/pango07/quod/actions/runs/42082394091)
passed CLI unit tests, recorder regressions, the syscall integration test,
and clippy. The first real action regression correctly failed when E1
never promoted; the implementation was corrected before benchmarking.

The benchmark harness rejects missing jobs/logs, mismatched test-line
multiplicities, differing job conclusions, and lineage disagreements.
It records runner images, lineage counts, whole-window and post-cold-start
cost, rounded minutes, and hit overhead beside the matching empty job.
A Vitest replay with a unit still executing is labeled a partial hit.

No new window result is claimed here until the shipped-action regression
and the 12-commit measurement have completed.

The [full-suite CPU profile](https://github.com/pango07/quod/actions/runs/42083296107)
measured 77.44 seconds plain and 95.15 seconds recorded with CPU profiling
active on the recorded side. That ratio includes profiler cost. Across
all profiled processes, Vite's `wrapCallSite`, `prepareStackTrace`, and
`cloneCallSite` consumed 20.94 sampled CPU-seconds; Quod's `toolCaller`
consumed another 3.98 seconds.

The resulting experiment memoizes caller attribution for a complete raw
stack within a synchronous turn. It retains the existing source-mapped
classification on the first encounter; it does not cache environment
values or drop observations. Script identity, location, formatter identity,
and npm context participate. A microtask boundary clears the cache.
Missing native frame support or a truncated stack takes the old full-stack
path. [Node's native call-site API](https://nodejs.org/api/util.html#utilgetcallsitesframecount-options)
provides script identities without invoking `Error.prepareStackTrace`, with
a maximum of 200 frames; hitting that limit deliberately disables caching.
A separate full-suite cached/uncached pair compares every test name and
state without the CPU profiler before this optimization can ship.

The paired full-suite cache experiment ([42085242039](https://github.com/pango07/quod/actions/runs/42085242039)) measured 88.511 s uncached versus 82.298 s cached on one runner, a 6.213 s / 7.0% reduction. All individual test states agreed. One pair is directional evidence, not a population claim. The matching Rust regressions, unit integration test and clippy passed in [42085237197](https://github.com/pango07/quod/actions/runs/42085237197).
