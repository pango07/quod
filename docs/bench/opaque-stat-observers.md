# Native observers and stat-result lifetime

The async stat diagnostic no longer retains its inspected promise and
result until process exit. It also watches native worker, addon and child
process entry points. These are diagnostic changes; the production engine
remains `081f20a`, and async size keys remain disabled.

## Result lifetime regression

[Baseline gate 42161966201](https://github.com/pango07/quod/actions/runs/42161966201)
at `0397956` demonstrates an observation side effect: after an async consumer
returns only the file size, a later Inspector heap query finds **one**
matching Stats object with the diagnostic, versus **zero** in plain Node.
Inspector's remote handles retain the native promise and its result.

The fix releases those temporary handles in `promiseOutcome`'s `finally`
block, including rejected results. The same regression now finds zero
matching objects with and without the diagnostic. This establishes the
tested completed-consumer lifetime behavior, not a complete exclusion of
every possible observer.

## Other observation capabilities

The diagnostic watches actual native functions: the Worker implementation
and its thread-start method, `process.dlopen`, linked bindings, and async
and synchronous child-process spawn functions. Cached functions remain
observable even after the public wrapper is replaced. Node's pinned source
shows the [worker start boundary](https://github.com/nodejs/node/blob/v24.18.0/lib/internal/worker.js)
and [native addon loading boundary](https://github.com/nodejs/node/blob/v24.18.0/lib/internal/modules/cjs/loader.js).

A regression confirms that a worker can connect to the main Inspector and
read a main-only value without a main-isolate `Connection` breakpoint firing.
The earlier worker creation is observed. This is why main-isolate Inspector
connection checks alone cannot authorize a size key.

The initial native-worker object census is descriptive, not a count of
active threads. One public Worker produced two native-prototype objects;
an initial one-object expectation failed in
[42162197235](https://github.com/pango07/quod/actions/runs/42162197235).
The corrected test checks for an increase without equating it to threads.
The cross-thread fixture then exposed its own lifetime bug in
[42162471890](https://github.com/pango07/quod/actions/runs/42162471890): an
Inspector request does not keep the worker's event loop alive. The fixture
now retains a timer until the reply. The final
[gate 42162973816](https://github.com/pango07/quod/actions/runs/42162973816)
at `45a982b` passes **38 observer tests and 65 production proof tests**.

## Actual convex-helpers driver

[Driver gate 42162475605](https://github.com/pango07/quod/actions/runs/42162475605)
at `8073db0` passes with 120 file/project instances and 4,131 unchanged
states. The fingerprint remains
`09ebb9e3a722220d3a21ddfe9f63992a8a074b0fcaa95764e54c37be90d469cd`.
There are zero replays and no new hit-job or cost-window measurement.

| Observation | At all 120 size-read dispatches | Whole diagnostic run |
| --- | ---: | ---: |
| Native Worker construction/start | 0 / 0 | 0 / 0 |
| Native addon loads | 1 | 1 |
| Async child-process spawns | 0 | 120 |
| Sync spawns / linked bindings | 0 / 0 | 0 / 0 |

The one addon is Rolldown's Linux x64 binding. All 120 consumers retain the
existing native-result, hook-identity, constructor-field and early-origin
checks, but **zero** have a history free of opaque entry points. The ledger
still has 299 `sm:`, 257 `sl:`, and 104 `st:` keys. These observations narrow
the current blocker to the already-loaded addon; they do not prove that
the addon actually reads Stats metadata.

## Exact native dependency, still not certified

The benchmark lockfile at convex-helpers `5eaa7e5` selects Rolldown and
`@rolldown/binding-linux-x64-gnu` 1.2.2. Static inspection of the npm archive,
after verifying its SHA-512 against that lockfile, found a 19,336,888-byte
native binary with SHA-256
`09efa62dc8d03fb62f06ab2aadbbc26bde416acb7da20a927f7d53b2bd440322`.
This is the **locked archive's** binary digest; the driver diagnostic has
not independently hashed the file it loaded.

The ELF dynamic table has 124 undefined symbols and only
`napi_register_module_v1` exported. Its imports include `dlopen`, `dlsym`,
`dlclose`, `dlerror`, `dl_iterate_phdr`, and `syscall`, with no direct
`napi_*` imports. Therefore absence of direct V8/Inspector imports cannot
be used as a capability certificate: symbols can be resolved dynamically.

Rolldown's [pinned initializer](https://github.com/rolldown/rolldown/blob/v1.2.2/crates/rolldown_binding/src/lib.rs)
starts a native Tokio runtime. Its
[Cargo lockfile](https://github.com/rolldown/rolldown/blob/v1.2.2/Cargo.lock)
pins `napi` 3.12.0, `napi-sys` 3.3.0, `napi-derive` 3.6.2, and `libloading`
0.9.0. The next proof must account for that native boundary and dynamic
symbol resolution, rather than whitelist the filename, accept matching
outputs as proof, or ignore the addon load. No allowlist or new replay
authorization was added here.

See [machine-readable evidence](opaque-stat-observers.json).
