# Cloud ledger saves in the Action post step

Action `41d6c96a2c78b2e1989e1710bae23eb9e165b86b` on
`exp/cloud-post-save-codex` contains the tested upload change. Its engine
remains `bin-f9d8153`; input checks, witnesses and replay policy are unchanged.
This is an experimental Action pin, not a production Cloud deployment.

The hook sends each test report inline, then marks changed suites for the
post step. Post uploads each suite's final ledger once, alongside the
existing Actions cache and dependency saves. It awaits every save before
removing the config containing the OIDC request credentials. At most two
Cloud CLI processes run at a time. A full replay queues no ledger upload.
Old configs keep inline uploads; failure to write the queue marker falls
back to an immediate save attempt. Failed tests preserve their exit code
and still queue changed quarantine state.

[Gate 42351489778](https://github.com/pango07/quod/actions/runs/42351489778)
passed for source `7856037c6277e38f28f615bfdd22e63311ed2068`, using harness
`b44e63230879374c8be2c33e4289e5221f9dcf0a`. It built both ncc bundles on
GitHub and passed all 14 source tests. The existing packaged Vitest checks
also passed: two lineage promotions, one plain audit with zero
disagreements, the lineage switch, recorder upgrade/downgrade and legacy
ledgers, configuration proof, and boolean proof.

The new source regressions cover coalescing, read-only and full-hit jobs,
failed tests, shadow mode, queue metadata failure, bounded concurrency,
independent cache completion, failed saves, literal arguments and symlink
rejection. The bundled post regression checks that config remains available
until a real child process exits and is then removed, for both successful
and failed children. An earlier gate failed because the fixture's mocked
`process.exit()` returned; the corrected fixture terminates execution.

Artifact `13565966883` is 2,482,037 bytes, SHA-256
`73e51e683d70c31b950440010efc5123b66ba164e11902d18809e2cfd916ad52`.
The downloaded archive matched GitHub's digest, and only its remote-built
`dist/main` and `dist/post` files were copied into the packaging commit.
No local builds or test suites were run.

The gate uses a fake Cloud child for error-path checks. A real Cloud-connected
old/new comparison remains required before claiming a performance benefit.
The existing CLI logs a Cloud transport failure and returns zero, so a
post-step completion line alone does not prove that the server stored the
ledger. Verification must include the Cloud save log and the next job's
restore. The Actions cache save remains independent.
