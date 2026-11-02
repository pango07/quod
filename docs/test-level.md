# Test-level replay (pytest)

2026-11-02 · exp/test-level. The unit of replay inside a pytest step is the
test. A typical commit changes a few files; a whole-step replay misses on
any of them and re-runs thousands of tests. At test level, a step re-runs
the tests whose inputs changed and replays the rest, with the summary and
exit code a full run gives. The safety invariant is unchanged: never
under-record, zero false replays.

The pieces: `crates/quod-fs` (the observer's ordered trace),
`crates/quod-tests` (the vendored pytest plugin, keys, attribution),
`crates/quod-cli/src/testlevel.rs` (plan, record, report), ledger v9, and
`quod_trust::decide_phased`. Tests: `crates/quod-tests/src/tests.rs`,
`crates/quod-fs/tests/trace.rs`, `crates/quod-cli/tests/test_level.rs`
(fixture `fixtures/test-level`, with the mutation suite). Measurement
driver: `scripts/bench-real/testlevel.py`.

## The unit, and when a step runs at test level

The unit is a pytest test, by node id; a parametrized case is its own test
(its id names its parameters). A step runs at test level when it is a
Python step whose script runs pytest (`pytest`, `py.test`, `python -m
pytest`, `.../pytest`), unless `QUOD_TEST_LEVEL=0`. It then has its own
step key: the key's env gains `QUOD_TEST_LEVEL=pytest-v1`, because the step
executes with Quod's plugin and so is not the same artifact as a plain run
(the tag is bumped when the plugin or the attribution rules change, which
starts every test over). Opt-out keeps the plain key.

`quod run` first decides the whole step as before. If it replays, it
replays (an identical re-run still costs a whole-step replay, ~1 s). If the
step executes and its whole-step manifest matched (a proof, a shadow or
decay audit, observe mode), every test runs: that execution is evidence
about a whole-step result and must be one. Only a whole-step miss (or a
recorded failure) plans test replays.

## Attribution

A test's manifest must contain everything its outcome can depend on. Three
mechanisms supply it.

**Windows, marked in order with the reads.** The plugin brackets each
test's setup, call and teardown (`pytest_runtest_protocol`), each test
module's collection (`pytest_make_collect_report` of a module and its
classes), each module-, class-, package- or session-scoped fixture's setup
(`pytest_fixture_setup`) and teardown (`FixtureDef.finish`), and each
module's first import (`importlib._bootstrap._load_unlocked`). A boundary is
a marker: the plugin logs `[n, begin|end, kind, id]` to its output and
looks up `/workspace/.quod-tl-<token>-<pid>-<n>`. The observer recognizes
the name (the token is random per run), logs `Marker(pid, n)` in its trace
and records no access. Every recorded access carries the requesting thread
and a sequence number taken under the log's lock *before the request is
answered*. Hence for any read R that could have influenced what a process
did before it logged marker M: R's request was answered before the process
acted on it, so seq(R) < seq(M). A read can only land in a later window
than the one it influenced, never an earlier one.

**The kernel must ask every time.** Only the first access to a path
reaches FUSE when the kernel caches entries, attributes and opens. On a
traced mount the observer answers lookups and getattrs of files and
symlinks with no cache time, never answers opens with zero messages (so
every open is a request), and keeps the directory TTL. Through the
overlay a step sees, every process's lookup, stat and open of a file then
reaches the observer in each window (`trace.rs`, both transports).
What can still be served from a cache is the session's (below).

**Imports are carried by the import graph.** A module is imported once
per process: a second test module that imports `calc.add` reads nothing,
yet depends on everything `calc.add`'s first import read. The plugin wraps
module execution (each first load is a window: its reads are that
module's) and both entries of an import statement (`builtins.__import__`
and `_bootstrap._gcd_import`, so `importlib.import_module` too): every
import, cached or not, is an edge from the current window to the module
and its parent packages (and `from x import y` submodules). A test depends
on the closure of its window's, its module's collection window's and its
module-scoped fixtures' imports over the module graph, with each module's
first-load reads. If the import system is not the one these wrappers
expect, the run is not recorded at test level.

**Processes.** The observer learns each requester's thread group and
parent from `/proc` while its request is in flight, and every ancestor's.
An access belongs to the windows of its thread group, or of its nearest
ancestor that runs tests: a test's subprocess reads in that test's
window, a worker's threads in the worker's. The plugin's pid is the one
inside the sandbox's pid namespace; markers carry it, which ties each
output file to the host process that logged the markers. Under xdist each
worker is a separate process with its own windows; the master's reads are
the session's.

**What goes to the session (every test).** Any access that is not a file:
directory listings, absent names, symlinks (the overlay keeps negative
dentries and merged-directory listings, and FUSE caches symlink targets,
so a second asker may not reach the observer). Any read outside every
window: interpreter start-up, pytest's own imports, conftest files, the
rootdir config, plugins, `site-packages` (the sealed venv, recorded whole),
reads between tests. Reads in a session- or package-scoped fixture.
Reads of a process that is not a test runner or one's descendant (the
step's shell, a second pytest the script runs, an orphan). The closure of
the session's imports: a conftest that imports the package under test
makes the whole package everyone's. A window in which a module was
imported off the main thread (its window cannot be told apart): its reads
and imports.

**Conservative throughout.** The run is not recorded at test level (and
if it replayed tests, it is run again in full, below) when: the traced
markers of a process do not match its output one for one; any process's
plugin output is missing, unfinished or tainted; two pytest sessions owned
the step; xdist workers collected different tests; the collected tests are
not exactly the executed plus the replayed; a test ran twice or both ran
and replayed; the plugin replayed something the plan did not name. The
plugin taints a run under `--maxfail`/`-x`, `--lf`, `--ff`, `--nf`,
`--sw`, `--pdb`, `--trace`, `--looponfail`, `--collect-only`,
`--setup-only/plan`, `--reruns`, `--count`, and the plugins randomly,
random_order, rerunfailures, forked, flaky, testmon and picked (order or
reruns change what a test sees).

Known over-attribution (costs replays, never soundness): a test that fails
or xfails formats a traceback, and `linecache.checkcache` stats every file
linecache holds, so such a test depends on them all.

## Keys, manifests, ledger

- A test's key: `b3(“quod test key v1”, step key, node id)`. Its manifest:
  the files attributed to it (its window, its module's, its imports'
  closure). Its result: a `StepResult` whose stdout blob is its reports
  (setup, call, teardown, serialized as pytest-xdist does, without
  durations and Hypothesis's statistics) and warnings, exit code 1 if any
  phase failed. Everything else about a test lives in the step tables
  under its key, so proofs, the shadow and decay audits, `equivalent`,
  quarantine and suspect marking apply per test unchanged.
- The session's key: `b3(“quod test session v1”, step key)`; its
  manifest is the session's accesses united with the matched previous
  session manifest (a replayed test's listings were recorded there). Every
  test replay requires it to match; when it does not, every test runs.
- Ledger v9: `tests (test_key, step_key, nodeid, last_run)` (what a step
  collected) and `test_steps (step_key, runs, since_full, full_due,
  disabled, coupled)`. Each test gets run rows (`mode = 'test'`, ids
  `<run>.t<i>` executed, `<run>.r<i>` replayed); the step's run row says
  `tests` when tests replayed.

## Replay

The plan: for every test the step key has collected, `find_match` and
`Trust::decide` under the step's policy with deferral off (a test's audit
costs that test, in a run that happens anyway) and phased (below). A test
replays when its manifest matches, its trust says replay, and nothing
forces a full run: the session manifest matched, no divergence asked for
one, and this is not every `FULL_EVERY`th (10th) test-level run. The plan
(node id to recorded reports) goes to the plugin in a file.

The plugin takes the planned tests out of the collected items after the
command's own `-k`/`-m` selection (as deselection would, without counting
them deselected) and reports their recorded reports through
`pytest_runtest_logstart`, `pytest_runtest_logreport` (marked
`quod_replayed`; `PASSED (replayed)` in verbose output), the recorded
warnings through `pytest_warning_recorded`, and `logfinish`, in collection
order, interleaved with executed tests. Under xdist every worker removes
the same tests (their collections agree) and the master reports them after
its loop. Replayed tests count as collected; a run where every test
replays exits 0, not 5. Nothing is reported after an interrupted session
(collection errors, a signal). The summary line and the exit code are a
full run's. `quod: N tests replayed` is printed at the end.

**A failure beside replays runs everything.** If a run that replayed tests
fails (or cannot be attributed), the step runs again with every test, and
that run is the step's result: a replay never makes a step fail that a
full run passes. The first attempt's executed tests are recorded; the
failed ones are compared with the full run, and a test that fails beside
replays and passes in the full run diverges: it is quarantined, and its
module is *coupled*: from then on its tests replay only when all of them
do. The cost: a genuine failure in a run with replays executes twice.

**Composition with the step level.** A run that replayed tests records no
whole-step result (its output is not a whole run's, and its observed
manifest lacks the replayed tests' reads); a run that replayed none
records one as before, so an identical re-run still replays whole.

## Hazards and their handling

| Hazard | Handling |
| --- | --- |
| Order dependence, shared in-memory state (a cache, a registry, monkeypatching) | Not visible to the observer. Caught by: per-test shadow audits (each test's first two would-be hits execute and compare), the phased decay audits, a full run every 10th test-level run and after any divergence (each test compared with its recorded result, `would-replay`), and the failure rule above (a test that fails beside replays is quarantined and its module coupled). A test whose *pass* depends on a replayed test's side effect is caught at once; one whose *failure* would be caused by a replayed test is caught by the next full run or audit, and its replays since are marked suspect. Residual. |
| Fixtures with session/module scope | Their setup and teardown are windows of their scope: module-scoped reads go to the module's tests, session-scoped to all. |
| Tests writing files other tests read | Writes go to the overlay's upper, which the observer never sees; a read of a file a test wrote is invisible. Same class as shared state: audits and full runs. Residual. |
| Randomness, time | Per-test results are compared after the trust layer's normalization; durations are not part of a test's result. A nondeterministic test diverges and is quarantined (Hypothesis's statistics, which carry run times and random counts, are dropped from results). |
| xdist | Supported: per-worker windows; same removal on every worker; the master reports replays. Load balancing changes which tests share a process: shared-state coupling across workers differs from run to run, caught as above. |
| Subprocesses | Their reads are attributed to the test whose runner is their ancestor, by the trace's process table. |
| Reads outside the observed tree | The workspace is the observed tree; the venv is sealed and recorded whole in the session; the image or host is in the step key, as for the whole step. |
| Plugins that alter collection or order | Known ones taint the run (above). Collection itself is always full (only execution is skipped), so collection-time plugins see what they always see. Unknown plugins that change order: residual, audits. |
| `-k`/`-m` | In the command, so in the step key; the plugin removes tests after the command's own selection. |
| Failing tests | A failure never replays (`unprovable`); it always executes and keeps failing. |
| Two pytest sessions in one script | The first owns the step (an exclusive file); the second runs plainly and its reads are the session's. |
| A test killed by the box | As for steps: a killed step records nothing. |

## Trust per test

Each test key has the trust row, proofs, audits and divergences of a step
key. Two adjustments: tests never defer an audit (inline is one test's
cost), and decay-phase audits are **phased** per key
(`quod_trust::decide_phased`, `audit_phase`): every test of a step shares
one history, so unphased they would all be audited at hits 4, 7, 11, ...
and every few runs would run every test. Shifting each key's decay by a
hash of the key spreads the same audits over the runs (unit test
`phased_audits_keep_the_shadow_and_spread_the_decay`: no run audits more
than a quarter of 500 keys after the shadow; the total is within 0.8-1.05
of unphased). The shadow (each test's first two would-be hits) is
unshifted. A divergence quarantines that test (its replays since its last
quarantine are marked suspect) and makes the next run full.

## The hooks path

Not in this branch: `quod run` only. The hooks run a step on a hermetic
surface rebuilt per job and keep audits inline; test level there needs the
plugin's directory bound into that surface, the observer's trace from the
hooks' mounts, and the per-test rows in the per-scope ledgers. The design
carries over unchanged; the plumbing is the follow-up.

## Measured

See "Measured" below (filled in from `scripts/bench-real/testlevel.py`).

## Residual risks

1. **In-process state not mediated by imports or files.** A test whose
   outcome depends on what an earlier test left in memory (a cache filled
   from a file, a registry entry, a monkeypatch) records a manifest without
   that dependency. If the earlier test's input changes and the later test
   replays, the replay may be wrong. Caught by the shadow and phased
   audits, the full run every 10th run, and the failure rule; the replays
   served meanwhile are marked suspect. This is the residual the design
   accepts, as the product owner's brief does.
2. **Writes the observer cannot see**: a test that reads a file another
   test wrote (workspace or /tmp). Same net as 1.
3. **Import tracking relies on CPython's `importlib._bootstrap`
   internals** (`_load_unlocked`, `_gcd_import`, `_resolve_name`,
   `_calc___package__`), present in 3.8-3.14. Missing, the run is not
   recorded at test level. A C extension that imports through the C API
   without `__import__` (`PyImport_ImportModule` does call it) or code that
   reaches a module only through `sys.modules` makes no edge; its load
   reads are found only if the module was first loaded in a window the test
   depends on.
4. **pid reuse within one step** could misattribute a short-lived process's
   reads (pid_max is 4M; the observer learns a pid once).
5. **Off-thread imports** make their window the session's; threads that
   do other reads are attributed by time to the worker's current window,
   which is sound for the current test and over-records for later ones.
6. **A full run every 10 runs** bounds how long a stale replay of class 1
   lives, in runs, not in time.
