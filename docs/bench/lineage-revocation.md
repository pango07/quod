# E1 end-to-end batch revocation

[Gate 42366964096](https://github.com/pango07/quod/actions/runs/42366964096)
passed at `31efac415300c41de6baef399351fa0b307c1511`. It runs the actual
`c12a7fe` binary bundled in Action
`beb12a8c9c32500ad2d19a87b10ee7cc7a95c197`, with no recorder source override.
The runner was `ubuntu26/20261101.186.1`, Node 24.18.0.

The earlier packaged regression established promotion and an agreeing audit;
the Rust regression checked batch revocation in isolation. This fixture adds
the complete path from a plain audit disagreement to persisted batch
revocation and the next execution decision.

| Phase | Executed files | Replayed files | Result |
| --- | ---: | ---: | --- |
| Cold recording | 3 | 0 | No trusted predecessor |
| Plain witness | 3 | 0 | Real plain trust established |
| Warm | 0 | 3 | Actual replay established |
| Changed input, same test states | 2 | 1 | Two records promoted under one stamp |
| Agreeing audit | 1 | 2 | One peer becomes plain-trusted; provenance retained |
| Injected disagreement | 1 | 2 | Audited file quarantined; both batch records reset to `n=1` |
| Next run | 2 | 1 | Both revoked files execute; independent control still replays |

The fault deliberately changes only the remaining pending record's stored
test-signature hash. It does not fabricate a promotion, alter input keys,
change test code/results, or model a naturally occurring recorder defect.
The actual plain run disagrees with that signature. Revocation also reaches
the peer whose earlier agreeing audit had cleared its pending flag, while
preserving the unrelated control's witness state.

Nine reports, including ordinary plain controls before and after, each
contain the same three passing test states. Independent artifact analysis
verified every report hash and state, the single-field injection, both
persisted revocations, quarantine, unchanged control trust and execution
after revocation. Fast rendering, narrowing and opportunistic audits are
explicitly disabled to isolate the E1 selection/revocation path.

The [evidence manifest](lineage-revocation.json) retains report and ledger
hashes. Artifact `13572520382` is 119,952 bytes, SHA-256
`851b9599eba41ec91a89d08ae9788d1e0c3659c5944563e646029fc371cdc6ab`.
The binary SHA-256 is
`d18f2618e859b5773968e11ab9269b7c72bc23817fb277e8ab0ff1e8839f9e8b`.

The fixture is also wired into future `benchmark-binary` release gates;
this run verified the existing packaged binary, not a new source build.
Initial run `42366686740` stopped at checkout because its abbreviated SHA
was treated as a branch name. No regression ran in that failed setup.
Run `42366773849` passed its assertions, but independent artifact analysis
found that the fixture's JSON round trip also rounded integer nanosecond
identities. The final fixture replaces only the signature's bytes; independent
analysis verified that every other byte remains identical. The earlier run
is superseded for the single-field fault claim.

This verifies revocation for the next invocation. It does not demonstrate
same-job rerunning of other cohort members after a fault that changes their
actual results; the injected signature fault leaves all actual tests passing.
The later [cohort recovery regression](lineage-cohort-recovery.md) reproduces
and corrects that separate same-job defect using a result-changing fault.

This closes an E1 integration-evidence gap. It changes no production policy,
Action pin or benchmark pin, and demonstrates no new performance result.
The original cold 12-commit window and 50% runner-time goal remain unmet.
