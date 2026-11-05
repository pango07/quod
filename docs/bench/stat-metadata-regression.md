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

The candidate is not yet validated or packaged. Its first remote build
found an incorrect ULID constructor name; the follow-up uses the repository's
existing `Ulid::generate()` API. The gate includes Rust metadata-state tests,
recorder tests, proof regressions, clippy, and the original end-to-end stale
replay case against the newly built binary. Performance must be remeasured:
fresh checkouts and dependency restores change metadata even when bytes match.

This regression covers path-based stat/lstat. It does not certify every
descriptor-based filesystem API or close other recorder coverage questions.
