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

The fix records a value dependency for property descriptor access while
preserving the native descriptor. Enumeration can therefore record more
values than necessary; this is conservative. Eight source regressions
cover direct, Object, Reflect and bulk descriptor reads, descriptor
attributes, missing versus empty values, enumeration and disabled recording.
The recorder's existing code hash prevents old records from inheriting
trust across this source change.

The full release-binary gate is
[42354812073](https://github.com/pango07/quod/actions/runs/42354812073).
Its result and Action packaging must be verified separately before claiming
the shipped fix is validated. The preceding attempt, `42354608781`, failed
at checkout because its ref input was an abbreviated SHA; it ran no build
or correctness test. The replacement uses the full
`9ed437a819b1a2a2f42ce25de0bf7ff7ab39b48c` SHA.

Artifact `13566968189` is 41,355 bytes with SHA-256
`8d22d3872885d93b7eaa20295a1c2ee385276ffc6c69ca0990adfdd4a26ddd8e`.
Its archive digest, all six raw reports per variant, warm replay and the
per-test disagreement counts were independently checked locally. Execution
and tests ran on GitHub, not on the Mac.

This finding does not change the observed zero disagreements in the
[12-commit convex-helpers window](convex-helpers-current-cloud-window.md).
It demonstrates why that corpus cannot establish universal correctness.
This regression is not a performance benchmark and supplies no savings
claim.
