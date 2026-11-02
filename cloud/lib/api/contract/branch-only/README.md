The "proving 0 of 1" bug (cloud/trust-plain). One run of `go test ./...` on
a branch that is not a writer (`refs/heads/exp/x`, its own ref's scope),
team proofs 1: what quod-server's test s16
(`crates/quod-server/tests/server.rs`, branch server/trust-branch-scope)
drives.

- `run-12.json`: the run; its receipt says `proof 1 of 1`.
- `steps-v3.json`: the steps list as exp/cloud-ledger-v3 answered it (trust
  read from the trusted scope only): `proving 0 of 1`, the bug.
- `steps.json`: the list with the fix: `replaying`, `trustRef: "exp/x"`.

Written by hand to the shapes s16 asserts (job and runner are empty: s16's
OIDC claims name no job). Record them again with `dump.rs` once the fix is on
the deployed server branch.
