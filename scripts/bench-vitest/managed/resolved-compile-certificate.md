# Resolved compile input certificate investigation

This separate profile is diagnostic and unused by the Action, dispatcher and
receipt ledger. The conservative `compile-input-certificate.mjs` is unchanged.
Native Vitest preparation always runs. A successful input validation issues an
opaque in-process proof of compile identity equivalence. It supplies neither a
worker bundle nor a response and cannot declare a command passing. A miss always
requires a fresh full managed compile.

The intended later sequence is fresh native selection/config preparation,
authenticated candidate lookup, fresh input verification, then independent
admission of the current full execution identity by the passing receipt ledger.
If receipt admission misses, the executor still requires an actual newly built
bundle. The native remainder remains mandatory.

## API and token boundary

`await createCertifiedManagedCompiler({project, adapterRoot?, sourceTreeCache?})`
is the builtin-only bootstrap. Call it before dynamically importing preparation,
planner or dispatcher modules, which can indirectly preload the ordinary compiler.
It records tool bytes before the first dynamic compiler import/TypeScript load,
rejects unknown prior cached tool modules, and retains that first loaded epoch.
Every build and capture rechecks the epoch. A/B relocation producers run in
separate plain Node children so they do not inherit the parent's project TS cache.

`captureResolvedCompileCertificate({project, adapterRoot?, built, options,
testFile, entry?})` accepts an actual closed compiled bundle and returns exact
canonical certificate `bytes` and `sha256`. Only actual build objects issued by
the certified factory's private WeakMap qualify. It binds their original exact
identity, closure and bundle digest; warm ordinary/deserialized/modified objects
decline. The ordinary compiler entry point otherwise remains available.

`validateResolvedCompileCertificates({project, adapterRoot?, requests})` accepts
the existing per-request `{certificateBytes, trustedCertificateSha256, options,
testFile, entry?}` shape. Each result is either `{hit:false, reason,
next:'full-compile'}` or a diagnostic identity plus an opaque `proof` object.
External SHA authority must come from an authenticated transport, not a field
inside the certificate. The diagnostic probes use explicitly constructed seed
authority and claim no SDK or receipt admission.

`readValidatedCompileIdentity(proof)` is the only interface a later trusted
ledger factory may use. A private module `WeakMap` accepts only tokens issued by
an actual completed validation in this module instance. A forged, copied or
serialized object fails. The reader returns a fresh copy of `{identity,
identitySha256, bundleSha256, configurationSha256}`. Mutating that copy or the
diagnostic result cannot modify the private proof. A later ledger factory must
still derive current preparation, runtime policy, native fingerprint, host and
expected instance independently. No ledger change is made here.

## Finite compiler input argument

The implementation is restricted to Linux x64, Node 24.18.0, esbuild 0.28.2,
the installed `@typescript/typescript6` 6.0.2 wrapper under `typescript`, and its
delegated `@typescript/old` implementation with package name `typescript` 6.0.3.
It hashes all four complete installed packages (esbuild, platform binary,
TypeScript wrapper and delegated implementation), Node executable and current
compiler/verifier source. The wrapper's exact 45 bytes are pinned by SHA:
`module.exports = require("@typescript/old");` followed by newline.
Actual ESM and CJS esbuild API entries must both be the hashed package's
`lib/main.js`; both TypeScript wrapper and delegated require entries must be
their hashed package's `lib/typescript.js`, and the
platform binary must be the hashed `bin/esbuild`. Manifest identity alone cannot
substitute for actual entry selection through package self-references or
conditional exports. The adapter package manifest is also retained. The source
argument for self-references is Node's [pinned package documentation](https://github.com/nodejs/node/blob/v24.18.0/doc/api/packages.md#self-referencing-a-package-using-its-name).

The bootstrap observes its own file and Node executable at module initialization.
The compiler records its module-evaluation source SHA/API route so cached code
cannot acquire a later file SHA merely through a new constructor. Loaded cache
Module identities, actual `.exports` identities, used esbuild build/version and
the finite used TS functions/version/enums are checked before and after each
build/capture. This is a reviewed finite config and trusted parent-host contract;
it does not attest arbitrary project config, mutable host intrinsics, global
hooks or all loaded JavaScript state. Sequential on-disk tool changes must still
decline, and restoration is allowed only when exact original bytes and recorded
loaded API identities recover. These controls do not claim general hostile
Node-host protection.

The first resolved gate [42412794953](https://github.com/pango07/quod/actions/runs/42412794953)
at `7cebf45b65fb9a4fca2f66641273b6931aaaccbd` failed at the initial tool-version
check before any candidate validation or workers. Its 55 compiler checks passed;
the matrix and 108-pair steps were skipped. Artifact 13590067499 remains a failed
gate's evidence: 2,208,194 bytes, SHA-256
`3c241c0fc838691f5072ea9c03c3ef4fd87aedc14d02ec99c81f0a69cd839d0d`.
The loaded API version 6.0.3 had incorrectly been treated as the wrapper manifest
version. Fixing the version alone would have left the delegated tool bytes out.

The exact installed primary bytes were streamed as data from the already
SHA-verified [native image artifact](https://github.com/pango07/quod/actions/runs/42409495211/artifacts/13589761021),
without installing or executing anything:

| Installed file | Bytes | SHA-256 |
| --- | ---: | --- |
| Wrapper package manifest | 841 | `d9b8fb53c67c947fece83bcb73fa8512227ea6a12b110e096fdab8ecdc02b655` |
| Wrapper `lib/typescript.js` | 45 | `d3f3cd2b04b7f466f4484df921b744223f7bd1f3e353ec9110bdf52695b983d5` |
| Delegated package manifest | 3,527 | `9332e97c30d3e53ed54910b89207ed657fb444066484df6e5b6965bf130865e9` |
| Delegated `lib/typescript.js` | 9,144,216 | `569177652966bd528c319171c7dd22860dbf72bde116cbc4f644f1d02bb12e39` |

The corrected epoch resolves the delegated API from the actual wrapper location,
hashes its full package before load, guards both cache Module/export identities,
and checks that the wrapper exposes exactly the delegated export object. Any
newly loaded CJS dependency outside the four measured tool roots declines.
Warm delegated-file and delegated-exports changes must reject and exact original
restoration must recover.

The conservative `5c6aa48` diagnostic passed 92 controls, observed twelve exact
bundle pairs and 108 native-conformant instances, but its toolIdentity also
hashed only the wrapper. Its namespace retained JSON/path facts rather than
the delegated TS JavaScript bytes. That historical result remains an unused
diagnostic and does not establish future tool closure under delegated API changes.
The prior 108-instance SDK gate still compiled and hashed every actual complete
bundle before receipt lookup; it did not rely on a compile-input certificate.

Nonempty Node CLI flags and `NODE_OPTIONS`, `NODE_PATH`,
`NODE_PRESERVE_SYMLINKS`, `NODE_COMPILE_CACHE` or `ESBUILD_BINARY_PATH` decline.
`NODE_ENV` and `ESBUILD_WORKER_THREADS` remain keyed. These are on-disk hashes,
not loaded-code attestation.

The reviewed primary source is esbuild tag 0.28.2, resolving to commit
`609683d892977362a0f99026cb74b96263d728a9`:

| Input channel | Pinned implementation | This compiler's restriction |
| --- | --- | --- |
| Default resolution metadata | [bundler.go:1147](https://github.com/evanw/esbuild/blob/609683d892977362a0f99026cb74b96263d728a9/internal/bundler/bundler.go#L1147) returns a successful plugin result before default resolution, copying only path, external state, plugin data and an explicitly supplied side-effects flag. | The plugin returns only the current real path. It does not return side-effects metadata or plugin data for physical modules. |
| Transform config and package type | [bundler.go:1574](https://github.com/evanw/esbuild/blob/609683d892977362a0f99026cb74b96263d728a9/internal/bundler/bundler.go#L1574) applies TSConfig/JSX/strictness and module-type fields from that result; [resolver.go:893](https://github.com/evanw/esbuild/blob/609683d892977362a0f99026cb74b96263d728a9/internal/resolver/resolver.go#L893) shows where ordinary default resolution otherwise obtains them. | Those fields are absent after this path-only plugin. Extension-specific `.mjs/.mts/.cjs/.cts` behavior remains a function of the exact real path. Nearest tsconfig may still affect resolution, which is freshly rerun. |
| Physical source bytes | [bundler.go:1235](https://github.com/evanw/esbuild/blob/609683d892977362a0f99026cb74b96263d728a9/internal/bundler/bundler.go#L1235) uses successful plugin-supplied contents and explicit loader. [bundler.go:1263](https://github.com/evanw/esbuild/blob/609683d892977362a0f99026cb74b96263d728a9/internal/bundler/bundler.go#L1263) falls back to filesystem loading only without successful plugin contents. | All JavaScript/TypeScript/JSON inputs must have an exact onLoad source row. A producer compile now asserts a multiset bijection with every physical metafile input, rejecting fallback loaders or missing records. |
| Other input namespaces | Generated define and the single compiler ALS facade are the only observed special inputs in this finite build. | Only `<define:__QUOD_CONFIG__>` and `quod-managed-als:public-async-local-storage` are admitted. Defines are tied to complete options SHA and compiler bytes; ALS is tied to its exact emitted route and fixed compiler contents. Unknown virtual inputs fail compilation. |
| Extra source-map reads | [bundler.go:641](https://github.com/evanw/esbuild/blob/609683d892977362a0f99026cb74b96263d728a9/internal/bundler/bundler.go#L641) reads original source maps only when source maps are enabled. | This compiler does not enable source maps, watch, inject, additional plugins or CSS output. Exactly one closed JavaScript output is required. |
| Public resolver callback | [api_impl.go:2109](https://github.com/evanw/esbuild/blob/609683d892977362a0f99026cb74b96263d728a9/pkg/api/api_impl.go#L2109) resolves using current build options and returns public result fields. | Ordinary compilation and validation use one shared resolver helper. It returns only a real path, rejects external targets/suffixes, and retains the exact alias/kind/importer policy. |

These restrictions justify testing an input relation that omits a global
namespace/all-JSON snapshot. They do not justify omitting current resolution.
Every original import, including erased type-only imports, and every emitted
or generated edge remains recorded. After deduplication, every ordinary query
runs in one new plain Node process and one new resolver-only esbuild build.
The child has a 30-second watchdog and checks tool bytes before and after.
This avoids reusing the parent's Node CJS resolution cache or esbuild service.
Special private routes are verified against their exact fixed source, target,
kind and policy. The child loads no project modules or tests.

Current complete loaded-source bytes are read once per distinct real module
across candidate instances, compared with original and raw byte SHA values,
then rechecked after resolution. Glob sets are also sampled again before any
proof is issued. Current root absolute real paths remain
keyed because bundle comments and generated identifiers can depend on physical
geometry. Glob name sets, real targets and actual admitted self-entry matches
are freshly enumerated. A changed source, alias/context, current resolver target,
glob set, tool, option or geometry produces a miss.

Adding a formerly absent higher-priority extension, package export, tsconfig
path mapping or symlink target cannot be justified by an old negative fact:
the fresh resolver must still choose exactly the old real target for every
query. Irrelevant paths/JSON may be added without invalidation only when current
queries, full loaded bytes and glob sets remain equal. A metadata-only
`package.type`, `sideEffects` or transform-only tsconfig change is expected to
validate only under the pinned path-only metadata behavior above. All such
positive controls compare actual full fresh identities and exact bundle bytes.

This is sampled validation in a nonadversarial, quiescent workspace. It does not
claim an atomic snapshot against active mutation. Per-read inode/device/size/
mtime/ctime samples, fresh tool observations and repeated source byte reads
detect observed changes, but there is no adversarial TOCTOU guarantee.

## Bounds, proof and payload gate

The explicit bounds are 100,000 source/tool/glob paths and resolver queries,
128 MiB installed tool bytes, 256 MiB shared loaded-source bytes, 16 MiB ordinary
file/certificate/resolver request/response, 128 MiB Node executable/candidate set,
and 512 candidates. The resolver's private temporary directory is mode 0700,
request mode 0600 and removed in `finally`. A child failure, timeout, malformed
response, unknown schema, missing external SHA or exceeded bound is a miss.

The remote job retains the existing 55 compiler checks, twelve actual paired
fresh bundle/identity comparisons, actual equal-metadata source mutations,
erased-original resolution changes/private aliasing, extension priority/removal,
exports, tsconfig paths, globs, symlinks, actual aliases/context, metadata-only
accepted inputs, forged token controls, actual Node CLI flags and adapter A/B
relocation. Mutation source snapshots, complete fresh bundles/build metadata,
certificates and decisions remain evidence. The tsconfig control also compiles
in a separate plain Node process; the two prior failed conservative investigations
remain documented in `compile-input-certificate.md`.

Additional controls warm the producer, mutate the same compiler/esbuild/TS API
file (including the delegated TS API) while keeping package versions unchanged, and require capture/build/new
factory rejection. A fresh plain child must compile the actual changed explicit
target policy, while ordinary warm outputs cannot acquire certified provenance.
Exact restored bytes recover the original epoch. Replacing wrapper or delegated TS `.exports` must
also decline. Fresh-child package self-reference fixtures keep the old manifest
target while routing esbuild ESM/CJS or TypeScript require through another API;
they must miss, with all files restored in `finally`.

The same current compiler then executes the complete native-conformance matrix:
108 passing managed instances and all 120 original native instances. A final
step bootstraps a fresh certified process, runs fresh native preparation, and
derives/compares all 108 current options and preparation identities. It freshly
compiles each current source, retains the actual new bundle/full build metadata,
and compares exact bytes/full identities with the current matrix's workers.
It captures and validates all 108 certificate identities, retains their exact
canonical bytes and reports total/max identity/certificate sizes and full cost.
Retained workers are read only as data; no artifact code is imported or executed.
Every retained evidence file is capped at 16 MiB, with 20,000 paths and a 1 GiB
expanded aggregate cap across the probe, matrix, compiler checks and paired 108
output directories, including a 16 MiB reserve for remaining small job evidence.
This intentionally retains both complete bundle sets; installed adapter copies
are deleted and never uploaded. Downloaders must use the reviewed 1 GiB expanded
profile instead of silently weakening or dropping paired bundle evidence.

Current wire certificates repeat tools and compile identity per instance. This
is not a production transport design. Actual twelve paired identities grew
from 1,055,558 to 2,802,158 bytes after original/emitted resolution facts were
recorded. A future authenticated envelope should store the exact passing receipt
once and reconstruct the certificate's `compile` field from that authenticated
receipt, verifying its saved exact certificate SHA before current validation.
Any compression must preserve exact bytes with separate raw size/SHA and bounded
decompression. The existing 32 MiB collection and 2 MiB entry bounds remain;
actual full-108 payload size must pass before transport is enabled. No SDK,
Action, envelope or receipt schema is changed by this diagnostic.
