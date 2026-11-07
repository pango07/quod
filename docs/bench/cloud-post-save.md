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

The gate uses a fake Cloud child for error-path checks. The existing CLI
logs a Cloud transport failure and returns zero, so a
post-step completion line alone does not prove that the server stored the
ledger. Verification must include the Cloud save log and the next job's
restore. The Actions cache save remains independent.

## Real Cloud comparison

[Run 42352064625](https://github.com/pango07/ci-bench/actions/runs/42352064625)
completed all eight jobs successfully. Harness
`4fe8a42fd3c1815bbd451af19d270d2e05d5d3ed` compared the old Action
`e7098a6` with `41d6c96` over original commits c10 and c11, starting cold.
Both use engine `f9d8153`, timing replay off, lineage on and Cloud on.
Each Quod job prepares both Action revisions in this comparison harness.

| Variant | c10 runner-s | c11 runner-s | Total runner-s | Rounded minutes |
| --- | ---: | ---: | ---: | ---: |
| Plain | 132 | 179 | 311 | 6 |
| Inline Cloud save | 177 | 161 | 338 | 6 |
| Deferred Cloud save | 172 | 95 | 267 | 5 |

All four Quod/plain comparisons agree on test-state fingerprints, normalized
output multiplicity and successful conclusions. Each job has 120 file/project
instances and 4,131 test states. There are no test or unit replays, lineage
promotions or lineage audits. All eight jobs use
`ubuntu-26.04/20261101.186.1`. The [raw evidence](cloud-post-save-jobs.jsonl),
[per-job report](cloud-post-save-report.md) and
[independent verification](cloud-post-save-comparison.json) preserve the
individual images, run identities, counts and arithmetic.

Both new-Action jobs saved exactly one final ledger in `Post quod`, with
121 new files each. Their Cloud save times were 5.024 s and 7.295 s. The
next job restored 196 files / 3,559 KB from Cloud in 585 ms. Cache saves
completed independently, before the Cloud saves ended, and post waited for
Cloud completion. The two Cloud save intervals began approximately 0.42 s
and 0.85 s before cache completion was logged. These timestamps confirm
overlap; they do not establish a repeatable end-to-end saving of that size.

The observed 71 runner-second difference is **not attributable to this
upload change** from two samples. On c11 alone, the old test step took
122 s and the new one 61 s; relocating a 6.427 s upload does not explain
that gap. Install and other step times also differ. This run verifies
publication, subsequent restore, awaited completion and result agreement.
It does not establish a new 40–50% savings claim or replace the
[full 12-commit result](convex-helpers-current-cloud-window.md).

Artifact `13565224592` is 9,967 bytes, SHA-256
`cedae56fd8d6716ca501a94858c172cbba9f46e3a088a41171e381e8f3b28b3b`.
The archive digest, all eight exact harness/run identities, per-job rounding
and four result comparisons were independently checked. Event excerpts and
digests of the four Quod job logs are in the verification JSON.
