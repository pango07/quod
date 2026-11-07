# Pinned WASI bundler feasibility

[Gate 42353101693](https://github.com/pango07/quod/actions/runs/42353101693)
passed at `d29561a0738dd8aeafc40eb26c933703a58039be`. It ran original
convex-helpers c11, `5eaa7e5160736054d53f8b71e2c522835eb427e0`, with Node
24.18.0, engine `bin-f9d8153`, and image `ubuntu26/20261101.186.1`.

| Phase, in execution order | Suite command seconds | Observed addon load events |
| --- | ---: | ---: |
| Native control before | 99.777 | 1 |
| Strict WASI | 99.811 | 0 |
| Recorded strict WASI | 106.000 | 0 |
| Native control after | 99.128 | 1 |

Each phase completed 120 file/project instances and 4,131 test states:
4,130 passed, one skipped. Independent comparison of the raw reports
confirmed identical file names, test names and states, preserving duplicate
instances. This does not prove identical behavior in every other respect.
All phases have the load observer; the recorded phase also enables the
existing optional names/type and boolean proofs. The fixed order, single
runner and instrumentation prevent a causal or general performance claim.
These are command times, not whole-job times or billed minutes.

The WASI phase explicitly uses `NAPI_RS_FORCE_WASI=error`. The exact
published Rolldown 1.2.2 loader supports this strict selection. Seven
dependency versions were pinned and checked. The separate package install
did not run install scripts. Its WASM file is 10,548,697 bytes, SHA-256
`52f234969469c4dd2d399a381d56928e4e1466350829348d04138d4762dd6ae4`.
The downloaded npm archive matched its SHA-512 integrity, and that file's
hash independently matches the remote artifact.

The observer records a file per process/thread isolate, including before
delegating to `process.dlopen`. The original result's `observedProcesses`
field counts those files, not distinct process IDs. The independent JSON
corrects that interpretation. Most worker reports lack a normal exit
checkpoint. Initial snapshots and logged calls remain evidence, but neither
those reports nor absence of observed loads proves complete native provenance.

Static parsing of the verified WASM binary found 114 imports: 89 `env`,
one `emnapi`, two `napi`, 21 `wasi_snapshot_preview1` and one `wasi`.
They include global-object/property access, JavaScript calls, filesystem
operations, shared memory and thread creation. The published loader preopens
the filesystem root and uses Node worker threads and filesystem proxies.
Replacing an addon with this package therefore does **not** establish an
isolated execution environment or complete input tracking.

There were zero replays, and metadata inputs remained complete where no
existing optional proof applied: 299 `sm:`, 257 `sl:`, 104 `st:` keys.
No size projection, production package or Action behavior was enabled.
This experiment establishes workload compatibility worth considering in an
architecture review; it does not resolve the replay boundary.

Artifact `13566123084` is 545,380 bytes, SHA-256
`3fe007c89d72227b23dc5a27d4d00abfe5d62bf98d7f5218f00718cba9ef4a5a`.
The digest was verified before extraction. [Independent evidence](wasi-feasibility.json)
retains report hashes, package versions/integrity, phase counts and the full
static import inventory. The [full 12-commit result](convex-helpers-current-cloud-window.md)
remains the authoritative product measurement.
