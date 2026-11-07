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
of the disjoint project and adapter roots. All regular JSON bytes are hashed,
including package metadata, lockfiles, tsconfig files and arbitrarily named
in-root JSON extends targets. Applicable config extends chains may not escape
the roots. Above both roots, the exact existing package/config JSON roster and
bytes are retained and external ancestor node_modules are declined. `.git` is
the only excluded subtree: certificate source/resolution targets in it decline,
and every actual glob is freshly enumerated. Unknown/special/external objects,
unreviewed compiler preloads/tool overrides and exceeded bounds produce misses.

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
