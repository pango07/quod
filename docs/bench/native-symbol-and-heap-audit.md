# Native symbol resolution and heap readers

The async size optimization remains disabled. This work resolves what the
locked Rolldown binary actually looks up during the real driver run, and
adds detection for a separate metadata observer that requires neither a
new Inspector connection nor a promise hook.

## Native loader evidence

[Gate 42164764622](https://github.com/pango07/quod/actions/runs/42164764622)
at `364aa67` passes a compiled native fixture and the actual convex-helpers
driver. The fixture resolves two N-API functions and `getpid` through
`dlsym`. The diagnostic observes all three, records their providers, and
preserves the fixture's plain result. It also checks path escaping,
truncated-log rejection and a missing post-run binary. Compilation and
execution happened on GitHub, not on the developer's machine.

The glibc audit callbacks return the original symbol addresses and leave
loader flags unchanged. The report is filtered to the actual driver's PID;
141 process log files were collected, but its 163 events describe one
process. This records successful bindings, not calls. Failed lookups,
direct/static calls, some non-PLT relocations, and raw memory access are
outside this diagnostic's coverage. It cannot certify an addon by itself.

| Actual driver observation | Result |
| --- | ---: |
| File/project instances | 120 |
| Test states matching the established fingerprint | 4,131 |
| Replayed files | 0 |
| Async size consumers | 120 |
| Loaded native addon paths | 1 |
| Distinct dynamic symbols resolved by that addon | 126 |
| Those present in the fixed N-API loader source | 125 |
| Other dynamic symbols | 1 |

The addon path is Rolldown's Linux x64 GNU binding. Its **post-run** digest
is `09efa62dc8d03fb62f06ab2aadbbc26bde416acb7da20a927f7d53b2bd440322`,
matching the previously verified lockfile archive. This establishes the
contents at that path after execution; it does not independently prove
that the mapped bytes were immutable throughout execution.

Of the dynamic resolutions, 124 are N-API functions and one is `uv_run`,
all provided by the Node executable. The remaining symbol is
`__pthread_get_minstack`, provided by libc. Rolldown pins Rust 1.97.1;
that version's [thread implementation](https://github.com/rust-lang/rust/blob/1.97.1/library/std/src/sys/thread/unix.rs)
looks up this symbol to account for thread-local storage when sizing a
thread stack. There were no observed dynamic resolutions of V8 or
Inspector heap APIs. That absence is an observation of this run, not an
exclusion of every possible native observer.

## Source correspondence

Rolldown tag `v1.2.2` resolves to
`872b98ac7476eb7d5892a2913e4ba010d124c6ac`. The npm registry's provenance
statement for the locked archive names that same revision and upstream
[publish run 30801837729](https://github.com/rolldown/rolldown/actions/runs/30801837729).
The statement's archive digest matches the lockfile. Its signatures have
not been independently verified in this work; this is not a reproducible
build attestation verified by Quod.

The reviewed `napi-sys` 3.3.0 loader enumerates compile-time function names
and resolves them from `libloading::Library::this()`. Its source declares
158 potential symbols across feature levels. The three initial crate
archives and eight additional N-API adapter/macro archives were checked
against Rolldown's Cargo.lock checksums before static inspection. Their
identities, the fixed symbol list and observed providers are retained in
the JSON evidence. No downloaded source was built or executed locally.

This narrows the source review to N-API handle access, callbacks, buffer
ownership, and native input access. N-API still permits calling JavaScript
and accessing global objects. Matching this symbol list therefore does
not establish that an addon cannot arrange a metadata observation.

## Public V8 heap readers

Node's pinned [heap utilities](https://github.com/nodejs/node/blob/v24.18.0/lib/internal/heap_utils.js)
show that `v8.queryObjects()` calls a native heap query and can return
object summaries, independently of an Inspector session. The new fixture
uses an accepted local consumer:

```js
async function size(file) {
  try {
    const s = await fs.promises.stat(file);
    sink(s.size);
  } catch {}
}
```

The sink receives only a number. Nevertheless, `queryObjects()` finds the
live Stats and its `mtimeMs`. Promise and async hook registries are empty,
and no Inspector connection entry is observed. Caching the public query
function and later replacing its export does not prevent this observation.

Commit `0a7501d` watches the actual native query, snapshot-stream,
snapshot-file, V8-flag and near-heap-limit snapshot entry points through
their pinned runtime bindings. Unknown binding shapes make the opaque
observation unavailable. The fixtures cover the public and cached query,
both snapshot paths, runtime flag changes, near-limit setup, and a control.
Snapshot contents stay in disposable fixture storage and are deleted;
they are not printed or uploaded.

[Gate 42165376974](https://github.com/pango07/quod/actions/runs/42165376974)
passes 45 observer tests, including seven new heap cases, and 65 production
proof tests. The native loader and regression gates use Ubuntu 26.04 image
`20261101.186.1`.

[Driver gate 42165380136](https://github.com/pango07/quod/actions/runs/42165380136)
at `0a7501d` also passes on that image, with 120 file/project instances and
the same 4,131-state fingerprint. All 120 size consumers have zero observed
heap-reader entries at their checkpoints. The whole run records one addon
load and 120 child-process spawns, with no watched V8 heap-reader entry.
There are still zero replays and the ledger retains 299 `sm:`, 257 `sl:`
and 104 `st:` keys. The addon remains the unresolved opaque boundary;
zero heap-reader entries alone does not authorize reuse.

## Remaining proof and benchmark

No addon allowlist, narrower async key, Action bundle or production engine
change is enabled by this evidence. The next step is a bounded native
capability argument tied to load-time integrity, followed by result-lifetime
exclusion, production integration and Rust query equivalence. An unknown
case must retain the full metadata input.

The corrected 12-commit window, billed-minute reduction and best
`floor + N s` hit remain unproven. These diagnostics are not performance
benchmarks. See [machine-readable evidence](native-symbol-and-heap-audit.json).
