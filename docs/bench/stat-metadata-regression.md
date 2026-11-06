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
