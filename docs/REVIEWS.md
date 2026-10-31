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
| exp/overhead3 fixes (5f91e2d), verification pass | `52c8cea..5f91e2d` | all 7 findings fixed or nearly; no new false-hit path; merge after: ledger v2 coordination with exp/trust, put_result/record_run atomicity, stricter nothing-recorded test, commit --ignored ring output (all done in cfc12a6, 2b0b3e5) | `d83dd51` | pending |
| exp/overhead3: io_uring FUSE observer (tip 1aae4e4) | `976744f..1aae4e4` | DO NOT MERGE as is: claimed differential test absent (critical); silent ring registration failure and racy setup can hang a step; dead queue thread hangs instead of failing; unverified 7.47 open_out layout is a false-hit path; transport not stored with results; ring acceptance ~40% slower end to end, unreported | - | pending |
| exp/split (review fixes, tip 3c06de1) | `6372c70..3c06de1` | merge after fixes: 2 new major (repeat absent lookups never reach FUSE through overlayfs; replays ignore same-step writes), prior metadata/chdir fixes partial, not rebased | - | pending |

| exp/trust fixes (25c049f), verification | `2a5fd10..25c049f` | merge after fixes: all prior findings fixed (spec drift partial), no new false-hit path; required: rebase + merge-gate with fault-injection targets, unversioned fingerprint quarantines every key when rules change | - | pending |
| exp/trust: trust layer, timeouts, panics, CLI (tip 16a6c83) | `merge-base(main)..16a6c83` | merge after fixes: HOME replayed but uncompared (go install into $HOME/go/bin); two identical flaky failures prove a failure; Stillpass conditions counted as step divergences, bar never decays; `needed` overflow. Absent-entry equivalence verified sound | - | pending |

| exp/rootfs fixes, rebased (c95d157) | `2a5fd10..c95d157` | verification: merge after fixes. C1, M1, M2, m2-m5 fixed; C2 partial (host class read from Stillpass's /proc not the step's; pids.max, binfmt_misc, seccomp filter content unkeyed; RLIMIT_NPROC re-keys exact RAM); m1 partial | - | pending |
| exp/rootfs: pinned OCI rootfs, whole-root overlay, key v3 (tip a6f7632) | `ad16706..a6f7632` | merge after fixes, 2 critical: inherited umask/rlimits/signals neither keyed nor fixed; host properties constant per host (RAM, online CPUs, kernel build, sysctls, LSM, seccomp, fs type, bwrap) "left to audit" though audits can never see them; rootfs mtimes and readdir order leak host filesystem; portability claim overstated (one VM) | - | pending |

| exp/runner: actions/runner container hooks (tip a2bc909) | `merge-base(main)..a2bc909` | DO NOT MERGE: uid not in key (false hit); offline sandbox results recorded as the step's (network failures, `\|\| true` side effects, missing .git, overridden workflow vars); raw pre-mask output of token-bearing steps stored; docker.sock bound into tainted steps; shared writable warm GOCACHE across forks; ref resolution can serve the wrong commit; gate logs not committed | - | pending |

| exp/polyglot fixes (fcda366): content-derived mtimes, no_outputs_under enforced, strict normalizer, corrected docs | `1dbe6a2..fcda366` | fixes reported; must port content_mtime to the io_uring attr encoder on rebase; verification review after rebase | - | pending |
| exp/polyglot: toolchain profiles, Node (luxon) and Python (starlette) | `merge-base(main)..1e92709` | merge after fixes, no false hit (stale-pyc refuted): mtime-keyed caches trusted under constant mtime record wrong results; sealed snapshots expose host readdir order (latent cross-host, same class as rootfs M1); unsupported "measured payoff" claims | - | pending |

| fix/uring-fallback: refused-registration teardown (66be2b5) | `2a5fd10..66be2b5` | merge after fixes: "only our mount" unimplemented (umount2 acts on the top mount; compare device numbers); fusectl verification claimed but unmounted in the dev container; teardown unbounded | - | pending |

Builders on Opus since the Fable limit (their branches will be listed here when
reviewed): exp/runner, exp/rootfs, exp/trust, exp/polyglot.
