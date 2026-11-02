# Trust v2: replay after the first record

2026-11-02 · exp/trust-v2. Supersedes the "proofs 2" default of exp/trust
(DECISIONS 2026-10-31); everything else of the trust layer stands.

## Why

On 40 real cli/cli commits (docs/bench/0b-x86-hetzner.md, bench/x86-hetzner),
`go vet` had unchanged inputs on 8 commits and `go test` on 6, and Quod
replayed 2 of those 14: the other 12 were a manifest's *second* identical
run, which under "proofs 2" executes to prove itself, and only commits 32-34
gave three identical trees in a row. Proof streaks are per (key, manifest),
and in a linear history almost every manifest is seen once or twice. Proofs
essentially never accrue in normal commit traffic.

Two risks have been living under the one word "proof", and they are not the
same:

1. **Under-recording** (a false hit): the manifest misses an input, so a
   changed tree matches a stale result. This is a correctness bug in the
   observer and stays forbidden. It is guarded by over-recording, A7 (0
   false hits over ~30k mutations per run), W4, and the `equivalent` check
   on every compared execution. Proofs never guarded a *hidden input
   channel*: two executions on the same tree read the same hidden input and
   agree. They did give one thing: v1's second proof ran `equivalent` on
   every manifest before anything replayed, so an observer that *lost* an
   access (a dropped or raced log entry) was likely caught on that
   manifest's second run. Under v2 that in-production check runs on the
   key's audit schedule, not once per manifest; the mutation suites (A7, W4)
   remain the guard against a lossy observer. Stated again under residual
   risks.
2. **Nondeterministic output**: the step's result is not a function of its
   recorded inputs (a timestamp printed, a random port in a log, a test that
   depends on the clock, a flaky test). This is what the second proof and
   the audits guard: a result seen once could be the step's lucky or unlucky
   day.

Trust v2 keeps the first risk's protections untouched and re-targets the
second: the evidence that a *step* is deterministic is pooled across every
manifest of its key, instead of being demanded again for every new tree.

## What changes

| | v1 (shipped) | v2 (this branch) |
| --- | --- | --- |
| Identical executions a manifest needs before replay | 2 | **1** (`--proofs 1`); any N stays available per invocation (`--proofs`), per runner pool, and per step pattern (hooks `step_proofs`) |
| Audit schedule | every 20th would-be hit of a key (5%) | **shadow phase**: the key's first `shadow` = 2 would-be hits are audited (100%), then a 1/k decay to the 5% floor |
| What a divergence does | quarantines the key; the bar rises by one per recent divergence (cap +5) | the same, **and** the shadow phase restarts and lengthens (`shadow × (1+d)`), the floor rises (`5% × (1+d)`), and **every replay the key has served since its last quarantine is marked `suspect`** in the ledger |
| What is compared | exit code; stdout/stderr up to go test durations and the GOGCCFLAGS build dir; outputs exactly, except scratch dirs | the same, plus ISO 8601 timestamps and test-runner durations erased from **stdout/stderr only**; files never normalized |
| Determinism class | none | `exact`, `normalized` (output carries recognized volatile fields), `divergent` (recent divergences); shown in `status` and `inspect`; `divergent` gets the stricter schedule above |
| Fingerprint tag | `quod result fingerprint v4` (main, exp/never-worse's `(cached)`) | `v5` (the normalization changed) |
| Ledger | schema v5 (main, exp/never-worse's lookup index) | **v6**: `runs.suspect`, `trust.shadow_from`, `trust.volatile`, index `runs_by_key`; **v7**: `audits_pending` (deferred audits) |
| Where a decay-phase audit runs | inline: the job waits for the re-execution | **deferred** (exp/deferred-audits): the replay is served at once and `quod audit` re-executes and compares it in the background; shadow audits stay inline; the hooks stay inline for now |

Where the hit rate comes from: a key must agree with itself twice (two
audited would-be hits, on any of its manifests) before anything of it replays
blind. After that, a new manifest replays on its second identical run.
Measured (see "Measured"): the 14 unchanged-input step runs that gave 2
replays under v1 in 0b's history give 7 under v2 (vet 4 of 8, test 3 of 6;
the other 7 are the shadow audits and the decay audits, executed and
compared, all agreeing), and in PR-then-merge traffic 57 of 74 unchanged-
input runs replay (`go test` 13 of 18) against v1's 21 of 74 (`go test` 1 of
18) on the same jobs.

## The schedule

For a key, let k = 1, 2, ... count its would-be hits (lookups that found a
proven, provable result) since the key's last quarantine (`trust.eligible -
trust.shadow_from`). The audit *weight* of hit k is

    w(k) = 1                      for k <= K          (shadow)
    w(k) = clamp(K / k, r, 1)     for k > K           (decay to the floor)

with K = `shadow` (default 2) and r = `audit` (default 0.05). The schedule
is deterministic, as v1's was: hit k is audited iff
floor(S(k) + ε) > floor(S(k−1) + ε) with S(k) = Σ_{j≤k} w(j), so the same
ledger always gives the same schedule, a step never sees the count, and
`--audit 0` turns every audit off (shadow included; benchmarks only, as
before). With `--shadow 0` the schedule is v1's: every 1/r-th hit.

Audits by hit number, defaults (K = 2, r = 0.05): 1, 2, 4, 7, 11, 19, 31,
49, 69, 89, 109, ... (10 of the first 100; v1: 5 of the first 100, none of
the first 19). Expected audits among the first n hits ≈ K + K·ln(n/K) until
the floor takes over at k = K/r = 40, then r per hit. The schedule is
asserted in `v2_schedule_shadows_then_decays_to_the_floor`.

After d recent divergences (among the key's newest 100 audits, d capped at
`MAX_EXTRA_PROOFS` = 5, as in v1): the bar is `proofs + d` identical
executions to re-prove (v1), K' = K·(1+d), r' = min(1, r·(1+d)), and the
count restarts at the quarantine. A key that diverged once and re-proved is
audited at 1, 2, 3, 4, 6, 7, 10, 12, 16, 20, 26, 33, 43, 53, 63, ... (every
10th once the floor holds; the sum does not land on an integer where the
floor takes over, so the floor's first audit is at 43, not 40).

Shadow audits are *inline*: the would-be hit executes, is compared, and its
own result is what the job gets. The hit is still a proof: the manifest's
streak advances, so a step's second run is both the shadow audit and proof 2.

### The cost model: inline for the shadow, deferred for the decay

An audit costs one execution D of the step instead of a ~1.3 s replay.
Inline, that D is wall time the job waits for; deferred (serve the replay
now, re-execute later, compare, mark the served replay `suspect` if it
differs), it is machine time nobody waits for. Deferred is cheaper for the
developer and the same for the machine, and the retroactive marking is
exactly its safety net. But a deferred audit means a wrong result has been
*served* (and perhaps merged on) before anyone looks. The shadow phase
exists so that nothing of a key is served blind until the key has agreed
with itself; deferring it would undo that. Decision: inline for the shadow,
deferred for the decay and the floor ("Deferred audits" below; the first
version of v2 had everything inline, which 0e measured at +172 s per 87
jobs). Deferral never applies to the shadow.

## Failure modes and the math

Model a step whose execution on a fixed tree produces a result other than the
recorded one with probability p (p = 1: a timestamp in stdout; p small: a
flaky test). A wrong replay is a replay served while the step would have
produced something else; p of the replays served before detection are wrong.

**Always-different output (p = 1).** Hit 1 is a shadow audit: it executes,
differs, quarantines the key. Zero replays served, zero wrong. v1 was the
same for the first manifest (proof 2 of 2 diverges) and the same for every
later manifest; v2 is the same for the first K hits and then relies on the
step having agreed with itself K times. A step that is deterministic on K
trees and prints a timestamp on the (K+1)th is the residual (see below).

**Flaky output (0 < p < 1).** Detection happens at the first audit that
sees a different result. The shadow fails to detect with probability
(1−p)^K; after it, each audit detects with probability p, so the expected
number of audit cycles after the shadow is 1/p, each cycle serving about
(1/w − 1) replays, w the weight at that point. With the floor reached:

    E[replays served before detection]  ≈ (1−p)^K · (1/r − 1) / p
    E[wrong replays served]             ≈ (1−p)^K · (1/r − 1)

| p | v1: (1−p)·19 | v2, K=2: (1−p)²·19 | v2 after one divergence (K=4, r=0.1): (1−p)⁴·9 |
| --- | --- | --- | --- |
| 1.0 | 0 | 0 | 0 |
| 0.5 | 9.5 | 4.8 | 0.6 |
| 0.2 | 15.2 | 12.2 | 3.7 |
| 0.05 | 18.1 | 17.1 | 7.3 |
| 0.01 | 18.8 | 18.6 | 8.6 |

The table is the single-manifest model (one tree, hit again and again).
v1's single implicit audit was the second proof, once per manifest; v2's K
are once per key. At the same floor, v2 serves fewer wrong replays for every
p on one manifest, and the decay (not in the formula, which assumes the
floor from hit K+1) makes it better still early in a key's life: hits 3-40
are audited at 1/k, i.e. 10 audits where v1 had 1. With M distinct
manifests v1 had M implicit audits where v2 has K plus the decay; that is
the per-manifest weakening named below.

**Expected time to detect**, in would-be hits, for a step that turns
nondeterministic (p) at hit k₀ in its life: the next audit after k₀ is at
most 1/w(k₀) hits away, then one audit per 1/w until one differs; E ≈
(1/w)·(1/p). At the floor: 20/p hits (v1: the same). In the shadow: 1/p.
During the decay at hit k: (k/K)/p.

**What proofs never caught and still do not.** A hidden input (p = 0 on a
fixed tree, wrong when the hidden input changes) is a false hit by
under-recording. It is caught by the mutation suites and by `equivalent`
(an execution that reads a path the hit's manifest did not is a divergence),
not by agreement counts. A result that depends on the wall clock in a way
that is constant within a day (a date in an output file) is p = 0 within the
day and p = 1 across midnight: v1 proved it with two same-day runs and
replayed it next day until the 20th hit; v2 does the same, except that the
shadow may spend its audits on the first day. Both are bounded by the floor,
and v2's suspect marking names the replays to re-run once an audit catches
it.

**Per-manifest nondeterminism.** A step deterministic on most trees and
flaky on one (a test that is racy only when a certain file exists). v1 gave
every manifest one implicit audit; v2 gives a new manifest none once the
key's shadow is spent, and relies on the floor and the decay. This is the
one place v2 is weaker than v1 per manifest, and the reason `--proofs 2`
and `step_proofs` stay available: a repository that knows such a step can
name it.

## Deferred audits

2026-11-02 · exp/deferred-audits. 0e (docs/bench/0e-combined-x86.md on
bench/combined-x86) put a number on inline audits: +172 s over 87 jobs of
realistic traffic and +99 s over 39 history commits, because an audit
re-runs the full `go test` with Go's test cache expired (40-41 s against
plain's 10-13 s) while the job waits. This section takes the decay-phase
audits off the job's critical path without weakening what they check.

**What happens now.** When replay mode selects a would-be hit for an audit
and the key is past its shadow phase, the replay is served at once, exactly
as a blind replay is (the job gets the recorded streams, outputs and exit
code), and the ledger records, in the transaction that records the replay,
a *pending audit*: the key, the manifest and result that were served, the
would-be hit number, the exact input tree as a document in the store, and
what the executor needs to run the step again. `quod audit` executes it
later: it rebuilds the tree from the store, checks the manifest still
matches it, runs the step with go test's cache expired (as every inline
audit does), and records the execution under `Reason::DeferredAudit`,
which compares it with the served result through the same
`Trust::record_execution` as an inline audit. A divergence does what it
always did: quarantine the key, reset its proofs, restart and lengthen the
shadow, and mark every replay the key served since its last quarantine
`suspect`, the one the audit checked included. The pending row is settled
(`passed`, `diverged`) in that same transaction.

### Where the audit runs

Three places were considered.

- (a) **In the same job, after the step, while later steps proceed.** The
  step's input tree is already a content-addressed document (the imported
  workspace plus the served `.git`), so the audit never touches the live
  workspace: later steps may mutate it freely. But an audit running beside
  the job's remaining steps competes with them for the same CPUs, and
  `go test` wants all of them: the job's later steps would slow down by
  about what the audit saves.
- (b) **At job end, as a post-job hook.** actions/runner's `cleanup_job`
  hook is the point (quod-hooks runs it); the runner waits for it, so an
  audit executed *inside* the hook is still job wall time from the
  runner's and GitHub's view. The hook can start a detached worker, which
  is (a) at job end.
- (c) **Queued to another runner or to Quod Cloud.** The pending row holds
  everything a remote executor needs (the tree digest, the request), so
  this is a transport question, not a model question. It needs the store
  shared or shipped, and a shared ledger (exp/cloud-ledger-v2). Not now.

**Chosen: a detached worker on the same machine, at idle priority, started
by the run that served the deferred replay, and runnable by hand or by a
timer (`quod audit`).** `quod run` and `quod step` start it right after
serving the replay (`setsid`, output to `<state>/audit.log`; a run that
only replays has no observer mounted, so nothing of the run is held up).
The worker takes `<state>/audit.lock` (one worker per state dir; an extra
one exits at once), puts itself in the idle CPU class (SCHED_IDLE, nice 19 where
that is refused) and the idle I/O class (so a job executing beside it,
on a shared box or on the same runner's next job, keeps the CPU: the
kernel runs an idle-class task only when nothing else is runnable), expires what nobody ran in time, and drains the
queue oldest first within a time budget (`--budget`, 1 h; each audit also
has the step's own timeout). `QUOD_AUDIT_SPAWN=0` leaves the start to the
caller: a cron job, a runner's post-job hook, the tests. On a runner pool
this is (a) and (b) at once: the audit runs between jobs or under the next
one at idle priority, from the recorded tree, never from the next job's
workspace.

**The hooks keep every audit inline for now.** A hooks step runs on a
hermetic surface built per job from the job's secrets, layers and command
files (`quod_hooks::surface`), which the hooks never store past the job
and must not (secrets at rest). A deferred hooks audit needs that surface
serialized with the pending row under the same rules as the rest of the
state; that is the follow-up, and it is a design question about secrets,
not about audits. `quod_hooks::Policy::trust` sets `defer: false`. The 0e
numbers were measured through the CLI driver, so the expected effect below
is what the driver sees; a runner job sees it once the hooks follow.

### The exposure window

The window is the time from a deferred replay to its audit's verdict,
during which a wrong replay may have been consumed. Inline audits have a
zero window for the audited hit, and *only* for that hit: every unaudited
hit between two audits was always served blind and never re-checked
except by the next audit. Deferral adds, for the audited hit, the time
the worker takes to get to it. With the worker started by the run itself
and the queue empty, that is one execution D of the step (plus the
worker's start): the verdict is in before the next job on a runner
finishes its own `go test`, and in the 0e traffic (a job every 30 s or
so, D ≈ 41 s for `go test`) before the next would-be hit of the key in
most cases. When it is not, the hits served meanwhile are exactly the ones
the retroactive marking names.

In the model of "Failure modes and the math" (a step whose execution
differs from the recorded result with probability p), the expected number
of wrong replays served before detection was (1−p)^K · (1/r − 1) with
every audit inline. Deferring a decay audit changes two things: the audit
still detects with probability p when it runs, but the replays served in
the window W (H_W of them, the key's would-be hits between the deferred
replay and the verdict) are served before it does; and the audited hit
itself is now served, so the audit cycle serves 1/w replays instead of
1/w − 1. Hence

    E[wrong replays served] ≈ (1−p)^K · (1/r + H_W)   (deferred, at the floor)
                            ≈ (1−p)^K · (1/r − 1)     (inline)

H_W is bounded by the worker: with the queue drained as it is filled,
H_W ≤ 1 for a key hit at most once per D. The cap `MAX_DEFERRED_PER_KEY`
= 2 bounds it when the worker falls behind: a key with two audits
outstanding has its next due audit executed inline, so at most two audit
intervals of a key (2/w(k) hits; 40 at the floor) are ever served without
a verdict, and a lost audit (below) brings the next hit inline at once.
For p = 1 nothing changes: the shadow catches it before anything is
served. For small p the inline and deferred expectations differ by
(1−p)^K · (1 + H_W), about one extra replay per detection: against the
(1/r − 1) = 19 the floor already allows, the window is a 5-10% addition
to a bound the schedule already accepts, bought for the whole audit cost
leaving the job.

**Shadow audits stay inline.** The shadow is the key's first evidence of
agreeing with itself; before it nothing of the key has been compared in
production, and a p = 1 step (a timestamp in stdout) is caught at hit 1
with zero replays served only because hit 1 executes. Deferring the
shadow would serve K replays of a key that has never been checked, which
is the one thing v2 promised not to do. The shadow also costs a bounded 2
executions per key per quarantine, where the decay costs ~10 of the first
100 hits and then 5% forever: the decay is where the wall time is (in 0e,
11 of 17 audits), the shadow is where the safety is.

### Lost audits

A pending audit that is never executed must not count as passed. The row
has five states: `pending`, `running` (claimed by a worker, with when and
by whom), `passed`, `diverged`, `lost`. Before claiming, a worker expires
the queue: a `pending` row older than `PENDING_TTL` (24 h) is `lost`
("nobody ran it"), a `running` row claimed longer ago than `LEASE` (3 h)
was abandoned by a dead worker and goes back to `pending` for another
attempt, `lost` after `MAX_ATTEMPTS` = 3. No attempt runs longer than
the lease either: the step gets the smaller of its own timeout and
`LEASE`, so a hung step cannot hold the worker's lock past the point
where the next worker would call the claim abandoned. An attempt that
produces no verdict (a Quod error, the machine's doing: an OOM kill, a
signal, a timeout, which under the worker's idle priority is the box's
as likely as the step's) releases the row the same way and leaves the
manifest's proof streak alone (an inline timeout resets it because the
user saw it). What no retry would change is `lost` at once: a request
that no longer parses, a manifest or result that is gone, a tree that no
longer matches, a result that is not the step's (a sealed-tree write
refused, a second Go toolchain outside the test-cache guard, the guard
no longer applying after a toolchain upgrade). The counters say so:
`trust.audits` counts executed comparisons only, never a pending or lost
row.

The schedule does not skip ahead over a loss: a `lost` row is *owed*
until an inline audit of the key repays it, and so is a `pending` row
older than `PENDING_TTL` whether or not a worker has marked it (the
decision reads the row's age, not its state, so a worker that never
starts cannot leave an audit counting as merely pending). `decide` sees
the key's backlog (`Ledger::backlog`: pending and owed counts) and, while
anything is owed, audits the key's next would-be hit inline whatever the
schedule says (a shadow audit if the key is in its shadow); that audit's
row repays the oldest owed one, an overdue pending row becoming `lost`
by that. The schedule's own audits (at 4, 7, 11, ...) are unaffected:
hit k is still audited at k. `quod status` prints "deferred audits: N
pending, M running, L lost and not yet repaid"; `quod audit --list`
names them.

A quarantine supersedes the key's other outstanding audits. Each of them
would compare the step's behaviour now against a served result the
quarantine has just declared suspect, and so count the one divergence
again (a longer shadow, a higher bar, the fresh proof streaks wiped) and
mark the replays the re-proof later serves. At a quarantine, inline or
deferred, `record_execution` drops them: `lost`, repaid (the restarted
shadow checks the key harder than an owed inline audit would), with the
reason on the row; a worker that claims one whose replay is already
suspect drops it the same way. The replays those audits were checking
are suspect already, by the quarantine.

### Resources

A worker is one process per state dir (the lock), in the idle CPU class
(SCHED_IDLE) and the idle I/O class; the step it runs inherits both. The
budget (`--budget`, 1 h) bounds a drain; each step has its recorded
timeout, capped at the lease (3 h). Nothing in the job's
own path waits: the replay is served, the worker is started, the run
exits. On a runner, a worker started by job N runs under job N+1 at idle
priority and is preempted by it; the window grows by the next job's
length, which the retroactive marking covers.

### Inputs

The audit never reads the live workspace. The replay that owes the audit
stores the tree it was decided on as a document in the store
(`rootfs::put_tree`: the imported workspace with the served `.git`
grafted, every blob already in the store from the import); the pending
row names its digest. Content-addressed, so a tree seen before (the case
of a replay, by definition) costs a hash and no write. The worker
rebuilds the tree from the document, checks `matches(manifest, tree)`
(a store that lost a blob fails loudly, never passes), checks the stored
spec keys to the step, and executes the step exactly as the run would
have: same rootfs or host pin, same warm cache, same go test guard, same
timeout, same ignored outputs, same policy. The step's warm build cache
is the state dir's, as for the run it stands in for: unobserved by
design, and the one thing an audit *should* see differently if it has
been poisoned (which is what T4 and D2 exercise).

### Threat model additions

- A deferred audit is decided by the same pure schedule; a step cannot
  learn whether its hit was deferred (it sees a replay either way).
- The pending row, the tree document and the request are written by the
  run that served the replay, in the ledger and store it already writes;
  a worker trusts them as it trusts any ledger row. The request is
  validated: the spec must key to the step, the tree must match the
  manifest.
- A worker that is never started (no cron, spawn disabled, the runner
  gone) turns every deferred audit into an owed one after `PENDING_TTL`
  (by the row's age, in `decide`, with no worker needed to say so), and
  every owed audit into an inline audit on the key's next hit: the
  failure mode is "audits come back inline", never "audits silently
  pass". Until then the cap bounds the exposure to two audits per key.
- The worker holds the ledger's write lock while it claims a row and
  while it records a verdict (the comparison runs inside that
  transaction, as an inline audit's does), at idle priority: a job's run
  that needs the lock meanwhile waits up to the ledger's busy timeout
  (60 s) on a saturated box. Both sections are short; a residual.
- Two state dirs never share a queue; the hooks' per-scope ledgers would
  each have their own (once the hooks defer).

### Expected effect on 0e's numbers

Computed from 0e's per-job rows, which name each audit; the combined
benchmark after the batch will measure it. The decay audits are the ones
past each key's second would-be hit.

| | 0e (inline) | expected (deferred) |
| --- | --- | --- |
| (2) traffic, audits' cost in job wall time | +172 s (17 audits: 6 shadow, 11 decay) | ≈ +69 s (the 6 shadow audits: `go test` at 2 pr and 2 merge ≈ 60 s, `go vet` at 1 merge and 2 pr, `go build` at 2 rerun and 8 rerun) |
| of which `go test` | +149 s (5 audits) | ≈ +60 s (2 shadow); the 3 decay audits (5 pr, 10 rerun, 23 merge: +31.5, +27.6, +28.4 s over plain) leave the job |
| (2) Quod / plain | 2,443 / 2,237 = **1.09x** | ≈ 2,340 / 2,237 = **1.05x** |
| (2) jobs where Quod is faster | 45 of 87 | 48 of 87 (5 pr, 10 rerun and 23 merge flip: ≈ 6, 3 and 6 s against plain's 14, 17 and 17) |
| (1) history, audits' cost | +99 s (7 audits) | ≈ +64 s: the decay audits are `go test` at commit 32 (+30 s) and `go vet` at 26 and 34 (≈ 2.3 s each) |
| (1) Quod / plain | 1,519 / 1,250 = **1.22x** | ≈ 1,484 / 1,250 = **1.19x**; commit 32 flips (≈ 6 s against 16 s): 8 of 39 |

Machine time is unchanged (every audit still executes, 41 s of `go test`
each), plus a tree document per deferred audit (one write for a tree not
seen before; a replay's tree was seen before) and a worker start. The
exposure in 0e's traffic: the three `go test` decay audits would have
been settled within ~45 s of their replay, before the next job's `go
test` finished; none diverged (all 24 audits agreed), so no replay would
have been marked.

## Determinism classification

Every comparison already records whether the two results were byte-identical
or identical only after `normalize`. v2 keeps the count on the trust row
(`trust.volatile`) and derives a class:

- `exact`: every comparison so far was byte-identical.
- `normalized`: at least one comparison needed normalization: the output
  carries recognized volatile fields (go test durations, a timestamp, a
  test-runner duration). Reported, not penalized: the volatility is
  understood and erased, which is the point of the normalization; it is not
  evidence about the step's *results*.
- `divergent`: the key has a divergence among its newest 100 audits. The
  stricter schedule above applies automatically (longer shadow, higher
  floor, higher bar), decaying as clean audits push the divergences out of
  the window, as the bar did in v1.
- `unknown`: no comparison yet.

### Normalizations (stdout and stderr only; never file outputs)

Each erases one shape of volatile text; everything else is kept byte for
byte, and a line of any other shape is untouched. File outputs, root
outputs and command files are compared exactly: a later step could read
them, and a normalized file would be a file Quod silently rewrote.

| # | Erased | Shape | Why |
| --- | --- | --- | --- |
| 1 | go test package duration | `ok  \tpkg\t0.412s`, `FAIL\tpkg\t1.2s` → `<dur>` | v1; every `go test` differs here |
| 2 | go test per-test duration | `--- PASS: T (0.00s)` and FAIL/SKIP/BENCH → `(<dur>)` | v1 |
| 3 | go's temporary build dir | `GOGCCFLAGS=...-ffile-prefix-map=/tmp/go-build<n>=...` | v1; setup-go prints `go env` |
| 4 | ISO 8601 timestamp | `2026-11-02T09:12:03Z`, `2026-11-02T09:12:03.007+01:00`, `2026-11-02 09:12:03` → `<ts>` | logs with a timestamp prefix; a date alone (`2026-11-02`) is **not** erased: it is as likely a version or a fixture as the clock |
| 5 | test-runner duration | a number with a unit in a bounded context: `(12ms)`, `(1.23 s)`, `in 3.21s`, `in 1.2 seconds`, `took 450ms`, `Time: 1.234 s`, `Duration: 2.1s`, `Done in 2.30s` → `<dur>` | jest, vitest, pytest, mocha, cargo test, yarn; only the number before the unit goes, so `1 passed in 3s` and `2 passed in 3s` still differ |

Nothing else: not hex ids, not ports, not temp names (`/tmp/tmp.XXXXXX`),
not `[no test files]`. A shape joins the table with a case that shows two
faithful executions differing there, as HOME_VOLATILE's entries did.
The fingerprint tag moves to v5 (v4 was exp/never-worse's `(cached)` rule,
merged first); stored v4 fingerprints are never compared
with v5 ones (the previous result is re-fingerprinted under the current
rules, as v1 already did for rule changes), so the change is not a
divergence anywhere.

## Pooling across branches, PRs, re-runs and matrix siblings

Checked on main: nothing in the key, the manifest or the ledger is scoped by
ref. The step key is the command, its environment, image and toolchain; the
manifest is tree content; the hooks' ledger is per `<state>/scopes/<repo>/
<trust>`, where `trusted` is every event of the repository's own code (push,
dispatch, schedule, and a pull request or workflow_run whose head is the
repository itself). So a PR run and its merge, a re-run, a matrix sibling
with the same environment, and two branches at the same tree already share
proofs, audits and quarantine within the trusted scope. The CLI's ledger is
one per state dir. No change was needed; v2 makes the pooled evidence worth
something.

One known gap stays (DECISIONS, MAJOR 3): in a job that is not a writer
(a same-repo PR), a go step after the job's first write into its layer over
the warm build cache is "not recorded": it may replay a clean record but
never records, proves or audits. On a docs-only commit the cache already
holds every action, nothing is written, and the PR's `go test` records; on
a code change the PR's `go build` records and its `go test` usually does
not. The follow-up named there (key such steps by the (key, manifest) of the
steps that wrote the layer) is unchanged.

## Threat model

Scopes (`job::scope_of`) are unchanged: `trusted` (the repository's own
events), `fork-<owner_repo>` per fork, `unverified`, each with its own
ledger, warm cache and workspace; only a *writer* (a push or dispatch on the
default branch or an operator-listed protected ref, or a schedule) writes the
scope's warm cache and persisted workspace.

- **A fork's first record never becomes a trusted replay.** Its ledger is
  another file; `find_match` on the trusted ledger cannot see it. v2 does
  not change where anything is written. Tested: `a_forks_record_is_never_
  a_trusted_hit` (quod-hooks) records a result in a fork scope and looks it
  up from the trusted scope of the same repository.
- **A same-repository pull request can record once, and a writer replays
  it.** This is within the trusted scope and is the writer-trusted loosening
  the owner asked for. It is no new capability: under v1 the same PR could
  push twice. Someone who can open a same-repo PR can push to the
  repository. The warm-cache poisoning paths (MAJOR 3, N2) are unchanged:
  the PR's later go steps are not recorded, and its layer never reaches the
  cache.
- **A trusted-scope job can quarantine a key** by making a step
  nondeterministic (a `date` in a script), which stops the default branch's
  replays of that key: a denial of replay, not a wrong result, and visible
  in `status`. v1 had the same. New in v2: the same divergence also marks
  the default branch's earlier replays of that key `suspect`, a visible
  "re-run these" list an operator may act on. Noise, never a wrong result;
  the suspect text names the run and audit that caused it.
- **The audit schedule cannot be gamed**: a step never sees the hit count
  or the schedule, and the schedule is a pure function of the ledger.
- **Suspect marking is append-only facts**: it never changes a stored
  result, only labels run rows; it is written in the same transaction as
  the quarantine.

## Retroactive marking

At a quarantine (any divergence: a compared result that differs, an offline
run that misbehaved, a different read set on a matching tree) the ledger
marks every `replayed` run of the key not already marked with
`suspect = "<when>: <divergence summary> (audit <id>)"`. That is every
replay since the key's last quarantine, or ever: the divergence says the
step's output is not a function of its inputs, and that is a property of
the step, not of the tree it was caught on. `quod status` prints per key
"N replays may have been wrong; re-run them" with the newest run ids, and a
total; `quod inspect <key>` lists them; the hooks' job summary names it in
the divergence line. The dashboard (cloud/, no Quod data yet) reads
`runs.suspect` when it reads runs. A re-proof does not clear a suspect: the
run it labels was never verified.

## Configuration

- CLI `quod run`: `--proofs N` (default 1), `--shadow K` (default 2),
  `--audit r` (default 0.05, the floor; 0 turns every audit off), env
  QUOD_PROOFS, QUOD_SHADOW, QUOD_AUDIT. `quod status --proofs` for the bar.
  `--inline-audits` (QUOD_INLINE_AUDITS) executes every audit in the job
  instead of deferring the decay-phase ones; `QUOD_AUDIT_SPAWN=0` serves
  deferred replays without starting the worker. `quod audit [--list |
  --one | --budget 1h]` is the worker.
- Runner API (`quod step`): `proofs`, `shadow`, `audit`, `inline_audits`
  in the request; a deferred replay's response carries `deferred_audit`
  (the pending row's id).
- Hooks policy (`quod runner pool --proofs --shadow --audit`, or the shim's
  JSON): `proofs`, `shadow`, `audit`, and `step_proofs: [{"contains":
  "<substring of the run: script>", "proofs": N}]`, first match wins, for a
  step pattern that should keep a higher bar. The hooks audit inline (no
  deferral option yet; see "Deferred audits").

## Migration

Ledger v6, one migration, after exp/never-worse's v5 (the lookup index); a
v4 ledger takes both in order in one open: `ALTER TABLE runs ADD COLUMN suspect TEXT`;
`ALTER TABLE trust ADD COLUMN shadow_from INTEGER NOT NULL DEFAULT 0`;
`ALTER TABLE trust ADD COLUMN volatile INTEGER NOT NULL DEFAULT 0`; `CREATE
INDEX runs_by_key ON runs (step_key, started_at DESC)`, and `UPDATE trust
SET shadow_from = max(0, eligible - audits)`: a key's would-be hits before v6 were
mostly blind replays, so only its audits so far count toward its shadow (a
key with 7 hits and 2 audits enters v2 with 2 of its 2 shadow hits spent; one
with 7 hits and no audit starts the shadow fresh). `fixtures/ledger_schema.sql`
is the new golden. No step-key change: the key
describes inputs, not trust rules. The result fingerprint tag is v5. v1
ledgers in the field are none (DECISIONS: disposable).

Ledger v7 (deferred audits): `CREATE TABLE audits_pending (id, step_key,
manifest_id, hit_run, replay_run, nth, tree, request, state, enqueued_at,
attempts, claimed_at, claimed_by, finished_at, audit_id, detail, repaid)`
and its index by `(step_key, state)`. Nothing else changes; a v6 ledger
has no pending audits, so nothing is backfilled. Golden updated.

## Measured

Measured on a dedicated Hetzner CCX23 ("server 2"; method, per-commit
detail and the plain-warm baseline in `docs/bench/trust-v2/hetzner.md`;
drivers beside it), cli/cli's 40 commits, `go build`/`go vet`/`go test`:

| | v1 | **v2 defaults** |
| --- | --- | --- |
| History, one job per commit: replayed / unchanged-input runs | 2 / 14 (0b, server 1) | **7 / 14** (vet 4/8, test 3/6; the other 7 are the shadow and decay audits) |
| Realistic traffic (PR head, merge, 10% re-runs; 87 jobs, same box): replayed / unchanged | 21 / 74 (28%) | **57 / 74 (77%)** |
| of which `go test` | 1 / 18 (6%) | **13 / 18 (72%)** |
| divergences / suspect replays, all series | 0 / 0 | **0 / 0** |

A `go test` replay took 1.7–2.1 s against ~41 s executed. The projection
in "What changes" (7 of 14 in the history) was the measured outcome, commit
for commit.

## Residual risks, plainly

1. A step deterministic on its first K trees and nondeterministic on a later
   one replays blind until the next scheduled audit (at most 1/w(k) hits
   away; 20 at the floor). v1 would have caught it on that manifest's second
   run. `--proofs 2` or `step_proofs` restores v1 for a named step.
2. Clock-dependent results (constant within a day) replay across the day
   boundary until an audit; same as v1; the suspect list then names them.
3. Normalization equates two outputs that differ only in a timestamp or a
   duration. If a step's *meaningful* output is a timestamp (a build that
   prints its own build time as the artifact), its stdout is replayed with
   the recorded time. Files are never normalized, so artifacts are exact.
4. Suspect marking is conservative: every replay of the key since its last
   quarantine, most of which were right when p is small. It is a list to
   re-run, not a claim of wrongness.
5. A same-repo PR's later go steps stay unrecorded (MAJOR 3), so PR → merge
   pooling for `go test` after a code change still needs the merge run to
   record. The hit rate in the traffic simulation below is measured through
   the CLI (one ledger, no reader/writer split) and is an upper bound for
   the hooks.
6. Deferred audits (CLI only). A decay-phase audit's verdict comes after
   its replay was served: for the window between the two, a wrong replay
   may have been consumed, bounded as "The exposure window" says (about
   one replay per detection over the floor's own 19, two audit intervals
   at the worst when the worker falls behind). The hooks still audit
   inline, so a runner job still pays the full audit cost until the
   hermetic surface can be stored with a pending audit. A worker that
   never runs degrades to inline audits after 24 h, not to silence. The
   tree document a deferred audit stores (a few MB for cli/cli) lives in
   the store with no reaper, like every blob.
7. A lossy observer (an access dropped from the log) was likely caught by
   v1's second proof on that manifest (`equivalent` ran before every first
   replay); under v2 that check runs on the key's audit schedule. The
   mutation suites (A7's 0 false hits over ~30k mutations per run, W4) are
   the guard against under-recording, as they always were; this is a
   narrower in-production net for one failure class, not a false-hit path
   in itself.
