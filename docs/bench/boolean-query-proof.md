# Bounded manifest queries: validation before a real-job benchmark

The next candidate targets c4's `exports` edit in the existing convex-helpers
window. A resolver that asks only whether `exports` is truthy need not depend
on every entry in that map. This must be proved at the read, not inferred
from matching test output or a trusted source filename.

Implementation: `ba0f05a`, branch `exp/boolean-exports-codex`. The
`QUOD_VITEST_BOOLEAN_PROOF=1` switch is opt-in and is itself a recorded input.
The packaged diagnostic exposed a compatibility gap: the old code hash
invalidated saved full-output renders, but did not invalidate ledger records
before witness selection. A fix now binds the whole ledger to the actual
recorder and key semantics. It requires fresh observation after a code change;
its remote regression gate is pending. The `ba0f05a` binary does not have this fix.

The proof reads actual executed ESM source from V8 and certifies a bounded
set of boolean language contexts. Any ordinary value reader remains keyed.
Every whole manifest read must pass through the watched JSON parse before a
broad key can be replaced. Top-level names, queried values, truthiness,
existence and access outcomes remain inputs. Invalid JSON anywhere in the
document retains the whole-file hash, including invalid unread fields.

Vite also stats these files. A separate bounded proof recognizes a direct
native `statSync` helper with an empty catch, consumed immediately through
`isFile`/`isDirectory` or a boolean success check. Its key follows symlinks
and retains file/directory/other/unavailable state. Prototype changes,
escaping results, metadata consumers and unknown source shapes retain the
broader dependency. This does **not** close the previously documented general
stat-metadata observation gap.

The first full-suite probe correctly rejected the candidate: Vite's
`isFileReadable` observations had not been proved. The read-site diagnostic
identified these as boolean success checks; the proof was extended with
regressions, without dropping unknown observations.

Validation:

- [Linux Rust, recorder, adversarial proof and clippy gate](https://github.com/pango07/quod/actions/runs/42114637800)
  passed; its binary is `bin-ba0f05a`, SHA-256
  `6f3b99ff838cb8da92e8e1c79530cfb4e9103851bb12bdabd85a3cd1b8329f14`.
- [Full-suite query gate](https://github.com/pango07/quod/actions/runs/42114642706)
  passed with 120 file/project instances and 4,131 per-test states in each
  historical manifest state: c3, c4 and c5. Each suite executed fully. No
  replay was authorized by this diagnostic.
- Independent artifact comparison also matches the earlier conservative-key
  diagnostic. For c3/c4, all 29 root and 33 nested-manifest reads were parsed
  and watched. The resulting queries retained `name`, dependency names and
  `exports` truthiness; existence, type and access keys remained.
- The deleted nested manifest in c5 remains an existence/type input. Its
  removal is not treated as equivalent to a present manifest.

The packaged replay gate and a short paired real-job chain must pass before
this can claim a performance improvement. The authoritative full-window
result remains 0.548x plain runner-seconds; this document adds no savings
to that result. The experimental Action also bounds Vitest interception to
stable 3.2–3.x, 4.x and 5.x, leaving future majors and prereleases plain.
