# Bounded configuration names proof

**Historical measurements, superseded engine.** The [names-proof window](config-names-proof-window.md) measured 968/1766 runner-seconds and a separate best hit of floor + 3 s, but the later [metadata-only stale replay](stat-metadata-regression.md) invalidated its recording semantics. These figures do not establish the corrected engine's safety or savings. The [corrected window](https://github.com/pango07/ci-bench/actions/runs/42167641326) is being measured separately.

Experimental, disabled unless `QUOD_VITEST_NAMES_PROOF=1`. This flag is
explicitly keyed, and the embedded proof/parser are included in `code_hash`.
This does not narrow npm script unit keys or change application source.

The candidate is a private top-level ESM JSON parse followed immediately
by a declaration whose first call is native
`Object.keys({...pkg.dependencies, ...pkg.devDependencies})`. A subsequent
filter/sort chain is allowed. Every reference to the private binding must
be one of those spreads. No intervening effects, awaits, revivers, dynamic
evaluation, exports, aliases, shadowed Object, or other binding uses qualify.

Runtime evidence uses native call sites and V8 `Debugger.getScriptSource`,
matched by script ID, never a filename or source map. A missing/async
inspector reply, non-module source or unsupported source shape falls back.
The inspector session is in-process and opens no listening port.

Before granting the proof, language intrinsics must match their captured
descriptors, and the package's dependency objects must still contain only
Quod's getters over primitive JSON values. The callee is checked immediately
before the parse returns, with no application code between that return and
the certified `Object.keys` lookup. This adjacency matters: checking a
callee only during its argument evaluation would permit a temporary getter
to substitute a value-reading function and then restore the native function.
Each omitted value read must occur at that exact executed call site.
Dependency names, order, shape and every ordinary observed value remain
inputs under the existing `pjf` hashing scheme.

This is a bounded proof, not a general config-output hash or a replacement
test runner. It does not prove arbitrary plugins, closures, transforms,
setup effects or changes to installed packages equivalent. The remote
configuration diagnostic still explicitly marks output equality as
`replayEligible: false`; real replay requires the normal recorder rules.

The lightweight regression suite covers the accepted runtime expression,
disabled behavior, version-dependent branches, escaping bindings, custom
builtins, transient getters, effects before the expression and disk source
that differs from V8's executed source. The GitHub audit gate runs these
alongside the existing recorder and Rust regressions. The config gate also
checks the actual Vite-loaded convex-helpers configuration, using a fixed
installed tree. Neither gate substitutes for a changed-commit benchmark.

Parser provenance: Acorn 8.15.0 from the official npm registry tarball,
SHA-512 base64
`NZyJarBfL7nWwIq+FDL6Zp/yHEhePMNnnJ0y3qfieCrmNvYct8uvtiV41UvlSe6apAfk0fY1FbWx+NwfmpvtTg==`.
The unmodified `dist/acorn.mjs` and MIT license are embedded together.

Validation at `b9e34fc`: [audit and Linux binary gate](https://github.com/pango07/quod/actions/runs/42098123727)
passed Rust tests, existing recorder regressions, 26 focused JS regressions
and clippy, with no formatting correction. [Actual config gate](https://github.com/pango07/quod/actions/runs/42098119329)
proved one private parse and excluded 48 copied values in each of the three
manifest declaration cases. Each retained only `devDependencies` names for
that parsed read, selected the same 9 projects/120 files and preserved the
controlled config/context hash. Config plus selection took 205–212 ms.
The gate caught and drove a missing-field refinement: the real manifest has
no `dependencies` field. Absence now qualifies only when the ordinary object
prototype cannot supply an inherited property/getter.

The packaged replay gate and a paired changed-commit diagnostic have now
passed; [the measured results](config-names-proof-results.md) preserve every
job, its runner image, the cold/confirmation costs and the narrower changed-
commit comparison. Proof used 78/330 runner-seconds on the two dependency
upgrades and 379/651 including cold recording and confirmation, with zero
wrong test-state comparisons. This does not replace the previous 12-commit
result: 1173/1905 runner-seconds for the best policy, still above 0.5x.
