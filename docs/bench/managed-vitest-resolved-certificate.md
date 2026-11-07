# Resolved compile certificate diagnostic

The [766ff38 correctness gate](https://github.com/pango07/quod/actions/runs/42415327673)
passed, and the independent data reader recomputed every retained bundle pair
and native test-state comparison. This profile remains unwired: it grants no
receipt admission and makes no SDK, Action, whole-job or original-window speedup
claim. Runtime source is pinned to `766ff388f1dcc3305d43a14e99655404dc01b596`.

| Observed result | Value |
| --- | ---: |
| Original compiler controls | 55 passing |
| Targeted certificate controls | 213 passing |
| Initial exact bundle/full-identity pairs | 12 |
| Actual input mutations | 18 |
| Warm tool/API/environment variants | 7 |
| Fresh same-version tool mutations | 5 |
| Final fresh certified bundle/full-identity pairs | 108 exact matches |
| Native project/file/pool instances | 120 |
| Native test states | 4,130 pass; 1 skip |
| Managed instances and states | 108; 3,946 pass; 1 skip |
| Ordinary instances and states | 12; 184 pass |

The final 108 compiles used fresh native preparation and matched the matrix's
actual raw worker bytes, full identities, options and preparation identities.
Native rows matched the independently byte-anchored `ff279ea` baseline as a
multiplicity-preserving project/file/pool/full-name/state multiset.

| Same-runner diagnostic measurement | Value |
| --- | ---: |
| Shared validation of 108 certificates | 11.846891 seconds |
| Fresh preparation, 108 builds, captures and retained writes | 272.328984 seconds |
| Sum of those builds' internal compile timers | 109.484663 seconds |
| Raw certificate bytes | 35,084,701 |
| Embedded compile-identity bytes | 29,187,279 |
| Largest certificate | 440,937 bytes |
| Largest compile identity | 378,877 bytes |
| Complete correctness-gate job | 915 seconds |

Validation timing excludes the future cache/receipt path, mandatory native
preparation and preflight, ordinary nonhits, install/build/release work and
complete job overhead. The 915-second gate intentionally performs two full
compile sets, native conformance, mutations and evidence transfer. The
162.844321-second difference between capture wall time and internal compile
timers includes epoch checks, preparation, captures and writes; it does not
isolate hashing cost. Raw certificates alone exceed the existing 32 MiB collection
limit. Any compressed envelope transport needs its own actual size and authority
gate.

The finite profile pins the reviewed compiler SHA and complete four-package
inventory: esbuild 0.28.2, its Linux x64 binary, the exact 45-byte
`@typescript/typescript6` 6.0.2 wrapper and its delegated TypeScript 6.0.3 package.
The inventory has 183 paths and 47,351,803 file bytes, canonical SHA-256
`7b7ae881ef3186e4b564c26ffa6e8d40b3d3875a15e41b10d93a6bb24cb3a354`.
Fresh unknown same-version tools and compiler sources decline before loading.
Actual entry routes, loaded Module/export/API identities, source bytes,
original and emitted resolutions, globs, root geometry and plain Node flags
remain checked. Initial Node/verifier integrity and the reviewed finite
configuration/parent host are trusted; sampled reads do not claim adversarial
filesystem atomicity or arbitrary-host attestation. See the
[input argument and limits](../../scripts/bench-vitest/managed/resolved-compile-certificate.md).

Failed investigations remain part of the record:

| Gate | Observed outcome |
| --- | --- |
| [42412794953](https://github.com/pango07/quod/actions/runs/42412794953) | Failed before pairs: wrapper manifest 6.0.2 had been confused with delegated API 6.0.3; delegated bytes were missing from the tool closure. |
| [42413896155](https://github.com/pango07/quod/actions/runs/42413896155) | Failed before pairs on an epoch mismatch. No actual field tuple was retained; preparation's explicit environment initialization explained the required ordering correction. |
| [42414441305](https://github.com/pango07/quod/actions/runs/42414441305) | 195 controls and 12 pairs passed; a fixture rejected the multiline text of a correct CLI-flag denial. Both 108 stages were skipped. |

The older conservative `5c6aa48` diagnostic passed 92 controls and observed its
pairs, but hashed only the wrapper. Its namespace/JSON roster did not include
the delegated parser JavaScript, so it establishes no future tool-closure
authority. Historical code and artifacts remain unchanged. The prior 108 SDK
proof compiled and hashed every full actual bundle before lookup and did not
use a compile-input certificate.

## Reproduce the data audit

Use a checkout containing source commit `766ff388f1dcc3305d43a14e99655404dc01b596`.
Download the two original ZIPs from these artifact links; retain the ZIPs rather
than executing any contained code:

| Artifact | ZIP bytes | SHA-256 |
| --- | ---: | --- |
| [Resolved gate 13591328320](https://github.com/pango07/quod/actions/runs/42415327673/artifacts/13591328320) | 89,838,252 | `0b84d4d439353b4d62c19260f612f4aedafb9ca66df66ab1e2a7d81a3542bfd3` |
| [Native baseline 13587034973](https://github.com/pango07/quod/actions/runs/42406036816/artifacts/13587034973) | 36,186,785 | `f8e20f3a68cc3fbec7ce01a5ea90e8b50a1d558cbf08146a7fb049e242adc282` |

```sh
gh api repos/pango07/quod/actions/runs/42415327673 > run.json
gh api 'repos/pango07/quod/actions/runs/42415327673/jobs?per_page=100' > jobs.json
gh api 'repos/pango07/quod/actions/runs/42415327673/artifacts?per_page=100' > artifacts.json
python3 scripts/bench-vitest/managed/resolved-certificate-audit.py \
  --archive resolved.zip --extracted resolved-data \
  --run run.json --jobs jobs.json --artifacts artifacts.json \
  --baseline-archive native-baseline.zip --output audit.json
```

The standard-library reader has no private helper dependency. It pins the exact
run/source/job/artifact/ZIP bytes, rejects duplicate JSON fields and ZIP path,
case/normalization, type and size hazards, then creates `resolved-data` only if
absent. Existing extraction contents must match the complete ZIP file/directory
sets and every file byte. Bounds are 128 MiB compressed, 1 GiB expanded, 20,000
paths and 16 MiB per file. The observed ZIP has 2,049 files and 562,638,286 expanded
bytes. The baseline member is independently pinned to 1,903,888 bytes and SHA-256
`1d3ceb0a7520632c23474769721d608875eb4b9cfed6c6d248f61b89c946ced1`.
Only immutable source blobs are read through `git show`; no artifact code is
imported or executed. Complete tool inventories are producer observations tied
to pinned primary API bytes, rather than a copy of every installed tool file.
