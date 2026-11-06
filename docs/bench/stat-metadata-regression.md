# Metadata-only edits can invalidate a passing test

The [packaged regression](https://github.com/pango07/quod/actions/runs/42129862480)
reproduced a stale replay in binary `ae52750`, packaged Action `8b0ad91`,
with probe `1182514`. Five tests passed plain, recorded, confirmed plain,
and replayed. Changing only the input's modification time and permissions
then produced **five stale passing states and exit 0**, versus five failures
and exit 1 in plain Vitest. The bytes did not change. The readers were
`statSync`, promise `stat`, callback `stat`, `lstatSync`, and the `mode` field.
Lineage was disabled: this defect predates lineage promotion.

This confirms the previously documented general stat-metadata gap. The
convex-helpers window's zero mismatches remain valid for that corpus, but
do not establish safety for metadata-dependent tests or universal correctness.

Candidate `baef88b` introduces separate `sm:` and `sl:` metadata inputs.
They retain the real path even inside installed packages, and preserve the
native Stats object. Linux `statx` keys device/inode, mode, ownership, link
count, size/blocks, and access/modification/change/birth timestamps. Unknown
support cannot yield a reusable key. Proven type-only reads keep their
bounded `st:` key. A lockfile cannot stand in for installed-file metadata
when deciding whether a job may skip installation.

The corrected binary at `8aec5c5` passed
[gate 42131084528, attempt 2](https://github.com/pango07/quod/actions/runs/42131084528):
98 CLI unit tests, one unit integration test, 16 recorder tests, 44 adversarial
proof tests, clippy, and the original end-to-end case. Unchanged inputs still
replayed; the metadata-only edit then executed the file and returned all five
failures, matching plain with **wrong = 0**, exit 1, and zero files replayed.
The first attempt hit the previously intermittent Go-toolchain layout test;
the exact-commit rerun passed. An earlier compile error in the fallback ID
constructor and outdated metadata-key assertions were corrected before this
successful gate.

Release archive SHA-256:
`5af462b810d880c78af7b2619b885d34405b6b8c9eb8ad662615ff02690fc76c`.
Action `afb1121` packages this binary. Its
[integration gate](https://github.com/pango07/quod/actions/runs/42132580268)
passed lineage and recorder-upgrade checks, then rejected a missed replay
after a discarded dependency-version edit: `sm:package.json` changed.
Per-test results still agreed. The bounded file-type proof was tied to the
Boolean-manifest switch, leaving names-only mode unnecessarily broad.
Candidate `6c05ad0` separates these switches without weakening the metadata
fallback. It also runs the metadata regression with proofs off, names-only,
and both enabled. Its
[gate 42136777526](https://github.com/pango07/quod/actions/runs/42136777526)
passed 161 tests, clippy, the release build and all three end-to-end modes.
Each mode replayed the unchanged record, then executed after the
metadata-only edit with five failures matching plain and wrong = 0.

No current performance claim applies to the correction. Fresh checkouts
and dependency restores change metadata
even when bytes match, so performance must be remeasured.

The [configuration diagnostic](https://github.com/pango07/quod/actions/runs/42131247085)
selected the same nine projects and 120 file/project instances. With the
existing type-only proof enabled, 40 paths used `st:` while 17 `sm:` and
12 `sl:` paths retained complete metadata. Eight unresolved call sites came
from Node's asynchronous realpath implementation, Vite environment/resolver
helpers, Vitest project discovery, and fdir. The probe executes no tests,
creates no ledger, and authorizes no replay. These are leads for bounded
proofs of what each consumer observes, not permission to ignore metadata.

This regression covers path-based stat/lstat. It does not certify every
descriptor-based filesystem API or close other recorder coverage questions.

A further bounded proof at `15cb5ab` accepts a native stat helper's private
`const` only when all references are consecutive, type-only `if` conditions.
Earlier successful branches must exit before a later type check; escapes,
metadata readers, arbitrary intervening calls, direct eval and modified
Stats prototypes retain full metadata. The
[actual-config and adversarial gate](https://github.com/pango07/quod/actions/runs/42137117720)
passed. It preserved the configuration/selection fingerprints and all nine
projects/120 file instances. Type-only observations increased from 669 to
680, broad stat observations fell from 63 to 52, and the unique complete
metadata keys fell from 17 `sm:` plus 12 `sl:` to 15 `sm:` plus 12 `sl:`.
The remaining observations still require their own proof; this diagnostic
executes no tests and establishes no new runner-time savings.

Engine `15cb5ab` also passed the full
[gate 42137374544](https://github.com/pango07/quod/actions/runs/42137374544):
164 tests, clippy, release build, and the metadata regression in all three
proof modes. Each unchanged record replayed; each metadata-only mutation
executed and agreed with plain (wrong = 0, exit 1, zero replayed files).
The release archive SHA-256 is
`eb75a5194e2cb97c443597299327b26bfe288b31730ff412b48237719d7d6db6`.

Action `9cf6d91` now ships that binary and the exact compiled bundle from
[successful packaged gate 42137916242](https://github.com/pango07/quod/actions/runs/42137916242).
The names-only discarded-version replay, lineage audit, recorder upgrades,
Boolean switch cycle and metadata-reader fallbacks passed. The subsequent
[fresh-runner probe](metadata-fresh-runner-probe.md) matched all per-test
states, but replayed no Vitest files because conservative metadata keys
changed across checkouts. It does not establish new full-window savings.

## Direct synchronous type readers

Engine `ebdf96d0c569dc5c4ccd7000013c3b737387ad58` extends the bounded type
proof to direct native `statSync` calls. It checks the actual immediate V8
caller and executed source, accepts immediate type/existence consumers and
private type branches, and rejects metadata access, escapes, unknown options,
bigint options, nested argument calls and changed Stats/Object prototypes.
Type-key failures now distinguish native error numbers, so a caller's catch
can distinguish a missing path from a non-directory path component.

[Gate 42140626596](https://github.com/pango07/quod/actions/runs/42140626596)
passed 167 tests, clippy, release compilation, and the metadata regression in
all three proof modes. Each mode retained wrong = 0 and reran the changed
metadata reader. A new built-binary test also verified the useful case:
changing only time and permissions still replayed one proven type reader;
changing that input from a file to a directory replayed zero files and
selected both tests, matching plain. The archive SHA-256 is
`9649172196b66af39155788824d46d7e2191e8fd7ab622da5b083f1f48fc7ea9`.
This intermediate engine was not packaged; the later `4d99319` package
below includes its direct type proof plus the internal-constructor guard.

The [complete-suite diagnostic](https://github.com/pango07/quod/actions/runs/42140629979)
matched all 120 file/project instances and 4,131 test states. It removed
complete metadata dependencies for the checkout root, `example-ui` and
`packages/convex-helpers`, replacing them with proven type dependencies.
Type observations increased from 771 to 783; broad stat observations fell
from 52 to 40. Unique `sm:` keys fell from 288 to 285 and `st:` keys rose
from 101 to 104, with 12 `sl:` keys unchanged. Temporary cache paths differ
between diagnostics and are not additional demonstrated product blockers.
This is a dependency refinement, not a measured whole-job speedup.

## Async stat observations remain complete

[Gate 42141395235](https://github.com/pango07/quod/actions/runs/42141395235)
passed 27 proof regressions, including an observer escape: a function only
uses the size of its awaited stat, while an `async_hooks` callback independently
reads the modification time from that promise. The record must retain `sm:`.
Proving the local size read alone would miss a real consumer.

The passive [provenance diagnostic](https://github.com/pango07/quod/actions/runs/42143434404)
at `caa4245` observed 354 native stat promises: 120 file/project reads covering
39 unique test paths, plus 234 temporary-cache reads. All 354 settled from
their own native continuation with pristine Stats prototypes at the observed
entry, without inserting `.then` reactions or changing returned values.
The full test fingerprint still matched. This identifies the creator; it
does not establish absence of other consumers or authorize a narrower key.

Its legacy `process.binding('async_wrap')` hook counters were unavailable,
not zero. Node's [v24.18.0 binding allowlist](https://github.com/nodejs/node/blob/v24.18.0/lib/internal/bootstrap/realm.js)
does not expose that binding. A diagnostic must not treat missing counters
as evidence that no observers exist. Unknown asynchronous consumers and
Node's realpath callbacks continue to retain complete metadata.

See [machine-readable proof progress](metadata-proof-progress.json). The
historical 45.2% runner-time / 41.7% rounded-minute reductions and floor + 3 s
hit still do not apply to this corrected engine. A fresh real-job hit is
required before the next full 12-commit window is useful.

The next [runtime diagnostic at 73ac7eb](https://github.com/pango07/quod/actions/runs/42144976636)
read the actual Node observer registries through Inspector without modifying
them. The workload step passed and retained the same full test fingerprint.
Initially both registries were empty. At dispatch and native-creator entry
for all 120 test-file stats, async hook counts were zero and each promise
hook list contained one callback—the diagnostic's installed observer.
All 354 native continuation traces still matched. These are checkpoint
measurements, not a proof of callback identity or absence of intervening
observations. This probe does not use the benchmark's report preload or
the complete production driver/worker setup.

That workflow's overall result was **failure**, because its async-hook
fixture incorrectly expected promise-dispatcher removal to be immediate.
Node defers removal to a microtask. The corrected
[isolated gate at 722f588](https://github.com/pango07/quod/actions/runs/42145373987)
passed all four cases: no observers, public promise hooks, async hooks
including deferred teardown, and an observer that reads modification-time
metadata through `util.inspect` without attaching a `.then` reaction.
An earlier scope lookup failure was also fixed by reading the live public
AsyncHook field bindings rather than assuming numeric offsets.

The registries make a guard worth investigating, but no async size-only
key is enabled. A production proof must establish native/preload provenance,
the actual consumer and stat outcome, relevant observer changes between
checkpoints, and safe fallback when any of that is unknown. Checking only
the local `.size` read or counting additional `.then` reactions is insufficient.

## Executed consumers and intermediate promise checkpoints

The consumer certificate at `c45ba09` accepts only a private `const` initialized
by an immediately awaited stat call, followed by a primitive size sink, with
an empty catch and no finalizer. It rejects options, aliases, returned Stats,
metadata readers, closures, eval, intervening statements and observable error
handlers. Runtime matching uses the actual V8 script and call offset, including
tests against rewritten source files and spoofed source URLs. **This is only
the consumer half of a proof; the recorder does not use it to narrow keys.**

[Gate 42146929273](https://github.com/pango07/quod/actions/runs/42146929273)
passed all 30 consumer/stat/Boolean regressions, four observer fixtures and
the full suite. The certificate recognized all 120 `FilesStatsCache.updateStats`
calls, covering 39 unique test paths. The 234 temporary-cache stat calls did
not qualify. All 4,131 test states matched the existing fingerprint, and the
285 `sm:`, 12 `sl:` and 104 `st:` keys were retained unchanged.

The observer trace now compares actual callback identities, not just counts,
and follows every promise created inside the stat call plus its awaiting
consumer. [Gate 42147365686](https://github.com/pango07/quod/actions/runs/42147365686)
passed seven fixtures. Two important cases are now explicit: replacing a
callback without changing the registry count, and attaching an observer during
I/O that removes itself before the root stat promise resolves. The latter
case looks clean at dispatch and native-result construction but is caught at
an intermediate binding promise. The probe still preserves the native Stats
prototype/data fields and inserts no `.then` reaction.

The combined [workload step at 10e62db](https://github.com/pango07/quod/actions/runs/42147926900)
matched all 4,131 states again. All 354 stat operations retained only the
diagnostic's exact callbacks at their 5,190 observed checkpoints; all 120
test-file calls also matched the size-consumer certificate. Native root
continuations matched in all 354 cases. This remains a main-process diagnostic
without the real-job report preload, a production ledger, or replay.

That workflow's overall result was **failure**: a new assertion incorrectly
equated the public `fs.Stats` constructor with Node's internal constructor.
The [Node 24.18.0 source](https://github.com/nodejs/node/blob/v24.18.0/lib/internal/fs/utils.js)
exports a deprecated wrapper sharing the native prototype. The corrected
[seven-case gate at 94124e6](https://github.com/pango07/quod/actions/runs/42148398507)
passed and verifies that distinction. The executed `internal/fs/promises`
and `internal/fs/utils` source hashes exactly match the primary Node source;
they are recorded in the machine-readable evidence. Names and source hashes
alone do not attest all closed-over native bindings or earlier preload effects.

The next production step must bind that native state and exclude unsupported
observers before using a size query. Unknown startup code and transient native
changes must retain complete dependencies or disable reuse. No size-only
record is currently emitted. Separately, the remaining callback reads include
Node's realpath traversal and fdir's classification of README/CHANGELOG paths.
Realpath reads device/inode identity for symlink caching, so blanket type-only
conversion of its lstat calls would still be unsound.

## Guard the actual native constructor

The wrapper distinction exposed a production guard defect. Changing the
internal constructor's `prototype` leaves the public `fs.Stats.prototype`
unchanged. A replacement `isFile()` can then read modification time while
the old guard still emits a type-only key. The isolated
[baseline at 94c8d18](https://github.com/pango07/quod/actions/runs/42149263438)
failed exactly that new regression: 56 of 57 proof tests passed, with the
new test missing its required `sm:` dependency. This demonstrates incorrect
key classification; it is not an old-binary stale-replay measurement.

Engine `4d993190050f71ec10a6c825328807b14c415b7a` now checks the actual
constructor's prototype as well as the public wrapper and the complete
captured prototype chain. It reads those properties through descriptors.
The diagnostic also detects replacement during I/O before the native result
is created. Unknown or changed shapes retain complete metadata.

[Audit and binary gate 42149721287](https://github.com/pango07/quod/actions/runs/42149721287)
passed 172 unit, recorder, integration and proof tests, clippy, release
compilation, and the built-binary regressions. All three metadata modes
reran changed inputs and matched plain (wrong = 0). The type-only reader
still replayed after a metadata-only edit and reran after a file-to-directory
change. The new replacement-prototype case also warmed successfully, then
reran after only a timestamp changed, selected two tests and matched plain.
Its result is `prototypeMetadataReplayed: 0`, `prototypeSelectedTests: 2`,
`wrong: 0`. Archive SHA-256:
`98dfb3addc02724c3fd185b84fbcebadc2db486774d7822f24cea7f2eb487fe0`.
Run 42149545364 selected no build job and is deliberately not validation
evidence.

[Full-suite diagnostic 42149546225](https://github.com/pango07/quod/actions/runs/42149546225)
passed all 31 proof regressions and eight observer fixtures. The 120 files
and 4,131 states retained fingerprint
`09ebb9e3a722220d3a21ddfe9f63992a8a074b0fcaa95764e54c37be90d469cd`.
All 354 stat operations retained their own native continuation and the
diagnostic's callback identities across 5,190 observed checkpoints. The 120
size consumers were recognized, but metadata keys remain unchanged at
285 `sm:`, 12 `sl:` and 104 `st:`. No async size-only key or new savings
claim is enabled by this work.

Action `d2bb917792130dcaf3fafec066b481bec5c2f674` ships that archive and
the exact compiled JavaScript bytes from
[packaged gate 42150430450](https://github.com/pango07/quod/actions/runs/42150430450).
The five Action tests, real lineage/switch sequence, recorder-version
boundary and names/Boolean switch regressions all passed. Its lineage
fixture promoted two records and ran one plain audit with zero disagreements;
these are fixture counts, not new convex-helpers window results.
Harness `99d0ca5af335d82fee1b7bbd37a35d4b251a376a` pins that Action,
binary version and archive hash. No current full-window or best-hit claim
has been established by this packaging gate.
