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
its [remote regression gate](https://github.com/pango07/quod/actions/runs/42117764717)
passed at `9f50dbf`: 97 CLI tests, 1 unit integration test, 15 recorder tests,
44 proof tests and clippy. The `ba0f05a` binary does not have this fix. The
replacement is `bin-9f50dbf`, SHA-256
`3fda1ab2288fd56eba076aef37a825d1120f161d683aa975a8545c1c54b7f544`.

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

**Diagnostic limitation discovered afterward:** that probe imported Vitest
before hooking the filesystem. Vite captures an async read function during
import, so the probe missed PostCSS configuration-search reads. The packaged
Action hooks before import and correctly retains the whole manifest: its
[cold diagnostic](https://github.com/pango07/quod/actions/runs/42117490345)
observed nine root-manifest reads but only eight watched parses. The extra
read flows through an async configurable loader, which this proof cannot
certify. The probe now uses the product's import order. The earlier matching
test states remain valid observations, but the refined-key result does not
prove full input coverage in the product.

The [corrected short probe](https://github.com/pango07/quod/actions/runs/42118232412)
confirms this on the unchanged convex-helpers workload: 31 root reads versus
29 watched parses, and 41 nested-manifest reads versus 33 watched parses.
Both manifests retain broad dependencies. This candidate therefore does not
yet remove the c4 invalidation. No new full-window benchmark is justified by
the current proof; the async consumer needs its own sound proof or another
optimization must supply the remaining savings.

The packaged fixture explicitly separates a bounded positive case (inline
PostCSS options, so no async search) from the default async-search negative
case. The negative case must execute and agree with plain after a manifest
change. This does not alter the convex-helpers benchmark or remove its reads.

The [packaged upgrade gate](https://github.com/pango07/quod/actions/runs/42118349818)
passed recorder upgrade/downgrade/legacy-record invalidation, lineage audits
and the names proof. Its isolated boolean positive case replayed correctly,
but the switch-off regression exposed a second omission: the flag was emitted
as an input, then hashed as a constant volatile environment variable. It now
uses the same value-keyed exemption as the names-proof switch, with an
explicit regression independent of the exemption list. The corrected [packaged gate](https://github.com/pango07/quod/actions/runs/42122296319)
passed at engine `ae52750`, including the off/on cycle, async-loader fallback,
export-value/shape changes and stat-size consumers. Every fixture's per-test
states matched plain. The preceding [Rust/recorder/clippy gate](https://github.com/pango07/quod/actions/runs/42118675755)
also passed (attempt 2, after rerunning the unrelated failed Go toolchain test).
The shipped candidate is `bin-ae52750`, archive SHA-256
`9cbb48d36f41f79ee9d3470285c803dddba7651e3992f39687aa6838869bb41a`,
in action `8b0ad91`. Its `dist` bundle is the exact remote-gate artifact.
These correctness regressions do not establish a new performance result.

A short paired real-job chain must still establish a performance improvement
before another full window is justified. The authoritative full-window
result remains 0.548x plain runner-seconds; this document adds no savings
to that result. The experimental Action also bounds Vitest interception to
stable 3.2–3.x, 4.x and 5.x, leaving future majors and prereleases plain.

## Async-consumer diagnostic

[Run 42125166984](https://github.com/pango07/quod/actions/runs/42125166984)
at `29f2188` selected all 120 file/project instances and reproduced the
31/29 root and 41/33 nested read/parse counts. It did not execute the tests
or create replayable records. All ten additional reads returned Buffers:
two root and eight nested. Each Buffer was decoded once at Vite 8.2.0's
`String(await fsReadFileAsync(filepath))` call, and matching-text parse
candidates used its `jsonLoader` from the package-manifest branch of search.
The diagnostic retains the broad keys and marks its recorder unobservable.
Buffer identity proves which read was decoded; primitive-text equality does
**not** prove which private value reached a parse.

This narrows the remaining proof to one actual executed consumer shape. A
production proof would have to bind the direct native String consumer, the
private raw-text binding and every possible use, the actual pure JSON loader,
and the matching parse invocation. It must reject escapes, custom loaders,
mutable decoder behavior, intervening callbacks and unknown paths. Merely
extending the synchronous pending-parse lifetime across an await is unsound.
No async reader is currently granted a refined key.
