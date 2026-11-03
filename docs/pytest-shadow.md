# pytest replay with shadow recording (`quod pytest`)

2026-11-03, branch exp/pytest-shadow (from exp/vitest-shadow). The shadow
design of docs/vitest-shadow.md, brought to pytest: the developer's job runs
only the affected test files, plainly, and replays the rest; a separate,
non-blocking job records. Code: `crates/quod-cli/src/pytest.rs` (the CLI)
and `crates/quod-cli/src/pytest/quod_pytest.py` (a pytest plugin that does
the selection, the replay, the recording and the certification; the CLI
writes it into `<dir>/plugin/` and runs it with the project's Python).
Benchmark: pango07/ci-bench `bench-pytest.yml`, `scripts/pytest/`.

## The two jobs

**Critical path**: `quod pytest --no-record --dir L --python PY -- <pytest args>`.
- Loads the ledger, hashes every recorded input once (stat cache), and
  decides per test file: replay or run (below). Nothing is recorded: no
  audit hook, no import tracking, no `sys.monitoring`.
- Runs ONE pytest session with the project's arguments. Replayed files are
  not collected (`pytest_ignore_collect`), so neither imported nor run; the
  files that run keep their original relative order (pytest's own
  collection order). Their recorded tests are inserted as `ReplayItem`s at
  their position in the full run's file order, and each one's recorded
  reports go through `pytest_runtest_logstart/logreport/logfinish` and
  `pytest_warning_recorded`: the terminal summary, `-rA`, `--junitxml`,
  `--durations` and the exit code are a full run's. The collected count and
  `-k`/`-m` deselections are adjusted to a full run's.
- First run ever (no ledger), a taint (`-x`, `--lf`, `--ff`, `-n`, random
  order, reruns, ...), other arguments, another pytest/plugin set/Python:
  every file runs; the job is plain pytest under the plugin.

**Shadow** (a separate job, `continue-on-error`):
`quod pytest --dir L --audits 4 --expect-report <critical path's --summary> -- <same args>`.
- Same selection, then runs the affected files (plus up to 4 audited
  replayable files) with recording.
- A file records only if its per-test outcomes equal the critical path's
  for the same commit (`--expect-report`, object addresses normalized for
  matching only); otherwise "recording changed behaviour": quarantined. A
  file the report lacks is not recorded.
- **Certification**: every newly recorded file then runs alone, in a fresh
  process (in parallel), with the same arguments and with recording. Its
  outcomes must equal its outcomes in the session; then it is *certified
  order-independent*. Only certified files replay. A file whose results
  differ alone is order-dependent: it runs, and so does every file before
  it. A file that cannot be certified (unrecordable alone) just runs.
- **A file's inputs are the union of what it used in the session and what
  it used alone.** In the session a file can use objects another file
  created and cached (faker: a provider instance cached by an earlier
  file); it never imports that module, so the session's attribution misses
  it. The bench's mutation check found exactly this (a broken provider
  replayed as passing). Alone, in a fresh process, nothing is cached: the
  file loads everything it needs itself. Also, every workspace module whose
  code a file executed (sys.monitoring) is its input, imported or not.
- Audits: an audited file whose results differ from its record is
  quarantined.
- Writes the ledger (`ledger.json`, `results/<id>.json`, `blobs/<hash>`)
  that the next commit's critical path reads. The hand-off is the vitest
  one (actions/cache keyed by branch and commit; see docs/vitest-shadow.md).

## Attribution inside one process

pytest collects every test file, then runs every test, in one process. The
recorder keeps a stack of **windows**: the session (start-up, conftests,
plugins, session- and package-scoped fixtures), each test file (its
collection and its tests, module- and class-scoped fixtures included), and
each module's first load. Everything the process does goes to the current
window:
- reads (`sys.addaudithook` "open"), directory listings (os.listdir and
  os.scandir by anyone but the import system and pytest's collection),
  stats (`os.stat`, `lstat`, `access`, `readlink` wrapped), env reads
  (`os._Environ.__getitem__`, enumeration keys the whole environment),
- imports (`builtins.__import__`, `_gcd_import`: every import, cached or
  not, is an edge from the window to the module; failed imports too),
- each module's source (its spec's origin, keyed at load: with a valid pyc
  the source is never opened),
- executed functions (`sys.monitoring` PY_START per window, 3.12+; events
  restart at every window change).

A file's inputs: its window's keys, plus every key of the module-load
windows in the closure of its imports. The session's inputs (and the
closure of the session's imports) are every file's. Import resolution is
keyed per imported name in every workspace directory it is searched in
(`x:dir/name`: a new `json.py` in tests/ shadows the stdlib). Not inputs:
the stdlib and site-packages (the interpreter and the distributions are
keyed instead), `__pycache__`, the temp dir, pytest's cache (a test reading
it is not recorded), files the session wrote.

**Session-level keys**: the interpreter (`py:`: version, executable,
prefix, flags, `PYTHON*`), `sys.path` and meta path at start, every
installed distribution by **name and file content** (RECORD-listed files
hashed, dist-info bookkeeping and pycs excluded; not by dist-info dir
names), the top-level `.pth` and `.py` files of site-packages, pytest's own
section of `pyproject.toml`/`tox.ini`/`setup.cfg` when pytest is the
reader (`ini:`; a version bump or a ruff setting is no input), the args
(output-only ones excluded), pytest, its plugins, the plugin's own hash.

**The project's own editable install** (attrs's lesson): its dist-info
(version in the dir name and METADATA) is not part of the distribution key.
A read of one of its files is keyed by file name inside the current
dist-info (`od:name/METADATA`): `importlib.metadata.version(...)` keeps the
version as an input of the files that read it; pytest's entry-point scan
keys only `entry_points.txt`.

**Unrecordable** (always runs): a subprocess, the network (anything but
loopback), ctypes, a write into the workspace, a per-run CI variable read, a
thread that outlives its file, a recorder error. The session is
unrecordable (nothing replays) when it is one of these, or when a plugin
interleaves test files.

## Function-level narrowing

As vitest's, with Python's `ast`: Quod keeps the source of every module key
by content. A file whose only changed inputs are module sources (`m:`)
replays when every change lies strictly inside the body of a `def` the file
never executed (its windows, the session's, and its import closure's load
windows; the innermost def that holds the change decides; the change must
be inside the same def in the new version, which must compile). A change
in a docstring, a signature, a decorator, a class body or at module level
runs. No narrowing for a path the file's recorded results quote
(warnings, captured logs: line numbers shift), nor for a path it read as
data (`f:`).

**Version bumps** (the `pj:` rule's Python counterpart). A change that is
only a module-level version constant (`VERSION`, `__version__`, `version`,
`__VERSION__`, `VERSION_STRING` assigned a string literal; the ASTs are
otherwise equal) replays for a file when its own source, every workspace
file whose code it executed and the defining module outside the
definition do not name the constant, and its recorded results quote neither
value. faker's `VERSION = "40.44.0"` is read by `faker/cli.py`: the files
that executed the CLI run, the others replay. Residual: third-party code
reading the project's version by introspection. Tests:
`crates/quod-cli/src/pytest/test_quod_pytest.py` (run before every bench).

**The fast path.** When more than half of the test files must run, the
critical path replays nothing: one plain session, no guard, no snapshots
(the selection, 20-200 ms, is the only cost).

## Cross-file state: the rule

pytest gives no isolation: a file can change state another file sees
(module globals, class attributes, registries, monkeypatch leaks, env,
cwd, warnings filters, logging, session-scoped fixture values). And the
whole collection (every test module's import) happens before any test runs.

**Certification** (above) proves at record time that a file's results do
not depend on what ran before it in the session.

**The leak guard** closes the other direction where state is fingerprinted.
A fingerprint covers: every attribute of every workspace module (values to
depth 2, and class attributes), rebinding of every other module's
attributes (a third-party global going from None to a value counts as lazy
initialisation), interpreter-wide state (env, cwd, sys.path, meta path,
warnings filters, logging root, recursion limit, tz, gc, decimal context,
socket timeout, locale, signal handlers, live threads), and session-scoped
fixture values. The shadow records, per file, the delta of its collection
(`cleak`) and of its tests (`rleak`).

Selection then:
1. A replayed file whose collection leaks is still collected (its tests
   replay) whenever anything runs: in a plain run its import side effects
   are there for every file.
2. A replayed file whose tests leak, placed before a file that runs, runs.
3. A file that failed certification runs with every file before it.
4. The critical path fingerprints at the boundaries of what it runs. A
   file that ran and changed state differently from its record (keys a
   replayed file can observe: its import closure, its fixtures, or any
   global) trips the guard: the whole suite runs again, plainly, and that
   is the job's result. Same for an unpredicted new test file beside leaky
   replayed files, and for missing records.

**Residual, stated precisely.** A replayed, certified file B can still
differ from a full plain run when a file that runs leaks state into B
through something the fingerprint does not see:
- mutation deeper than the fingerprint's depth (an object two levels down a
  module global, a container longer than 200 entries mutated in place with
  its length unchanged, an object's state reachable only through closures,
  C-level state of an extension module, a `functools.lru_cache`'s
  contents, `codecs`/`atexit`/`copyreg` registries, abc caches);
- a third-party module's new attribute, or a third-party global going from
  None to a value (treated as lazy initialisation);
- files written outside the workspace (the temp dir) that B reads;
- threads still running (taint only checks the files that started them).
B was certified (alone == in session) when recorded, so such a leak needs a
change in the file that leaks. The bench compares every replay with a full
plain run; it would expose any instance.

Not fingerprinted on purpose: the global `random` state (every test that
draws a number would be a leaker) and the decimal context's flags. A file
that runs and draws unseeded random numbers sees a different random state
than in a full run (fewer files ran before it); seen once in the bench, on a
faker mutation check: `test_providers_formats.py` (it ran on both sides)
failed in plain and passed on the critical path because a random format
choice did not reach the mutated function. That file is nondeterministic by
order under plain pytest too; it is counted as flaky, not wrong.

Also residual: a file that runs is run without the replayed files before it
(like a developer's subset run). If it depends on state they leave that the
fingerprint misses, it differs from the full run. Rule 3 covers files known
to be order-dependent; a changed or new file is not yet known.

## Measured

pango07/ci-bench `bench-pytest.yml` (`scripts/pytest/shadow.sh`): 20
consecutive first-parent commits ending at each project's HEAD on
2026-11-03, nothing picked; per commit plain `python -m pytest` (warm venv,
uv, Python 3.12), then the critical path, then that commit's shadow; every
critical run's JUnit XML compared with plain's test by test; a mutation
check (a raise in a function a recorded test executed) on commits 4, 9, 14,
19; a monotonic clock. Neither project uses xdist in CI. Coverage flags are
not used on either side.

| tier | project | plain total | critical path total | speedup total / median | files run / replayed (narrowed) | shadow compute | guard trips | wrong | mutations |
|---|---|---:|---:|---:|---:|---:|---:|---:|---|
| 1 | Textualize/rich (63 files, ~5 s; `TERM=unknown` as its CI) | 102.4 s | 79.5 s | 1.29x / 1.18x | 582 / 678 (5) | 245.6 s | 0 | 0 | 4/4 |
| 2 | joke2k/faker (62 files, ~26 s) | 533.2 s | 286.0 s | 1.86x / 2.06x | 351 / 881 (707) | 630.7 s | 2 | 0 | 4/4 |

Release binary bin-be9d27a: rich run 41601468959, faker run 41601473053.
The branch head (1711c7e: equal-value rebinding, path patterns in the
pre-scan, the fast path skips narrowing, no per-test wrapper when nothing
replays) could not be built (pango07/quod Actions blocked by billing); the
same plugin through the bench's `bin=dev` stand-in: rich 1.27x / 1.13x,
0 wrong, 4/4 (41606745587); faker 2.04x / 2.10x, 0 wrong, 1 guard trip,
mutations 3/3 compared plus one that broke faker's pytest plugin at load,
where both sides exited 3 identically (41606750047).

- Commits that touch only docs replay everything (faker ~10x, rich runs
  the ~10 files that always run: unrecordable, failing on this runner, or
  leaking state into later ones: ~2.2x). A faker release bump (`VERSION`
  only) replays 55 of 62 files.
- Full runs (the fast path, or a shared input changed): +0.8% to +8%
  over plain on rich (mean +3%), within ±1% on faker. The cost is fixed
  (the plugin's start-up and the selection, 100-250 ms), so it shows on a
  5 s suite.
- The mutation check found one wrong replay during development (faker: a
  file using a provider another file had cached); fixed by recording inputs
  from the alone run too. Since then 0 wrong in every run.
- rich's `tests/test_live.py` is order-dependent (alone != in session):
  whenever it runs, every file before it runs too (the 1.0-1.2x commits).

## What blocks a bigger number

1. Shared modules in the session closure: faker's pytest plugin imports
   faker at start-up and lists every provider directory; a new locale
   re-runs everything. rich's core (console, text, segment, style) is
   executed by nearly every file, so core changes run most files and take
   the fast path.
2. Files that can never replay: rich writes into the workspace in two
   files, fails 2 files under this runner's terminal, and has an
   order-dependent file that drags its prefix in.
3. Leaky files forced to run (direction 2): files that leave state other
   files can observe (rich's global console, faker's `Generator._global_seed`
   via `Faker.seed()`).
4. Guard trips cost a second full run (faker `time.tzset` in test_ssn.py is
   order-dependent: whether it changes `time.timezone` depends on what ran
   before).
5. Fixed overhead of ~100-250 ms per run (the Python plugin's start-up and
   the selection); moving the selection to Rust (as `quod vitest` does)
   would cut most of it.
