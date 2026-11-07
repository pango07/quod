# Fresh-job managed matrix SDK replay

Verified research proof; production remains disabled. In [gate 42408971680](https://github.com/pango07/quod/actions/runs/42408971680), a fresh GitHub job restored one authenticated cache collection and replayed **108 of 120 actual project/file/pool instances**. That preserved **3,947 of 4,131 original test states** (3,946 passed and one skipped). The other 12 instances executed natively and passed their 184 tests. All 4,131 project/file/pool/full-name/state outcomes, including duplicate names and project aliases, matched both fresh controls and the previously verified native baseline. No original assertion or selected specification was removed.

The 108 replayed instances comprise all 81 Zod project instances and 27 root instances. The consumer started zero managed **test** workers and one private boundary preflight worker. **`fullJobHit` remains false** because 12 native instances still run.

| Fresh job | Test execution | Dispatcher seconds | Complete job seconds |
| --- | --- | ---: | ---: |
| Seed |108 managed instances plus 120 native conformance instances |224.889 |264 |
| Uncached control |108 managed instances plus 120 native conformance instances |282.088 |328 |
| Consumer |108 authenticated replays plus 12 native instances |97.102 |139 |

Complete-job elapsed time comes from the GitHub Jobs API's start/end timestamps and includes setup, installs, execution, artifact upload and cleanup. It excludes dependency and queue wait. These are diagnostic timings for one fixed source profile. The seed and uncached control perform native conformance in addition to managed execution; they are not timing controls for ordinary execution alone. This experiment provides no controlled speedup estimate and does not update the [original twelve-commit benchmark](managed-vitest-frozen-cold-window.md) or establish its 50% objective.

The consumer still freshly compiled all 108 candidates before lookup: **85.777s compiling**, **5.347s native execution**, and about 5.978s for the other dispatcher work. Compilation accounts for 88.3% of its 97.102s. Removing that cost requires a separately validated proof of current source.

The seed made one collection restore (miss) and one save, acknowledged with positive cache ID **10172279436**. The consumer made one restore and zero saves; the uncached control made zero collection SDK calls. The actual SDK-returned consumer service key exactly matched the seed's acknowledged immutable key. Its suffix authenticated the **17,130,107-byte** collection SHA256 `fa192c77a05a57993f9bf5b0e98b661a2d2b7c1a3d552faf9375e2f0e59fee96`, which in turn authenticated all 108 receipts. Each receipt matched its current closed execution identity, native conformance and exact original worker response bytes. The consumer's native batch was exactly the selected 120 instances minus those 108 hits. Receipts crossed jobs through the cache service; retained artifact copies were audit evidence only.

The pinned supervisor removed its private cache session file immediately after loading it, before test execution, and asserted that cache authentication was absent outside awaited SDK operations. All three roles retain those lifecycle assertions. No session/auth file was retained in the verified artifacts. The prerequisite ran 10 real two-file cases and 89 checks, including malformed receipts, actual native failure, supported assertion failure, file-only hook failure, caught unsupported fetch, exact hit omission and nonempty line-filter rejection.

Runtime/workflow source: `d575e2221c0aac26305e0667928a0472e0b733af`; collection Action source: `02f65151206f3a12d6f17e3b3d1831b704f28a0b`. The [machine-readable report](managed-vitest-matrix-sdk.json) retains source hashes, exact artifact IDs/sizes/SHAs, SDK authority, counts, timings and proof limits. Its standalone [data-only auditor](../../scripts/bench-vitest/managed/matrix-sdk-audit.py) verifies every archive entry and recomputes the full archive→service key→collection→receipt→response/native-state chain. It never executes artifact code.

Run it from a checkout containing the pinned source Git object, using the JSON report's `auditInputTemplate` after downloading the listed archives and retained API metadata:

```sh
python3 scripts/bench-vitest/managed/matrix-sdk-audit.py evidence-index.json
```
