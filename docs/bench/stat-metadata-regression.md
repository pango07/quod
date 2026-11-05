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
This binary is not yet packaged into the Action, and no current performance
claim applies to it. Fresh checkouts and dependency restores change metadata
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
