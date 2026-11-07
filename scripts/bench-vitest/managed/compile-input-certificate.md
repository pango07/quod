# Diagnostic compile input certificates

This is unused by the dispatcher, Action, receipt ledger and collection reader.
Native Vitest preparation always executes. A successful validation returns only
a prior compile identity candidate, never a response, bundle or passing command.
A miss requires the ordinary full managed compile. A later authenticated passing
receipt lookup must independently admit the complete current execution identity.

The compiler now records every original import resolution before erasure, as
well as emitted and generated resolutions. The ordinary compiler and the fresh
resolver-only esbuild build use one shared resolver implementation. Validation
deduplicates all candidate ordinary edges into one new resolver-only esbuild
build; it loads no imported modules and performs no bundling of tests. The
imported esbuild API may reuse its existing operating-system service process.

Certificates retain the full namespace path/kind/permission and symlink roster
of the disjoint project and adapter roots. Their actual absolute real paths are
also retained: equal canonical module names do not imply equal esbuild comments
or generated identifiers after physical adapter relocation. All regular JSON bytes are hashed,
including package metadata, lockfiles, tsconfig files and arbitrarily named
in-root JSON extends targets. Applicable config extends chains may not escape
the roots. Above both roots, the exact existing package/config JSON roster and
bytes are retained and external ancestor node_modules are declined. `.git` is
the only excluded subtree: certificate source/resolution targets in it decline,
and every actual glob is freshly enumerated. Unknown/special/external objects,
unreviewed compiler preloads/tool overrides, nonempty Node CLI flags and exceeded
bounds produce misses.

Every original loaded source is freshly read in full, checked against its
original compiler SHA, and given an additional raw-byte SHA. Validation shares
those reads across instances and rechecks the bytes after resolution. It also
hashes complete installed TypeScript/esbuild/platform-binary packages, the Node
executable and current compiler/verifier source. Current options, actual entry,
test selection, alias/context and every glob name/real target must match.

Namespace snapshots and tool hashes run before and after resolution; each source
read compares its sampled inode/device/size/mtime/ctime. Tool hashes describe
on-disk bytes, not attestation of already loaded JavaScript or native code.
These checks detect observed changes. They do not provide an atomic filesystem
snapshot against an adversarial or actively mutating workspace. The diagnostic
requires a nonadversarial workspace; no such concurrency guarantee is claimed.

The explicit limits are 100,000 namespace paths, 128 MiB JSON metadata, 128 MiB
compiler package bytes, 256 MiB distinct source bytes, 16 MiB per ordinary input
or certificate, 128 MiB per Node executable or candidate certificate set, and
512 certificate candidates. The source byte limit also applies to the shared
retained source map across all candidates.
Namespace, metadata and tool observations are deliberately conservative:
irrelevant path additions and unused package JSON changes can cause misses.
An invalid certificate, unknown schema, omitted original edges, wrong external
SHA or any fresh verification failure returns `next: "full-compile"`.

The diagnostic wire format repeats common namespace/tool evidence in each
certificate. It is not ready for collection transport: a future separately
reviewed pack would share these observations once. In particular, the 108
retained passing c11 workers total about 139 MiB; there is no proposal to restore
all those bundles. The intended future sequence is validate a candidate compile
identity, ask the existing authenticated ledger for its receipt, and compile a
real fresh bundle only on a receipt miss. No executor may receive an identity
candidate in place of a real bundle.

The remote probe compares twelve actual project/file instances against complete
fresh compile identities and exact bundle bytes. It then checks real equal-size
and nanosecond-mtime source mutations, erased type-only resolution shadowing and
private aliasing, extension priority, removed files, package exports, tsconfig,
project alias/context, globs and symlinks. Constructed malformed certificates
also decline. These checks grant no receipt authority and make no performance
claim beyond retained same-runner diagnostic timings.

Every mutation retains the tiny actual fixture bytes and symlink target
inventory, full resulting bundle/build metadata or rejection, and certificate
decision before assertions. The tsconfig control additionally invokes a
separate plain Node process with a 30-second watchdog, 16 MiB per output file and
1 MiB per captured log. It requires a conservative metadata miss and exact equal
full identities and bundle bytes from both fresh compilers. This checks the
observed compiler behavior; it is not a TypeScript semantic-change control.

The failed investigations [42408563595](https://github.com/pango07/quod/actions/runs/42408563595)
and [42409190659](https://github.com/pango07/quod/actions/runs/42409190659) remain
failures. The second retained actual `useDefineForClassFields: true` and `false`
inputs, equal source bytes, and seed/same-process/separate-plain-Node bundles:
all 884,283 bytes, SHA-256
`2796f604f39b3b97a1416b9394ce0f3a9b1898a388cbe329ae0c0a1d7e9a275e`.
All three complete compile identities also matched. This disproved the old
fixture's expectation that that metadata mutation changes this compiler.

That result agrees with the pinned esbuild 0.28.2 implementation at commit
`609683d892977362a0f99026cb74b96263d728a9`:
[successful plugin resolution](https://github.com/evanw/esbuild/blob/609683d892977362a0f99026cb74b96263d728a9/internal/bundler/bundler.go#L1147)
returns its path without copying the default resolver's transform config or
module-type metadata; [transform options](https://github.com/evanw/esbuild/blob/609683d892977362a0f99026cb74b96263d728a9/internal/bundler/bundler.go#L1574)
only apply when those fields are present. Our ordinary resolver returns only
the real path, and onLoad supplies an explicit loader. This observation does
not remove any conservative metadata check in the validator.

A separate relocation control copies the complete trusted installed adapter
into two adjacent sibling directories, preserving every byte, mode and symlink
target. It loads the compiler and verifier from each copy, verifies equal
canonical namespaces, ancestor config rosters and tools, and compares both
fresh bundles. The A certificate must validate in A and miss in B because the
physical root geometry changed. Copies are bounded to 20,000 paths, 512 MiB
total and 256 MiB per regular file and removed in `finally`. The result, full
bounded data inventories, A/B certificates (including namespace and tools),
bundles and build metadata are retained for independent byte comparisons.
