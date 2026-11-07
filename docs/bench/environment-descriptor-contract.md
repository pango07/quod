# Environment descriptor replay regression

[Run 42354463413](https://github.com/pango07/quod/actions/runs/42354463413)
demonstrates a stale replay in the packaged `f9d8153` engine and validates
the source fix in `d6560712114056551d3a5e5911cf60156169ab02`. It uses
Action `41d6c96a2c78b2e1989e1710bae23eb9e165b86b` and Node 24.18.0.

Two synthetic Vitest tests read an environment variable with
`Object.getOwnPropertyDescriptor(process.env, name).value` and
`Reflect.getOwnPropertyDescriptor(process.env, name).value`. The recorder
previously tracked the variable's presence, although these operations also
expose its value without an ordinary property read.

The regression first records, confirms and demonstrates an unchanged-input
hit. It then changes only `CACHE_DESCRIPTOR_INPUT` from `before` to `after`.
Test source, configuration and other fixture files remain unchanged between
training and mutation. Plain Vitest fails both tests on the new value.

| Engine / recorder | Warm files replayed | Changed files replayed | Wrong test states | Quod / plain exit |
| --- | ---: | ---: | ---: | --- |
| Packaged f9d8153 | 1 | 1 | 2 | 0 / 1 |
| Same binary, fixed JS source override | 1 | 0 | 0 | 1 / 1 |
| Release binary built from 2bd297b | 1 | 0 | 0 | 1 / 1 |
| Packaged Action through npm test | 1 | 0 | 0 | 1 / 1 |

The fix records a value dependency for property descriptor access while
preserving the native descriptor. Enumeration can therefore record more
values than necessary; this is conservative. Eight source regressions
cover direct, Object, Reflect and bulk descriptor reads, descriptor
attributes, missing versus empty values, enumeration and disabled recording.
The recorder's existing code hash prevents old records from inheriting
trust across this source change.

The full release-binary gate
[42355061852](https://github.com/pango07/quod/actions/runs/42355061852)
passed at `2bd297b436082c70b540fc9889995cd3d9c2a373`: 118 Rust tests,
73 JavaScript tests, Clippy, existing unit/metadata/type regressions and the
new built-binary environment regression. The formatting artifact is empty.
The release regression uses the binary's embedded recorder with no source
override; its raw reports independently confirm warm replay and correct
invalidation after the value change.

The earlier attempt `42354608781` failed at checkout because its ref input
was an abbreviated SHA; it ran no build or correctness test. The full-SHA
attempt `42354812073` exposed an obsolete regression that required
presence-only enumeration. Commit `2bd297b` changes that expectation to
retain values; direct descriptor regressions remain in place.

Artifact `13566968189` is 41,355 bytes with SHA-256
`8d22d3872885d93b7eaa20295a1c2ee385276ffc6c69ca0990adfdd4a26ddd8e`.
Its archive digest, all six raw reports per variant, warm replay and the
per-test disagreement counts were independently checked locally. Execution
and tests ran on GitHub, not on the Mac.

The release environment evidence is artifact `13567058039`, 22,417 bytes,
SHA-256 `810cc36ee296b4e8702aa4eebd85f8a7cfca2450d0688d5bc8956b10c7d4ee27`.
The release ZIP is artifact `13566984979`, 6,069,939 bytes, SHA-256
`518f12aa2b661e9060bffe4dd2da903a570816e0f1d016add4d12dfdf2101fba`.
Both downloaded archives matched their GitHub digests. The shipped tarball
SHA-256 is `00b99d670054bed66f4111bec627c1366f2b110145abc2efcecd9985cf9af1c0`;
its sole `quod` binary hashes to
`75d703fca2c87c21808a8556f5ca2af47b316527c58426ee008a369c05feae84`.

The packaged Action gate
[42355652832](https://github.com/pango07/quod/actions/runs/42355652832)
passed with Action source `a66fe5379caa93cacbc620784d04ad046b59c290`.
The final Action commit
`a1b9a37a4bf38e1883a4afc07d397616fa985942` contains the exact remote-built
bundle from that gate and ships `bin-2bd297b`. It is on
`exp/cloud-post-save-codex`; this is not a main push or a Cloud deployment.
Older shipped archives were removed from this experimental package; their
original commits remain available.

The gate passed 14 Action source tests, recorder upgrade/downgrade/legacy
checks, configuration and boolean proof checks, the environment regression
through `npm test`, and deferred Cloud-save cleanup. Its separate lineage
fixture produced two promotions, one plain audit and zero disagreements.
Those audit counts belong to the small integration fixture, not a new
convex-helpers window.

Action artifact `13567406963` is 2,491,678 bytes, SHA-256
`2860582c12ea3529f7883f4a8cb18fa46f18b4775cede1abe0d32f7cd496a5ae`.
The bundle tarball SHA-256 is
`7465a417064bf31c5acad3be3fcd18b10c3f3694f30bf7e39977b76b0db5d661`.
All six packaged descriptor reports, the actual warm replay log, changed
exit states and archive contents were independently verified. The committed
bundle files match the artifact byte for byte and pass local syntax checks;
no local build or test suite was run.

The first Action attempt `42355503236` failed its required warm-hit
assertion because the test harness varied its report path, changing the
Vitest command arguments. The corrected harness keeps that argument fixed
and preserves per-phase reports by copying them after each run. It retains
the warm-hit requirement rather than accepting a plain fallback.

This finding does not change the observed zero disagreements in the
[12-commit convex-helpers window](convex-helpers-current-cloud-window.md).
It demonstrates why that corpus cannot establish universal correctness.
This regression is not a performance benchmark and supplies no savings
claim.
