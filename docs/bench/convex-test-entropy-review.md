# Expensive property test: time and randomness source review

This is a static review of the exact c11 source and installed package
versions, not a runtime determinism proof or permission to replay. It
checks whether a proposed execution contract would accidentally assume
that the property's seeded generator is its only source of variability.

The test's own generator uses three fixed default seeds and 24 cases, with
explicit environment overrides. It also mocks `Date.now` in `beforeEach`
and restores mocks in `afterEach`. No cases, seeds or mocks were changed.

The exact `convex-test@0.0.54` package supplies document IDs from a
per-database counter (`dist/index.js`, `_generateId`, line 79). These are
not randomly generated IDs. Insertion reads `Date.now` and advances a
per-database creation-time value when necessary (lines 103–107). The test's
existing clock mock therefore matters to the database semantics it checks.

There is nevertheless another random value in the `t.run` path: a request ID
created using `Math.random` and passed to `invokeAction` (lines 1767–1769).
In `convex@1.46.0`, `server/impl/registration_impl.js` constructs action,
auth, scheduler, storage and vector-search contexts from that request ID.
The inspected action/storage implementations retain it in closures and
forward it with their corresponding syscall arguments.

This review does **not** establish that the random request ID affects any
assertion in `stream.property.test.ts`. It also does not prove the value
irrelevant across all supported observers. Do not replace that distinction
with either "randomness makes the test uncacheable" or "these are only IDs,
so discard them." A runtime contract must account for observable data and
effects; a string mask or two matching runs is not that contract.

Both package archives were fetched from the public npm registry and checked
against their published SHA-512 integrity values before source inspection.
No package scripts, installs, builds or tests were executed locally. The
[evidence](convex-test-entropy-review.json) retains archive URLs, integrity
values, SHA-256 digests for the archives and inspected files, the exact
test-source hash and the limits of the review.

This is positive evidence for investigating reproducible execution of the
unchanged property test: its document identities and test data already
have deliberately repeatable construction. It supplies no new speed,
cost, hit-rate or correctness claim.
