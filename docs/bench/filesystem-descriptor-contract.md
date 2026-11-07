# Descriptor metadata: stale replay and conservative correction

[Gate 42360842978](https://github.com/pango07/quod/actions/runs/42360842978)
at `800771d09bb7f0786cc108cc0c84564924612f50` demonstrates a correctness
defect in packaged `bin-2bd297b` and verifies the staged source correction.
The test files and input bytes remain unchanged; only input timestamps move.

The old JavaScript recorder records file content when `open()` is called,
but does not record metadata exposed through `fs.fstat`, `fs.fstatSync` or
`FileHandle.stat()`. After a genuine warm replay, a metadata-only mutation
therefore reuses three passes that are three failures under plain Vitest.
An ordinary content-reading test remains passing throughout.

| Phase | Warm files replayed | Files replayed after mutation | Wrong assertions | Quod/plain exit |
| --- | ---: | ---: | ---: | --- |
| Shipped baseline | 2 | 2 | 3 | 0 / 1 |
| Corrected source | 1 | 1 | 0 | 1 / 1 |
| Old ledger with corrected source | 2 | 0 | 0 | 1 / 1 |

The correction marks public descriptor metadata calls unobservable. It also
marks `fs.promises.open` unobservable because the returned handle's methods
call internal bindings directly, bypassing public `fs` wrappers. This is a
deliberately conservative support restriction: those files run normally.
It does not guess a pathname for an inherited, reused, unlinked or anonymous
descriptor. The ordinary pathname content-reading control still replays.

Eight recorder regressions cover value preservation, disabled observation,
both descriptor call styles, FileHandle metadata, an unlinked descriptor and
ordinary content reads. The packaged experiment executes six runs in each
of three phases. The upgrade phase shows that the changed recorder source
invalidates the old ledger; no manual deletion substitutes for that check.

[Verified evidence](filesystem-descriptor-contract.json) retains the exact
source, old Action/binary pins, artifact digest and independently recomputed
wrong/replay counts. The source-override gate is distinct from the replacement
release and packaged-Action gates. It supplies no new benchmark savings and
does not certify every possible filesystem or runtime observation.

## Gated release binary

[Release gate 42360988903](https://github.com/pango07/quod/actions/runs/42360988903)
passed at the same source commit: 118 Rust tests, 81 JavaScript tests,
Clippy, the release build, the existing metadata/type/environment regressions
and the new descriptor regression. The actual embedded binary, without a
source override, retains one content-control replay and agrees with all
three plain metadata failures. Its artifact and per-test evidence were
downloaded, digest-checked and independently compared.

Release archive SHA-256:
`51262a23754deffb5cfdf8f47cb5e4acbaa577487329d3f4f46e4709a5719205`.
Contained binary SHA-256:
`db7f59a22391cd2e1553a10d2b5ca8f47eda41c95727e91e75c1c6632c680b61`.
## Gated Action package

[Action gate 42361428513](https://github.com/pango07/quod/actions/runs/42361428513)
passed against source-stage Action `ecb4b229a545bf10845116253d29312519c0fb44`.
It built the Action remotely and exercised its actual npm/node shim with the
new embedded binary. The six descriptor-regression executions retain four
assertions each. Both the warm and timestamp-mutated runs replay the content
control; the three metadata failures agree with plain, with zero wrong
assertions. The downloaded reports and replay logs independently confirm
those counts.

The same gate passed recorder-upgrade, configuration, boolean, environment,
deferred-save and cleanup regressions. Its lineage fixture promoted two
records and ran one plain audit, with no disagreement or revocation. These
are fixture results, not lineage activity in the full benchmark window.

Action `57269ba1c1b43ae003dd64a5a5cc6c0df8f0e4b3` contains those exact
remotely built bundles, verified against the downloaded artifact. The bench
workflow pins that Action and `bin-800771d`. This is an experimental-branch
correctness update, with no new full-window performance result.
