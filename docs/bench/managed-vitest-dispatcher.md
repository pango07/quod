# Managed dispatcher product gate

The [product gate](https://github.com/pango07/quod/actions/runs/42389105301) passed at
[`c96a3ba4`](https://github.com/pango07/quod/commit/c96a3ba4b075921afc3b16301292158b11e3889c). Independent
artifact audits passed for both jobs. The dispatcher follows the experimental
[managed Action contract](../managed-vitest-action-v0.md); its default remains
`off`. This gate does not enable production replay or verify the complete
Action hook and remote cache transport.

A later [focused gate](https://github.com/pango07/quod/actions/runs/42391903760)
also passed for the native ordinary-remainder adapter. Its verified evidence is
recorded below without replacing the original workload and boundary proof.

## Original workload and boundary checks

The unchanged c11 stream property file passed all 47 tests in both managed and
ordinary execution, with every name and state agreeing. Its original three
property seeds and 24 cases per seed remain unchanged. Managed completion was
sealed: pending timers, pending timer callbacks, pending microtasks and pending
event callbacks were all zero.

The gate passed 40 compiler checks, 31 capability checks in each of two fresh
processes, 18 isolated adversarial boundary modes, 12 real Vitest fixture states,
and seven module fixture states. All 107 receipt admission checks passed; that
matrix includes real compilation with constructed passing responses and is
separate from actual runtime replay.

| Paired diagnostic | Seconds |
| --- | ---: |
| Uncached managed phase | 160.263 |
| Managed bundle compilation | 1.022 |
| Ordinary command | 85.428 |

This fixed managed-then-ordinary diagnostic runs on one host. It establishes
correctness for the reviewed file and does not establish job savings. The image
was `ubuntu26/20261101.186.1` with Node `v24.18.0`.

## Actual dispatcher controls

All 12 cases and 45 checks passed. Each case starts a separate dispatcher
process on the same GitHub runner. The hit publishes the seeded receipt result
without starting a managed test worker; native preflight and compilation still
run. Missing authority, an incorrect digest and a coherently forged cache
receipt all execute the managed test again.

| Control | Execution | Exit | Command seconds |
| --- | --- | ---: | ---: |
| seed | managed | 0 | 3.411 |
| hit | replay | 0 | 2.265 |
| missing-trust | managed | 0 | 3.400 |
| wrong-trust | managed | 0 | 3.409 |
| forged-receipt | managed | 0 | 3.489 |
| unsupported-pass | ordinary | 0 | 3.325 |
| guard-assertion | ordinary | 0 | 3.452 |
| failure | ordinary | 1 | 3.373 |
| unsupported-failure | ordinary | 1 | 3.377 |
| multiple-files | ordinary | 0 | 0.622 |
| custom-config | ordinary | 0 | 0.598 |
| custom-reporter | ordinary | 0 | 0.625 |

The guard-assertion case fails when managed execution denies a data-URL `fetch`
and passes under ordinary Vitest. It confirms that unsupported managed failures
use the ordinary outcome. Both real assertion failure controls exit 1 and emit
no reusable receipt. Multi-file, changed configuration and unsupported reporter
requests preserve their ordinary selected tests.

Receipt authority in these controls is an external trusted digest. A mutable
cache manifest supplies no authority. Authenticated Action cache-key transport
and complete fresh-job behavior are evaluated separately; this same-runner hit
is not a new fresh-runner replay result.

## Native ordinary-remainder follow-up

The focused gate passed at fixture/auditor commit
[`a5766bbb`](https://github.com/pango07/quod/commit/a5766bbb45226426cb689ea60ce972d154980cac).
The runtime remains pinned to
[`2a4d6f2f`](https://github.com/pango07/quod/commit/2a4d6f2f7547dd00e720a32d5b451229784ba8fb),
packaged by Action
[`d79c8635`](https://github.com/pango07/quod-action/commit/d79c8635f4dbc93c842ec1da8bd5ee6bf8e72928).
The original 12 dispatcher cases and 45 checks passed again, including one
same-runner receipt hit. Two additional mixed-execution controls passed all
20 checks, and both independent audits passed locally against verified archives.

Project configuration can override CLI exclusions. An earlier full-suite smoke
therefore ran the candidate again in ordinary Vitest and fell back rather than
publishing a reusable receipt. The new adapter resolves native Vitest project
specifications and excludes the exact candidate project/file instance before
running the remainder. It retains the selected, excluded, remaining and executed
multisets, preserving paths that occur in multiple projects. This is the adapter
used for the 119-entry remainder of the reviewed 120-entry suite.

The focused controls exercise two selected project/file instances: one managed
candidate and one ordinary remainder. The candidate passes exactly once, while
the ordinary test controls the whole-command outcome:

| Native remainder | Exit | Merged tests | Reusable receipts | Command seconds |
| --- | ---: | ---: | ---: | ---: |
| passes | 0 | 2 | 1 | 3.284 |
| fails | 1 | 2 | 0 | 3.240 |

Both cases verify the native selection before and after execution, the complete
merged report, preserved ordinary report rows, and console-marker multiplicity.
The failed whole command makes no receipt-record callback despite its passing
managed candidate. These small controls do not establish complete 120-entry
execution, remote cache reuse, or twelve-commit savings.

The [first focused attempt](https://github.com/pango07/quod/actions/runs/42391646433)
remains failed evidence: its first eight selection/outcome checks passed, then
the console-marker check failed because its JSON-only reporter suppressed native
console output. Adding the default reporter made that observation available;
the runtime bytes did not change for the successful rerun.

The successful [dispatcher job](https://github.com/pango07/quod/actions/runs/42391903760/job/128461043111)
took 69 complete runner-seconds, or two modeled rounded minutes. Its archives
were `managed-dispatcher` (13581333524, 1,882,414 bytes, SHA-256
`5f849cc7b08897db14c757d70762e150650e35e38bd84172b862e3b842f6a600`) and
`managed-dispatcher-remainder` (13581288491, 462,175 bytes, SHA-256
`17b7263e93e6112170337edf3cba7eed9940cd79418a3b046419cf8ff0bd3b0d`).
Both exact byte counts and digests were verified before extraction. The
`nativeRemainderFollowup` record in the JSON evidence retains both audits,
archive metadata, timings, file hashes and the failed attempt separately.

## Original product run accounting and evidence

| Successful product job | Complete runner seconds | Modeled rounded minutes |
| --- | ---: | ---: |
| benchmark-managed-dispatcher | 61 | 2 |
| benchmark-managed-runtime | 326 | 6 |
| Total | 387 | 8 |

Rounded minutes are the sum of each complete job rounded upward to a minute,
not invoice data. These figures cover these two jobs only. They exclude the
invalid earlier dispatch and separate Action build/compatibility runs. This
gate does not measure the twelve-commit whole-job window or claim a cost saving.

Both archives were downloaded with their exact GitHub-reported byte counts and
SHA-256 digests before extraction. [The JSON evidence](managed-vitest-dispatcher.json)
contains full archive digests, job URLs, every control result, auditor identities
and retained file hashes. The artifacts are `managed-runtime` (13580977534,
3,535,650 bytes) and `managed-dispatcher` (13580248343, 1,881,197 bytes).

The earlier [fresh-runner research evidence](managed-vitest-verified-replay.md)
and [ordinary twelve-commit window](convex-helpers-current-cloud-window.md)
remain separate records.
