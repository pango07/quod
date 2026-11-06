# Native Stats guard: real-job package probe

The corrected package matches plain Vitest, but still does not recover test
reuse across fresh runners. This probe establishes no full-window savings.

Engine `4d993190050f71ec10a6c825328807b14c415b7a`, Action
`d2bb917792130dcaf3fafec066b481bec5c2f674`, and harness
`99d0ca5af335d82fee1b7bbd37a35d4b251a376a` ran convex-helpers commit
`5eaa7e5160736054d53f8b71e2c522835eb427e0` on branch
`bench/metadata-4d99319`. Names proof and lineage were enabled; Boolean
manifest refinement, timing replay, Cloud and task cache were disabled.
The install/build/test commands and report preload were unchanged.

| Job | Run | Runner-s | Rounded min | Vitest replayed | Wrong vs plain |
|---|---|---:|---:|---:|---:|
| Plain | [42150962645](https://github.com/pango07/ci-bench/actions/runs/42150962645) | 166 | 3 | — | — |
| Quod initial | [42150965835](https://github.com/pango07/ci-bench/actions/runs/42150965835) | 123 | 3 | 0 | 0 |
| Quod reuse attempt | [42151279597](https://github.com/pango07/ci-bench/actions/runs/42151279597) | 140 | 3 | 0 | 0 |

Every job used `ubuntu-26.04/20261101.186.1` and matched all 120 file/project
instances and 4,131 test states, fingerprint
`09ebb9e3a722220d3a21ddfe9f63992a8a074b0fcaa95764e54c37be90d469cd`.
Both Quod jobs made zero lineage promotions and ran zero lineage audits.
The separate packaged fixture ran one audit with zero disagreements.

The first Quod job had no dependency tree cached for this new branch. It
restored a main-branch ledger, but no old test records were eligible; all
120 files were recorded. The second ran only after the first completed and
saved its ledger. It restored dependencies in 3.6 seconds, skipped install,
and hashed 1,913 inputs. **47 input values changed**, including the shared
complete metadata of test files, forcing every file to run again. The first
reported changes now start with test-file metadata rather than the three
directory stat reads refined by the direct type proof.

The recorded suite took about 49 seconds in the initial job and 105 seconds
in the reuse attempt. The latter included a property test at 25.8 seconds
of a 30-second timeout; it remained ineligible for lineage promotion.
This variation prevents attributing the initial timing difference solely
to Quod. There is one plain job and two Quod jobs, so their totals are not a
window savings ratio. All three round to three minutes: neither Quod job
demonstrated a rounded-minute saving against this baseline. Actual charges
for these public jobs were $0.

There were no full hits, so best/median hit and a best hit expressed as
floor + N s are unavailable. Neither the 12-commit window nor c1–c11 was
measured here. The founder's performance bar remains unmet on the corrected
engine; the historical 45.2% runner-time / 41.7% rounded-minute reductions
cannot be presented as current results.

The next performance work remains a sound proof for asynchronous stat
consumers and Node's callback/realpath metadata observations. Unknown
consumers must keep complete metadata. See the
[guard and proof evidence](stat-metadata-regression.md) and
[machine-readable job evidence](metadata-native-guard-probe.json).
