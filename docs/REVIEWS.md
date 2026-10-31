# Review ledger

Every change reaches `main` only after an adversarial review. Fable is the
standard reviewer. While Fable was unavailable (usage limit, 2026-10-31) the
owner approved Opus reviews; every Opus-reviewed change is listed below with
**Fable re-review pending** and gets a Fable pass when Fable is available again.

To re-review: `git diff <base> <merge>` for the range in the table, with the
same brief as the original review (false-hit paths first, then unsound
arguments, then unsupported claims).

## Reviewed by Fable

| Change | Range | Verdict | Fixes |
| --- | --- | --- | --- |
| Checkpoint 1: T01-T05 (CAS, key, manifest, FUSE serve) | `86f4d02..0420bf4` | 3 major | `f69a08a` |
| Checkpoint 2: T06-T11 (overlay, sandbox, ledger, replay, CLI, acceptance) | `f69a08a..11ca255` | 1 critical, 4 major | `70f5263` |
| exp/misses: outputs moved into CAS, warm state | merged `af38959` | merge after fixes | `ef1e45a` |
| exp/overhead: FUSE passthrough, sealed module cache | merged `6372c70` | 1 critical, 1 major | `e241d7f`, `b796a0f` |
| exp/overhead2: shared snapshots, io_uring probe | merged `28964c8` | 1 critical, 2 major | `9465426`, `c095e2f` |
| exp/split (first version, pre-rebase) | `3eb2f63..493cfca` | 4 major | re-reviewed by Opus, below |

## Reviewed by Opus: Fable re-review pending

| Change | Range | Opus verdict | Merged as | Fable re-review |
| --- | --- | --- | --- | --- |
| exp/overhead3: io_uring FUSE observer (tip 1aae4e4) | `976744f..1aae4e4` | DO NOT MERGE as is: claimed differential test absent (critical); silent ring registration failure and racy setup can hang a step; dead queue thread hangs instead of failing; unverified 7.47 open_out layout is a false-hit path; transport not stored with results; ring acceptance ~40% slower end to end, unreported | - | pending |
| exp/split (review fixes, tip 3c06de1) | `6372c70..3c06de1` | merge after fixes: 2 new major (repeat absent lookups never reach FUSE through overlayfs; replays ignore same-step writes), prior metadata/chdir fixes partial, not rebased | - | pending |

Builders on Opus since the Fable limit (their branches will be listed here when
reviewed): exp/runner, exp/rootfs, exp/trust, exp/polyglot.
