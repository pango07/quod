# vitest critical path, run phase (`exp/vitest-turbo`)

2026-11-02. Built on exp/vitest-shadow (critical path `quod vitest
--no-record`, recording in a shadow job). Selection and narrowing are
unchanged; this branch is about the critical path's run phase and the job
around it.

## Where a small run's time goes (router b810a7e, hosted runner, 4 CPUs)

`scripts/bench-vitest/breakdown.sh` (ci-bench mode `breakdown`): plain
vitest on 1/4/10 files with a timing reporter, variants, CPU profiles, and
the critical path itself.

| plain vitest, 1 file (useLink.spec.ts) | ms |
|---|---:|
| node start, `import vitest/node`, createVitest (until `onInit`) | 426 |
| pool start, fork, worker boot (to the file's start) | ~120 |
| environment (jsdom) | 257 |
| collect (imports through vite's module runner: transform plus eval) | 310 |
| the tests | 46 |
| report, close, exit | ~40 |
| **total** | **1220** |

Main process CPU over that run: 643 ms idle (waiting on the worker), vite
158, module compilation 131, rolldown (transforms) 67, vitest 58.

**The critical path's floor on router is not Quod's.** With nothing changed,
`quod vitest --no-record` takes 2.83 s. Two files always run:
`src/unplugin/core/RoutesFolderWatcher.spec.ts` (unrecordable: fs.watch;
its test "triggers when new pages are added" waits 2026 ms of real time)
and `codegen/generateParamParsers.spec.ts` (failed when recorded). Every
router commit's critical path is ≥ ~2.5 s because of the first. Making it
replayable is a recording rule (agent A). The router tables below are also
run with both files excluded (`--exclude`, for plain and Quod alike) so the
run phase's gains are visible.

## What changed, and what each bought

1. **Rendered full replay** (kept). When every test file replays, the
   critical path no longer starts vitest: it emits what vitest printed and
   wrote for a full replay of exactly this recording. The shadow makes it,
   right after it records, by running the driver once more with every file
   replayed and the reporters' files redirected (`render` mode; ~200 ms,
   off the critical path). Emitted only when all of these hold: every
   test file vitest found at the record is recorded and replays now; the
   render is of these results (`render_sig`: ids, results, states, record
   counts); vitest's arguments are identical; the same quod (driver hash),
   node (`node --version`), platform; and the workspace's paths (every name
   but node_modules/.git/the ledger dir, not contents) are the record's, so
   vitest's glob would find the same files; a symlink to a directory
   disables it. Reporters: only built-ins whose effects are stdout/stderr,
   an outputFile (json, junit) or GitHub's step summary (appended); a
   reporter with its own outputFile, any other reporter, coverage, or a
   project root outside the cwd means no render. What differs from a
   vitest replay: the "Start at" and "Duration" lines and the JSON
   report's start time are the render's. ufo: 1.1 s → 0-10 ms on those
   commits.
2. **`quod vitest --plan FILE`, before `pnpm install`** (kept). The same
   selection with every key under the workspace's node_modules taken as
   recorded, which holds only if the install would be the same:
   `install_key` = every package.json, lockfile, pnpm-workspace.yaml,
   .npmrc, .pnpmfile (whole content, anywhere in the workspace), ~/.npmrc,
   npm/pnpm/corepack environment, node, platform, pnpm's version (the
   `pnpm` on PATH resolved to its package; not found: uncertain), quod.
   pnpm lockfile only. Function-level narrowing needs the project's
   typescript, so a file that would need it means `run`. If everything
   replays it emits the render, writes `done` and the job skips install;
   anything else writes `run` (install, then `--no-record` as before).
3. **node compile cache** (dropped to opt-in, `QUOD_VITEST_COMPILE_CACHE=1`):
   plain vitest with a warm cache: 1 file 730 → 670 ms. But the critical
   path runs on a copy of the state dir that is thrown away, and record runs
   must not use it (V8 coverage), so every critical run paid the cache
   writes: same-job on ufo it cost ~40 ms a run.
4. **Every CPU for large runs** (kept, > 4×CPUs files to run): maxWorkers =
   CPUs instead of vitest's CPUs-1 when neither config nor arguments set
   it. Same-job on router's two big commits (55, 60 files): 8.25 vs 8.89 s,
   8.38 vs 9.08 s (-7%). 10 files got slower in the breakdown (2.1 → 2.2 s),
   hence the threshold.
5. **Transform cache** (measured, not shipped): vitest's own
   `experimental.fsModuleCache` in the state dir (`QUOD_VITEST_FSCACHE=1`,
   populated by the shadow). Same-job on router, 20 commits: total -7.7%,
   median per commit -7% (2.49 → 2.26 s on the 16 commits running 1-14
   files), 0 wrong in that window. Not shipped: its key (id, content,
   config, plugin names, lockfile) does not cover every input of a
   transform (another module file read during it, e.g. an SFC's imported
   prop types or a macro plugin, which Quod's shared inputs exclude because
   they are modules); a stale hit runs old code, and sampling in the shadow
   cannot make that never-wrong. A sound version needs per-transform read
   attribution in the recorder (AsyncLocalStorage around transformRequest,
   every read of another module file joining the key) and a plugin
   allow-list; that is recording-side work (agent A's area).
6. **Scheduling** (no change): vitest's sequencer already runs
   longest-first from its results cache, which the bench keeps; maxWorkers
   8 on 4 CPUs is slower (3.7 vs 3.3 s), 2 slower too.
7. **Overlap / pre-spawn** (not pursued): selection is 3-17 ms, nothing to
   overlap with vitest's start; vitest exposes no worker pre-spawn.

## Numbers (same-job A/B: A = this branch, B = exp/vitest-shadow 4630f05)

Run in one job per tier, plain/A/B in rotating order per commit, both
compared with plain per file and per test (0 wrong everywhere; 0 recorded
runs differing from plain; mutation checks 0 wrong).

| tier | plain total | A total (B total) | A/B | whole job plain → Quod (`--plan`) |
|---|---:|---:|---:|---|
| ufo, 20 commits (run 41568727172) | 23.6 s | 15.4 s (17.0 s) | B/A 1.10 | 48.3 → 37.0 s (1.31x), 3 installs skipped |
| router, 20 commits (run 41568730885) | 224.9 s | 48.2 s (50.3 s) | B/A 1.04 | 237.6 → 60.6 s (3.92x), 1 install skipped |
| vuejs/core, 20 commits (run 41568738125) | 480.9 s | 170.4 s (181.9 s) | B/A 1.07 | 506.6 → 194.3 s (2.61x), 4 installs skipped |

Rendered full replay, per commit where every file replays: ufo 220-270 ms
→ 0-10 ms; router 410 → 20-40 ms; core (7 of 20 commits) 40 s plain → 20 ms.
