# Polyglot corpus (exp/polyglot): does the engine generalize?

The 0a engine (FUSE observer serving a CAS tree as an overlay lower,
bubblewrap sandbox, manifests, ledger, replay) was proven on one Go repo.
This experiment runs it on one real Node repo (Jest) and one real Python
repo (pytest), after moving everything Go-specific into a toolchain
profile, and re-runs the Go acceptance on cli/cli. Same box as every other
report: Docker Desktop on Apple Silicon, 18 arm64 cores, 7.7 GiB, kernel
7.0.14-linuxkit. This session shared it with four to six other agents'
builds and benches: load averages of 12 at the quietest and 30-90 for most
of it, and three global OOMs. Two of those killed a Jest worker of this
experiment's own runs when it ran several heavy jobs at once; both runs
were stopped and repeated alone, and nothing here comes from them
(DECISIONS.md). Every timing below is paired with its load; compare series
within a table, never across reports.

Per-repo bench reports: `polyglot-luxon-arm64.md`,
`polyglot-starlette-arm64.md`. Acceptance logs: `polyglot-acceptance.log`.

## The corpus

| Toolchain | Repo | Commit | Step A (network, never memoized) | Step B (observed, memoized) | Suite |
| --- | --- | --- | --- | --- | --- |
| Go | cli/cli | `9490d204a2ba` | `go mod download` | `go test -p 4 ./...` | 259 packages |
| Node | moment/luxon | `f427515a38f6` | `npm ci` | `export LANG=en_US.utf8 LIMIT_JEST=yes TZ=America/New_York; npm test -- --maxWorkers=4` (Jest 29, babel-jest, `--coverage`) | 1,222 tests, 58 suites |
| Python | encode/starlette | `2745bbc5afa2` | `uv sync --frozen` | `python -m pytest` | 1,414 tests (asyncio + trio) |

Why these two: each is the largest suite found in its ecosystem that has
real Actions CI, a lockfile, and passes offline after step A inside the
sandbox as uid 1001 (DECISIONS.md lists the candidates that failed one of
those). luxon's step is its workflow's `npm test` with the job-level env
the workflow sets; starlette's is its `scripts/test` without coverage.
`-p 4` and `--maxWorkers=4` cap parallelism on the shared box; both are
part of the step, so of the key, and fixed for every run here.

## What generalized as-is

Unchanged from the Go-only engine and doing its job on all three repos:
the observer (FUSE over the CAS tree, passthrough reads; its one change is
the content-derived mtime below), kernel overlayfs over it, the output diff, the manifest kinds (`f`, `d`,
`a`, `l`) and the matching rule, the ledger, replay, the sandbox (uid 1001,
no network, fresh `/tmp`, read-only root), the sealed-prefix snapshot
machinery, and A7/A7b. The one rule that carried the safety argument across
languages is "a stat records the file's content hash": every cache in the
corpus that is validated by a source's mtime and size (Python's pycs,
pytest's rewritten modules, Jest's haste map, @babel/register) stats the
source to validate it, so the source's content is in the manifest and a
changed source can never replay a result recorded with the old one, whatever
the cache believed. The same rule is also the biggest cost on Python (below).

## What the engine needed

New, and generic (no language in run.rs, seal.rs, bench.rs or the suite):

- **Toolchain profiles** (`sp_key::profile`): the fixed step env, the
  dependency-install step and its network env, download caches bound only
  into step A, ephemeral dirs cleared before step B, sealed prefixes with
  carve-outs, warm state with its cache-disabling knobs and over-record
  rule, `no_outputs_under`, and the suite's file picks. Go, Node and Python
  are three values of one struct; the step key names the toolchain (schema
  v3, which collides with exp/rootfs's v3: the second to merge takes the
  next version). The split driver is the one slot not filled (exp/split
  owns it).
- **Sealing by rule, not by GOMODCACHE**: a prefix is an env variable's
  directory (GOMODCACHE, VIRTUAL_ENV) or every directory of a name not
  nested in another (node_modules, so monorepos work), each with carve-outs
  the tool writes to (`gomod/cache`; `node_modules/.cache`, `.vite`,
  `.vitest`). The snapshot machinery is unchanged (completion marker,
  shared locks, rename-away gc, blobs linked or copied by mode, read-only
  bind, every tree entry under the prefix recorded) apart from fixed times
  in the snapshot (key format v2).
- **Step A as a network step on the repo** that fills node_modules or the
  venv in the workspace, with npm/pip/uv download caches bound from
  `<state>/setup-cache/<toolchain>` so they never enter a tree.
- **The environment pin covers whole toolchain installs**: binary hashes
  and versions as for Go, plus a listing hash of each install root
  (npm's JavaScript, Python's stdlib, GOROOT), `__pycache__` directories
  left out of the listing and unchecked-hash pycs pinned by content.
- **Content-derived mtimes** (review fix): served files, and the files of
  sealed snapshots, carry an mtime derived from their content hash
  (`sp_fs::content_mtime`, 58 bits of the hash) instead of one constant, so
  an mtime-keyed cache sees a new mtime whenever content changes;
  directories a fixed time.
- **mtime-keyed caches never become outputs** (review fix): outputs under a
  profile's `no_outputs_under` patterns are dropped when recorded and when
  materialized.
- **EROFS taint** (review fix): a run with sealed prefixes whose output
  reports a read-only file system is shown and never memoized, since CI's
  write would have succeeded.
- **Acceptance per repo and step**: `scripts/acceptance.sh go|node|python
  [checkout]`, A3/A4 picks from the profile, A7b's path comparison up to
  implied absence, `--no-verify` commits.

## Input-hiding caches, and what each needed

A cache hides an input when a run that hits it reads less than a run that
misses it, or when it is trusted on a key the observer cannot see (a path,
an mtime). Under this observer a stat records the file's content hash, so a
cache validated by a stat of its source (mtime and size) can never turn a
changed source into a false hit: the stat is in the manifest. What such a
cache could do, while the observer served one constant mtime, was make the
*recorded result* wrong once an entry written under observation came back
into a tree (an outputs directory materialized into the next checkout): the
edited source showed the same mtime. Two defences close that now: served
mtimes are content-derived, and outputs under mtime-keyed cache paths are
never recorded or materialized. The table is every cache found on the
corpus, how it is keyed, and its handling.

| Toolchain | Cache | Keyed by | Persists? | Handling |
| --- | --- | --- | --- | --- |
| Python | `__pycache__` timestamp pycs | source mtime + size | in the tree | `PYTHONDONTWRITEBYTECODE=1`: no step writes one; one already in a tree is read (recorded) and its source stat'd (recorded); outputs under `__pycache__` are never recorded |
| Python | site-packages pycs from install | source hash (checked-hash, PEP 552) | in the sealed venv | `SOURCE_DATE_EPOCH` makes uv's compileall write checked-hash pycs (flags=3 on all 2,500); sealed venv is recorded whole |
| Python | pytest assertion-rewrite pycs | source mtime + size | `__pycache__` | honours `sys.dont_write_bytecode`; never written |
| Python | `.pytest_cache` (lastfailed, nodeids) | node ids | workspace | read if present (observed), written as an output; step B's workspace is discarded |
| Python | importlib FileFinder listing cache, `sys.path_importer_cache` | directory mtime | in process | dies with the process; the listing it caches is a recorded `d` |
| Python | stdlib `__pycache__` under /usr/local/python | source mtime + size, or none (unchecked hash) | toolchain install | read-only inside the sandbox; the directories are left out of the fingerprint's listing (root pythons outside the sandbox write timestamp pycs, which are validated against their pinned sources); unchecked-hash pycs are pinned by content |
| Python | `.mypy_cache`, `.ruff_cache` | mtime + size | workspace | never recorded or materialized as outputs |
| Python | uv / pip download caches | content | host | step A only, bound from `<state>/setup-cache`, never in a tree |
| Node | Jest haste map | file mtime + size | `/tmp/jest_*` | sandbox `/tmp` is a fresh tmpfs: cold every run; not warm state |
| Node | Jest transform cache (babel-jest) | content + config hash | `/tmp/jest_*` | fresh tmpfs |
| Node | V8 compile cache (npm enables it) | source hash | `/tmp/node-compile-cache` | fresh tmpfs; content-validated anyway |
| Node | `node_modules/.cache` (babel-loader; @babel/register: filename + mtime) | content, or mtime | node_modules | carve-out (observed, writable); never recorded or materialized as an output, and served mtimes follow content, so a stale entry cannot match an edited source |
| Node | `.eslintcache`, `.stylelintcache` (default metadata strategy) | mtime + size | workspace | never recorded or materialized as outputs |
| Node | Vitest results cache | test file | `node_modules/.vite`, `.vitest` | carve-outs |
| Node | npm update notifier, debug logs | time | npm cache dir | `npm_config_update_notifier=false`, `npm_config_logs_max=0` |
| Node | npm download cache | content | host | step A only, bound from host state |
| Go | module index (GOCACHE) | module path | warm GOCACHE | `GODEBUG=goindex=0` in warm runs (unchanged) |
| Go | test result cache | size + mtime | warm GOCACHE | `-count=1` in warm runs (unchanged) |
| Go | vet verdict | compiled files only | warm GOCACHE | over-record every Go source in listed dirs (unchanged) |

Node and Python get no warm state (W1-W4 do not apply to them). The caches
that would make their misses cheap, Jest's haste map and Python's import-time
pycs, are keyed by mtime. With content-derived mtimes they follow content up
to 28 bits of the hash in whole seconds (plus size), which is a probability,
not the proof warm state needs; a content-keyed variant is the precondition.
Python comes close: a warm `PYTHONPYCACHEPREFIX` seeded with checked-hash
pycs is validated by source hash on every import and rewritten as
checked-hash when stale, but a module with no pyc yet gets a timestamp pyc on
first import; it would need a pre-pass compiling new sources as
checked-hash. Not built here.

## Absent-path storms

Node's resolver probes `x.js`, `x.json`, `x/index.js` and a `node_modules`
in every ancestor; babel and Jest search every directory up to the root for
six spellings of their config; Python's FileFinder probes each package for
`__init__` with every extension-module suffix, and pytest probes every
collected directory for `conftest.py`, `__init__.py`, `pyvenv.cfg` and
`conda-meta`. Each distinct absent path is one `a` entry, and correct: A7
creates every one of them and every creation misses.

What reaches the observer, per run (from the observer's request counters):

| Repo | Dependency tree | Inputs recorded | `a` entries | Lookups | Absent lookups | Opens | Readdirs |
| --- | --- | --- | --- | --- | --- | --- | --- |
| luxon | sealed (default) | 24,987 | 88 | 190 | 88 | 1,707 | 14 |
| luxon | observed (`STILLPASS_SEAL=0`) | 5,797 | 3,241 | 5,796 | 3,241 | 21,653 | 26 |
| starlette | sealed (default) | 25,325 | 76 | 235 | 78 | 96 | 42 |
| starlette | observed (`STILLPASS_SEAL=0`) | 1,757 | ~417 | 1,758 | 417 | 1,050 | 180 |
| cli/cli (exp/overhead, before sealing) | observed | 21,051 | ~4k | 21k | 4k | 45k | 20k |

Medians over the overhead bench's runs (cli/cli from DECISIONS). Unsealed,
luxon's 3,241 `a` entries are 3,156 inside node_modules: 428 nested
`node_modules` probes, 314 `package.json`, then the `.js`/`.json`/`.node`
spellings of every bare import. Starlette's unsealed absent lookups count
417; its `a` entries were not dumped (sealed: 76 entries for 78 lookups).

With the dependency tree sealed, the probes inside it never reach the
observer (the snapshot is a plain directory) and are covered by the
prefix's recorded listings, which A7 also mutates; outside it, overlayfs
caches negative dentries, so an absent path costs one request however
often it is probed (absent lookups equal `a` entries: 88 and 3,241 on
luxon, 78 for 76 on starlette). A negative-entry TTL in the observer's
LOOKUP reply would save nothing here. The storm is a Node problem: the
resolver's probes and opens inside node_modules are over 90% of luxon's
unsealed traffic, and sealing halves its execution overhead (below);
Python's FileFinder lists a directory once and answers from the listing,
so starlette sends 1,758 lookups even unsealed.

## Acceptance

`scripts/acceptance.sh <toolchain> <checkout>` on each repo, with the code
committed on this branch. A7 mutates every recorded entry (the whole sealed
prefix included) and asserts a miss; A7b corrupts files the observer did
not record, asserts a replay, then re-executes under a fresh key and
compares exit code, stdout and the set of paths read; stdout is normalized
only in the timing tokens each runner prints (go test's durations, Jest's
`Time:` and `(N s)`, pytest's summary line), with no sorting or dropping.

| Repo | Manifest (f / d / a / l) | Under the sealed prefix | A1-A6 | A7: entries mutated, false hits | A7b: unread files mutated, agreeing | W1-W4 |
| --- | --- | --- | --- | --- | --- | --- |
| cli/cli | 29,878 (23,938 / 5,320 / 620 / 0) | 27,752 (`.stillpass/gomod`) | pass | 29,879, **0** | 10 of 292, 10/10 | pass; W4: 29,843 entries with an empty and a full warm cache, 0 hidden |
| luxon | 24,987 (22,053 / 2,807 / 88 / 39) | 24,800 (`node_modules`) | pass | 24,987, **0** | 10 of 70, 10/10 | no warm state |
| starlette | 25,325 (24,264 / 981 / 76 / 4) | 25,092 (`.stillpass/venv`) | pass | 25,326, **0** | 9 of 9, 9/9 | no warm state |

All three rows are runs of this branch's final code, after the review
fixes (content-derived mtimes, output filtering, EROFS taint, strict A7b
normalizer), rebased on main: luxon and cli/cli with no other Jest or
`go test` job of this experiment beside them, starlette beside luxon (one
pytest job). Each repo had also passed with earlier states of the branch,
with the same counts; one luxon acceptance run and one history run were
lost to OOM kills of a Jest worker and repeated alone (DECISIONS.md). Log:
`polyglot-acceptance.log`.

Notes per repo:

- luxon: A3 edits README.md (Jest reads only test/ and src/). A4 edits
  `src/datetime.js` and A5 adds a file to `src/`. A first full run with
  the session's starting code passed with A4 on `babel.config.js`; the
  picker now prefers a source in a listed subdirectory, and the next run
  then found luxon's husky hook rewriting the tree on commit (see below).
- starlette: README.md is a recorded input (pytest's collection sweep), so
  A3 edits `.github/pull_request_template.md`, the shallowest unread
  markdown file; A4 edits `benchmarks/__init__.py` (the first project
  source in a listed directory) and A5 adds a file to `benchmarks/`. Only
  9 of 151 repo files are unread, all under `.github/`, so A7b samples all
  nine. Each execution probes one random `pytest-cache-files-*` name in the
  root; the comparison treats such an `a` entry as implied by the other
  run's recorded root listing.
- cli/cli: unchanged behavior under the Go profile (same manifest size,
  29,879 entries, as exp/overhead2), W1-W4 included.

## Overhead (A8)

`stillpass bench --interleave --runs 10 --extra-series STILLPASS_SEAL=0`
per repo: plain (same sandbox, no FUSE, no overlay), observed, and observed
with the dependency tree unsealed, alternating run by run, first run of each
dropped. Load averages 33-74 (luxon) and 30-62 (starlette) throughout; full
per-run tables in the per-repo reports. The benches ran on the branch
before the review fixes; those add a content-derived mtime per attribute
reply, timestamp-setting on new snapshot builds and the fingerprint's pyc
scan (about 40 ms in median, measured paired), and were not re-benched.
p95 is not reported: luxon's plain series has a 33.3s outlier that makes
its p95 meaningless.

| Repo | Plain median | Observed median (wall) | Wall overhead | Observed execution median | Execution overhead | Unsealed: wall / execution |
| --- | --- | --- | --- | --- | --- | --- |
| luxon | 6.20s | 9.95s | +60.5% | 7.71s | +24.4% | +92.5% / +57.3% |
| starlette | 12.55s | 14.40s | +14.8% | 13.06s | +4.1% | +16.1% / +4.4% |

What the wall adds beyond execution is per-run fixed cost, the same for
any suite length: the host fingerprint (0.7-1.2s per run at load ~65,
measured paired), importing
the tree (0.87s luxon, 0.34s starlette: hashing ~25k dependency files),
mount 0.06s, output diff 0.09-0.19s, ledger write 0.08-0.10s. Against a
5-6s Jest suite that is most of the overhead; against cli/cli's 17s it was
under a tenth. Luxon's plain series has three loaded outliers (33.3s, 11.0s, 11.4s);
its quietest pair (plain 4.7s, observed 6.8s) is +45% wall, the same
shape.

Execution itself: starlette +4% (the whole pytest run is ~370 requests
sealed); luxon +24% (1,707 opens, consistent with Jest giving every test
file a fresh module registry, so the ~25 src modules are opened again for
each of 58 test files, and with passthrough every open is still a round
trip). The A8 target of 10% holds for the observer on Python and not on
Node. On Node the levers are the open round trips (FUSE over io_uring,
4.4x per request in exp/overhead2's probe) and the fixed costs, which a
cached fingerprint and a stat-keyed import cache would remove.

## Hit rate over history (A11)

Last 40 first-parent commits of each repo, oldest first, step A on every
commit, the ledger persisting; the plain runner is measured on a copy of
each commit's tree (`--measure-plain`).

| Repo | Hits | Stillpass total | Plain total (measured) | Saved | Commits touching no recorded input |
| --- | --- | --- | --- | --- | --- |
| luxon | 6/40 | 429s | 321s | -33.6% | 6 (docs/, site/, README.md) |
| starlette | 0/40 | 605s | 431s | -40.3% | 0 |
| cli/cli (T12, for reference) | 12/50 | | | about break-even (DECISIONS) | |

Why commits miss (first mismatching path of the closest manifest):

- luxon: every commit that could hit did. The six that touch only
  docs/, site/ or README.md hit; the rest change `src/*.js` (23, several
  of them JSDoc-only), tests (2), package.json (4), the version or lockfile
  (3, which `npm ci` turns into a different node_modules), or
  jest.config.js (1). Hits replay in 1.4-4.8s against 7.7-19.2s plain;
  each of the 34 misses pays the observer and the fixed costs on a 4-12s
  suite, and six hits do not repay that at this load (46-100).
- starlette: every commit changes a recorded input: source and tests (27),
  a docs page or AGENTS.md only (6), a dependency bump that changes only
  uv.lock and so the sealed venv (4), a release bumping
  `starlette/__init__.py` (3). The six docs-only misses are pytest's
  collection sweep (a lookup records content); with existence-only records
  they would hit (a projection; see the next step). The 4.3s average gap
  per commit, not broken down per commit, includes the observer, the fixed
  costs, a new snapshot whenever the venv changes (1.1s measured for this
  venv, built by copying because its 0644 files cannot share the 0444
  blobs' inodes), and a lookup that deserializes every earlier 25k-entry
  manifest of the key before missing (on the luxon history, lookup took
  0.07s with one candidate and 0.73-0.87s with 28-33).
- Both repos: step-level memoization of a single-process test runner hits
  only when nothing the runner touched changed, which on these histories
  is docs work. Per-test-file splitting (exp/split, for Go) is the lever
  for the source commits; the lookup-only record is the lever for the docs
  commits on Python.

## Where the Go-shaped assumptions cost something

- **"A stat records content" vs pytest's collection sweep.** pytest
  resolves every file of every directory it collects from, so on starlette
  142 of the repo's 151 files are content inputs (57 of them never opened:
  markdown, docs assets, scripts, `uv.lock`); README- and docs-only commits
  miss. Sound, and the largest avoidable miss source found. Jest's
  `roots: ["test"]` keeps luxon's crawl to test/ and src/.
- **Fast suites make fixed costs visible.** luxon's suite ran in 3.4s and
  starlette's in 6.8s in single observed runs at load ~1, against cli/cli's
  ~17s; the fingerprint (0.7-1.2s at load ~65) and the import of a 25k-file
  dependency tree (0.16-1.0s with the store warm, load-dependent) are a
  larger share of each run. See the overhead section.
- **A carve-out appearing in the tree is a miss.** The snapshot always has
  the carve-outs as empty mount points, so the step sees the same
  node_modules listing whether or not the tree has `.cache`, but the
  recorded listing is the tree's. Over-recording; it surfaced only because
  luxon's own pre-commit hook writes `node_modules/.cache`.
- **The repo's own hooks.** `npm ci` runs luxon's `prepare` (husky), which
  sets `core.hooksPath`; acceptance commits now pass `--no-verify`.
- **Toolchain drift on old commits.** luxon's 2025 commits fail 11 locale
  tests under Node 24's ICU (their CI pinned older Nodes); a failing step
  memoizes like a passing one.
- **Sealed prefixes show the host's directory order.** Snapshot file and
  directory times are now fixed (content-derived and constant), but
  readdir order under a sealed prefix is ext4's (hash order, seeded per
  filesystem), and Python's importlib.metadata and pluggy load plugins in
  that order. The same snapshot key can behave differently on another host:
  a blocker for any shared or remote ledger until sealed prefixes and the
  rootfs (exp/rootfs, same class) are served through one non-logging layer
  with sorted readdir.
- **Writes into a sealed prefix fail.** A tool writing under node_modules
  or the venv outside the carve-outs (.prisma, .svelte-kit, .nx) gets EROFS
  where CI's write succeeds; such a run is shown and not memoized (it
  reports EROFS), and the fix is a carve-out in the profile.
- **Profile env differs from Actions.** `PYTHONHASHSEED=0`,
  `SOURCE_DATE_EPOCH=315532800`, `PYTHONDONTWRITEBYTECODE=1`, npm's
  notifier and logs off, files dated 2000-2008 by content: all in the key,
  so never a false hit, but a recorded result can differ from what Actions
  would produce (DECISIONS.md lists each).

## Next step: record what a name lookup reveals, not the file's content

The single largest source of avoidable misses found here is the schema
rule "a stat records `f` with the content hash". It was written for Go,
which opens what it stats. pytest does not: collecting from the rootdir, it
resolves every file of every directory it recurses into (`_in_venv` probes
`<path>/pyvenv.cfg`, so `<path>` is resolved as a path component and the
kernel answers ENOTDIR; the step learns "not a directory", nothing more).
On starlette 57 of the 142 content inputs outside the venv are, by file
type, such files: every non-.py file but pyproject.toml (31 markdown pages,
docs assets, extensionless scripts, uv.lock). That is an estimate: the
observer logs (inode, kind), not which operation, so it cannot yet say
which files were only looked up. Every docs-only or README-only commit
therefore misses.

The observer can tell the cases apart at the protocol level if file
attributes are made uncacheable (attr_valid 0 for file inodes, entry_valid
kept long): a path walk is a LOOKUP, a stat() of an already-resolved file
is a GETATTR, an open is an OPEN (exp/split already serves files with TTL
0 for per-process attribution, directories keeping theirs because expiring
them costs a request per path component; this needs only the attribute
TTL at 0). A new manifest kind `e` (exists: kind and
mode) would be recorded for a file only ever LOOKUP'd, and `f` (content) on
GETATTR, OPEN or READ, so a step that learns size or content still records
content. What must be proven before trusting it, by an A7b-style audit and
not by argument: no kernel path hands a process a FUSE file's size from the
lookup reply without a GETATTR (statx with AT_STATX_DONT_SYNC does; so may
overlayfs copying attributes into its own inode), and overlayfs's stat of a
lower file always reaches the lower's getattr. Projected effect, not
measured: starlette's six docs- and AGENTS.md-only commits of 40 would
become hits (a lockfile bump still changes the sealed venv); on luxon
nothing (Jest opens what it crawls).

Two smaller follow-ups the corpus points to:

- **A sealed prefix as one manifest entry.** A sealed dependency tree is
  recorded as every entry under it (24,800 for luxon's node_modules,
  25,092 for starlette's venv), so every manifest is ~25k entries, and a
  miss deserializes up to 50 of them (on the luxon history lookup took
  0.07s with one candidate and 0.73-0.87s with 28-33). The snapshot key
  already is a hash over exactly those entries; a manifest entry "this prefix has subtree hash K" (matched
  against the same hash of the tree's subtree) would carry the same
  information in one line, shrinking manifests 15x and lookup with them.
- **Mode-variant blobs for snapshots.** node_modules and venv files are
  0644; blobs are 0444 and a hard link shares its inode's mode, so every
  new snapshot of those trees copies ~25k files (1.1-2.7s here) where Go's
  0444 module cache links. A store-side 0644 twin per blob, made once,
  would let later snapshots link.
