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
3. **node compile cache** (kept, small): `module.enableCompileCache` in the
   state dir (travels with the ledger, ~2.5 MB on router), main process and
   workers (NODE_COMPILE_CACHE). V8 checks each entry's source hash. Plain
   vitest with it: 1 file 730 → 670 ms, 10 files ~-40 ms.
4. **Every CPU for large runs** (kept for > 4×CPUs files): maxWorkers =
   CPUs instead of vitest's CPUs-1 when neither config nor arguments set
   it. Full router run 9.4 → 8.4 s; 10 files 2.1 → 2.2 s (worse), hence the
   threshold.
5. **Transform cache** (measured, not shipped): vitest's own
   `experimental.fsModuleCache`, warm: 1 file 1.22 → 1.03 s, 10 files 3.3 →
   3.0 s. Not shipped: a transform of X can depend on files other than X
   that vitest's key (id, content, config, plugin names, lockfile) does not
   cover, and that Quod's global inputs exclude because they are modules
   (e.g. an SFC's imported prop types, a macro plugin); a stale hit would
   run old code, and sampling in the shadow cannot make that never-wrong.
   A sound version needs per-transform read attribution in the recorder
   (AsyncLocalStorage around transformRequest, every read of another module
   file joining the key) and a plugin allow-list.
6. **Scheduling** (no change): vitest's sequencer already runs
   longest-first from its results cache, which the bench keeps; maxWorkers
   8 on 4 CPUs is slower (3.7 vs 3.3 s), 2 slower too.
7. **Overlap / pre-spawn** (not pursued): selection is 3-17 ms, nothing to
   overlap with vitest's start; vitest exposes no worker pre-spawn.

## Numbers

See the tables in the branch's final report (filled from the ci-bench runs).
