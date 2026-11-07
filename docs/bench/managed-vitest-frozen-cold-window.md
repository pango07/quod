# Managed Vitest: frozen-slot cold window

The [second 48-job experiment](https://github.com/pango07/ci-bench/actions/runs/42396324964)
and its independent data audit passed. All twelve original commits ran once in
each of four variants, with separate cache namespaces starting cold for the
managed and uncached-control chains. Subsequent jobs may reuse dependencies;
the uncached control disables candidate receipt reads and writes throughout.
All 24 managed/control archives were verified
against GitHub's raw archive sizes and SHA-256 digests before extraction.
The [complete evidence](managed-vitest-frozen-cold-window.json) retains the
auditor output, artifact provenance, original state fingerprints, per-job
durations, timing budget and concrete remaining gaps.

| Original cold twelve commits | Runner-seconds | Sum of rounded minutes |
| --- | ---: | ---: |
| Checkout/setup floor | 127 | 12 |
| Ordinary baseline | 1,848 | 35 |
| Uncached managed control | 1,884 | 36 |
| Managed with receipt reuse | 857 | 20 |

These are complete successful job durations, including install, build, test,
diagnostic transfer and post work. Queue time and the coordinator are excluded.
Modeled minutes are `sum(ceil(each job's runner-seconds / 60))`; they are not
the ceiling of the chain's total time. The actual GitHub runner charge for
these public repositories is **$0** under GitHub's
[public-runner compute policy](https://docs.github.com/en/actions/concepts/billing-and-usage).

The **runner-time bar is met**: 857 seconds saves **53.63%** against this fresh
ordinary baseline and **50.61%** against the earlier 1,735-second reference.
The conservative limit is 867.5 seconds, requiring at most half of both
ordinary measurements. The **modeled-minute bar is unmet**: 20 minutes saves
**42.86%** against the fresh 35-minute baseline and **44.44%** against the
earlier 36-minute reference. Half of the fresh baseline is 17.5 minutes, so
an integer modeled total must be at most 17. Five replay jobs exceed 60 seconds;
the cold first job takes 229 seconds/four minutes. Excluding that required
cold job gives 628 seconds/16 minutes and does not replace the window result.

There are **11 authenticated partial candidate hits**, one cold managed
execution, zero fallbacks and **zero full-job hits**. All hits start zero
managed test workers, repeat capability preflight, and retain the 119 native
project/file instances. The cold receipt receives positive cache SDK ID
`10168652595`. Each hit's actual authenticated SDK-returned service key binds
the exact original response and receipt bytes; no payload manifest supplies
authority. All twelve uncached controls have no receipt transport and their
complete candidate responses equal the selected responses at each commit.

There are zero disagreements in original Vitest full test names and statuses:
120 project/file instances per job and all 49,557 original states across each
chain, including the skip and the later added tests. The original candidate's
47 tests, three property seeds and 24 cases per seed remain unchanged. Exactly
119 native remainder instances execute without rerunning that candidate,
including paths repeated under different Zod project aliases. The original
native release-test tail also runs at c2–c11 and matches ordinary results.

The **floor-plus-8-second full-job target remains unmet**. C1 takes 37 seconds
against its 9-second floor and 17-second target. The native remainder takes
13.934 seconds of the 16.467-second dispatcher. Subtracting its entire
17-second test step arithmetically still leaves 20 seconds outside testing.
This budget subtraction is not a forecast of an implementable speedup.

The uncovered roster is 36 edge instances, 81 Zod project instances and two
jsdom/React instances. Zod alias/context transfer and multi-instance planning
are implementation gaps. The guarded Web/storage operations, DOM behavior and
native Node release integration require additional reviewed execution
contracts. The release tests exercise filesystem writes, child Node/npm/Git
processes, scratch repositories and loopback HTTP. They cannot run unchanged
inside this closed workerd policy. Full-job reuse also needs complete-command
authority and an early install/build decision that preserves required outputs.
The JSON evidence retains 35 pinned source references for this assessment;
no such broader capability or future floor-time result is claimed here.

Reporter metadata compatibility remains incomplete: the merged report counts
120 file instances as suites while native Vitest's 119-instance remainder
alone counts 798 nested suites. Managed assertion ancestor/title structure is
flattened and managed duration/file-time fields are omitted. Assertion counts,
full names and states are correct; the original native remainder rows and
snapshot data are preserved. Zero wrong states does not mean complete native
JSON metadata equivalence.

This separately keyed policy preserves seven original Array/Promise slots
before freezing four constructor/prototype objects. It has explicitly changed
inherited assignment semantics; older-policy receipts cannot be admitted.
The [product and boundary gate](managed-vitest-protector-gate.md) retains all
original checks plus 51 policy checks in two fresh processes and 16 policy
and cross-profile receipt checks. The packaged Action passed all 22 checks.
The same-host diagnostic is fixed-order and does not establish a general or
cross-window causal speedup. The [first complete window](managed-vitest-cold-window.md)
remains separate: 889 seconds/19 modeled minutes under its earlier policy.

Measured pins are Action
[`c77d8656`](https://github.com/pango07/quod-action/commit/c77d86564686c439aa8e5b9d42a1f8573da771b8),
runtime source
[`293afb71`](https://github.com/pango07/quod/commit/293afb713a1ceab141f290db8a002728298cb5c0),
and harness
[`73c786a5`](https://github.com/pango07/ci-bench/commit/73c786a58cbc48597517b99b3e8eec47c898165f).
The optional one-line [Action integration](../managed-vitest-action-v0.md)
remains experimental and off by default. Runtime bytes match the passing gate
and pinned packaged source; subsequent evidence commits change no runtime code.
