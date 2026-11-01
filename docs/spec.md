

# Quod: Product Spec v0.1
CI that refuses to repeat itself. Name checked free on npm, crates.io and GitHub, Oct 30 2026.
Engineering brief for the spike, written for a coding agent: [Phase 0 handoff]
2026-10-29 · @julio

## Hypothesis
A runner that records what every CI step actually read, and replays proven results instead of re-running them, cuts median CI wall-clock time by 98% on a typical commit. No workflow changes. No false passes.
Two claims hide inside that sentence, and both have to hold:
- 
Redundancy. On a typical commit, 98% or more of CI step-seconds go to work whose real inputs are byte-identical to a previous run. Most commits touch a few files; most steps don't read them.
- 
Observability. Those inputs can be captured at the OS level completely enough that replaying a result is as safe as recomputing it.
What would falsify it:
- 
Measured redundancy below 90% on the benchmark corpus after test splitting. Then 98% is a slogan, not a product.
- 
Any false hit in audit mode that a full run would have caught, once taint rules are tuned. One is too many.
- 
Tracing overhead above 10% on cold runs. Cold runs still happen, and if they get slower nobody adopts.
Secondary bets: the receipt (which run proved this step, on which inputs) is what earns trust, not the speed. And a laptop and CI sharing one ledger means your local test run can finish CI before you push.

## The problem
CI redoes work. Every push boots a fresh machine, installs the same dependencies, compiles the same unchanged packages, and runs tests whose inputs haven't moved since last Tuesday. The commit changed 40 lines. The pipeline acted like it changed everything.
The market's answer has been to run the redundant work faster. Bare-metal runners, co-located caches, Firecracker boots in under three seconds. That lowers the bill and shaves minutes, but the shape stays the same: a 14-minute pipeline becomes an 8-minute pipeline, and every one of those 8 minutes is still spent recomputing known answers.
The tools that do avoid redundant work all require buy-in first. Bazel and Buck2 need every target declared. Nx and Turborepo need your tasks described in their config. Dagger needs the pipeline rewritten as code. Coverage-based test skippers need a per-language agent in your test process. Each asks you to describe your build before it will help, and each only sees what it was told about.
The cost is human time, not compute. An engineer waiting 12 minutes to learn a lint rule failed is the bottleneck. A coding agent that pushes 30 near-identical commits a day and waits for each one is the new, bigger bottleneck. Agents don't get bored, but they do get billed.
Nobody has built the obvious thing: watch what each step really reads, at the OS level, in any language, and refuse to run it again when nothing it read has changed.

## Landscape
Every piece of this exists somewhere. Nobody has assembled them under an unchanged .github/workflows file, and nobody has solved trust.

 | 
Approach | 
Examples | 
What it does | 
The gap it leaves
 | 
Faster runners | 
Blacksmith, Depot, Namespace, WarpBuild, Ubicloud | 
Bare metal, sub-3s microVM boot, co-located caches, sticky disks | 
Runs redundant work faster. Never removes it. Crowded and VC-funded.
 | 
Coverage-based test skipping | 
Datadog Test Impact Analysis, Launchable, Develocity | 
Per-language agent maps tests to covered files, skips tests whose files didn't change | 
Blind to library deps, compiler flags, data files, external services (Datadog lists these itself). Default branch excluded by their own recommendation. Seven languages, one at a time.
 | 
Function-trace test skipping | 
YourBase (2021, faded) | 
Traced which functions each test called, Python and Ruby | 
A 2024 mutation study found 69% of gap mutants survived. Trust never arrived. Language-specific.
 | 
Declared-input task caching | 
Bazel, Buck2, Pants, Nx, Turborepo | 
Content-hash declared inputs per target or task, remote cache | 
Requires describing the build up front. Nx and Turbo are JS-world. Bazel is a migration, not an install.
 | 
Pipeline-as-code caching | 
Dagger, Earthly (build tool sunset) | 
Function-level caching if you rewrite CI in their model | 
Rewrite required. Earthly proved the switching cost kills adoption.
 | 
Syscall-inferred build graphs | 
Kotinos (research, 2020), tup, fabricate, Rattle | 
Infer dependencies by tracing what each step touched, no config parsing | 
Papers and old build tools. None sits under GitHub Actions as a drop-in runner.
 | 
Alternative Actions runners | 
Velnor (Rust, Rust projects only), act | 
Re-implement runner execution | 
Velnor proves a Rust runner is tractable but is narrow. act is approximate by design. Neither memoizes.
The Datadog limits list is the clearest specification of the gap. At the syscall level, a changed library, a changed compiler binary, and a changed fixture file are all just file reads. Coverage tools are blind to them by design. Trace at the OS level and the blind spots collapse into one category: inputs you observed.

## How it works
GitHub still owns the YAML, the scheduling and the UI. Quod owns execution. That split is the whole design: nothing the user touches changes, and everything that makes it fast lives in three new components.
The job flows top to bottom. GitHub assigns it, the orchestrator drops it into a warm microVM, the runner executes steps while the observer watches what each one reads, and the ledger decides whether a step needs to run at all. The local CLI shares that ledger, so a test run on a laptop can be the proof CI replays.
The observer is the novel part, and its trick is that it mostly isn't instrumentation. The workspace is served into the VM from a content-addressed store on the host. The host already knows every blob it handed over, by hash. That log is the input manifest, free, with no in-guest agent and no language support. In-guest eBPF only exists to attribute those reads to individual processes for finer granularity later.

## Step lifecycle
The lookup is two-level, because you can't know a step's inputs before it runs. Bazel knows inputs because you declared them. Quod only knows what previous runs read.
So the step key (command, env, action version, image) finds the manifests of past runs. Each manifest is the exact set of file hashes, directory listings and missing paths that run observed. If the current workspace matches one of them byte for byte, the past result is valid for this tree. This is how ccache's direct mode and tup have worked for years; nobody has applied it to a whole CI runner.
Missing paths matter as much as present ones. A step that checked for a local config file and found nothing must re-run the day that file appears. The observer records failed lookups too.
On a hit, outputs come back lazily. A node_modules tree on a hit is a promise, not a 400 MB copy; only the files later steps actually open get fetched from the CAS. That is what makes replay cheap even when outputs are huge.
The audit loop is what separates this from a cache. A configurable slice of hits re-executes in the background and the results are compared. Zero divergence over time is the evidence; a single divergence quarantines that step key and tells you exactly which step is non-deterministic.

## What counts as an input
An input is anything that could change the step's result. The table is the contract: if something isn't observable, the step gets marked tainted and never replays. Honesty here is what makes replay safe.

 | 
Input | 
How it's observed | 
Hashed as | 
Status
 | 
Files read | 
Host CAS filesystem serves the blob, logs the hash | 
BLAKE3 of content, never mtime | 
Clean
 | 
Directory listings | 
readdir served by host, logged | 
Sorted names + types | 
Clean
 | 
Missing paths | 
Failed lookup served by host, logged | 
Path + ENOENT | 
Clean
 | 
Command and args | 
From the workflow step | 
Canonical string | 
Clean
 | 
Environment | 
Passed at exec; volatile vars (run id, timestamps, tokens) normalized out | 
Sorted key=value, secrets salted | 
Clean
 | 
Tool binaries, shared libs | 
They are files; read like any other | 
Content hash via CAS | 
Clean
 | 
Container image | 
Pulled by digest | 
OCI digest | 
Clean
 | 
Action version | 
uses: resolved to commit SHA | 
SHA | 
Clean
 | 
Pinned registry fetches (npm, PyPI, crates.io, Docker Hub, GitHub releases) | 
Guest eBPF sees the socket; recording proxy hashes the response | 
URL + response hash | 
Clean when lockfile-pinned
 | 
Other network | 
eBPF sees the socket, response unknown | 
Host + port | 
Taint
 | 
Wall clock | 
vDSO bypasses syscalls; not reliably observable | 
n/a | 
Taint unless clock is frozen
 | 
Randomness, PIDs, thread interleaving | 
Not observable as inputs | 
n/a | 
Caught by audit, not prevented
Outputs are the mirror image: files created or modified under the workspace, stdout and stderr after GitHub's secret masking, exit code, anything written to the GitHub output, env and summary files, and artifact or cache uploads by reference.
Two honest gaps. Time: a process reading the clock through the vDSO never makes a syscall, so the observer can't see it. The mitigation is to run steps with a frozen, deterministic clock inside the VM (the kernel is ours), and to let audit catch what slips. Randomness and scheduling: not inputs you can hash. Audit is the only answer, and it's a good one, because a step whose output varies with scheduling is a flaky step, and surfacing that is a feature.
Side-effect steps (deploy, publish, notify, anything that writes to the outside world) are never replayed. They taint by network rule automatically, and a one-line annotation lets a user force it either way.

## Trust model
The product is the receipt, not the cache. Every predecessor that skipped work died or got fenced off the default branch for one reason: nobody could see why a skip was safe. Quod makes every skip auditable, and every repo earns replay one step at a time.
Receipts. Every replayed step prints a block at the top of its log: replayed from run N on commit X, these 2,314 input hashes matched, 2m14s saved, audited 17 times with 0 divergences. The job summary totals it. Click through and you see the full manifest.
Observation mode is the default. On install, nothing is skipped. Every step executes, the observer records, and the ledger computes what it would have replayed. The dashboard shows the savings you aren't taking yet. This is the 98% evidence, generated per repo, before anyone has to trust anything.
Trust is earned per step key, not per repo. A step key becomes replayable only after it has been proven deterministic: N consecutive executions with identical manifests produced identical outputs. The threshold is configurable, and the default is conservative. A tainted key never becomes replayable.
Audit never turns off. A configurable percentage of hits (default in the single digits) re-executes in the background. Results are compared byte for byte. One divergence quarantines the key, re-runs it on every commit until it proves itself again, and alerts. The trailing divergence rate is a public number in the dashboard.
Phase 0 as built (exp/trust; DECISIONS.md has the detail). Comparison is byte for byte after one normalization, go test's durations, and leaves out the step's scratch directories (what the toolchain profile calls scratch or a download cache and the env points at: Go's build cache, npm's, pip's and uv's caches; the carve-outs of sealed dependency trees, such as the module cache's `cache/`; a short list of volatile HOME paths), which are recorded but neither compared nor written by `--outputs`; receipts and banners name them. Failures, and output that says the step was killed or timed out, never prove themselves by default. Audits run in the foreground: a sampled hit executes instead of replaying (the step's own result is what the user gets), and observe mode compares every would-be hit. A divergence quarantines the key until a manifest re-proves itself, with a bar that rises with the key's recent divergences and is capped; it is reported in the banner, the audit log, `quod status` and `quod inspect`, and there is no alerting channel yet. Policy by branch and taint reporting are not built.
The default branch comes last. Policy by branch: feature branches can replay at the base threshold; the default branch requires a higher proof count and zero audit divergences over a trailing window. Teams that want a full run before every release get it, and still save the 98% everywhere else.
Taint is loud. Every tainted step shows why: reads the clock, hits an unpinned host, writes outside the workspace. That list is a determinism linter for free, and fixing items on it is how a team moves its hit rate up.
The argument to a skeptical staff engineer: this is Bazel's correctness model (content-addressed actions, hermetic execution, remote cache) with the input declaration replaced by observation, and with an audit loop Bazel doesn't have.

## Where the 98% comes from, and where it won't
98% is reachable on the aggregate, and on identical-tree re-runs it's 100%. On a single code commit through a monolithic compile step, the honest ceiling is closer to 90%. The spec should say that out loud, and the benchmark should measure both.
Three mechanisms stack, and each one needs the one before it:
- 
Skip (memoization). The step's manifest matches a proven run. Cost drops to replay latency. This is the whole win for setup, install, checkout and anything that didn't read the changed files.
- 
Split (granularity). A step that reads everything can't be skipped, but its processes can. go test ./... and cargo test already run one process per package, so they split for free. Jest, Vitest, pytest and RSpec run everything in one process; a splitter fans them out to one process per test file, in parallel, and each file memoizes on its own.
- 
Warm (incremental state). Compilers and bundlers read the changed file and can't be skipped at any granularity. What helps is starting them warm: the previous run's .tsbuildinfo, target/, or bundler cache is materialized as starting state, so the tool's own incremental mode does the work. Quod doesn't make this correct; the tool does. Audit catches the tool when it lies.
A worked model, not a measurement. A plausible Node monorepo pipeline, baseline 14 minutes, on a commit that touches three source files:

 | 
Step | 
Baseline | 
Skip only | 
Skip + split + warm | 
Why
 | 
Checkout | 
20s | 
1s | 
1s | 
Runner serves the tree from CAS; no git fetch
 | 
Setup Node | 
15s | 
1s | 
1s | 
Pinned release, hit
 | 
npm ci | 
90s | 
1s | 
1s | 
Lockfile unchanged, lazy node_modules
 | 
Lint | 
60s | 
60s | 
5s | 
Single process reads all files; per-file split re-lints 3
 | 
Typecheck | 
120s | 
120s | 
15s | 
Reads everything; warm tsbuildinfo makes it incremental
 | 
Build | 
180s | 
180s | 
30s | 
Same; warm bundler cache
 | 
Unit tests | 
300s | 
300s | 
15s | 
Per-file split; ~12 of 400 files read the changed modules
 | 
E2E | 
60s | 
60s | 
60s | 
Browser, timing, live server: tainted until recorded
 | 
Total | 
845s | 
723s (−14%) | 
128s (−85%) | 

Two things get this pipeline from 85% to 98%. First, making E2E deterministic (recorded network, frozen clock) so it splits and memoizes like unit tests. Second, the two compile steps, which no memoizer can remove on a code change; they fall only as far as their incremental modes allow. On Go, Python, Ruby, or a well-split monorepo, those steps are small or absent and the per-commit number clears 95%.
Where 98% is easy: docs-only commits, config commits, dependency bumps that touch one lockfile, flaky-test retries, re-pushes after rebase with no content change, merge-queue validation of a tree whose files were all tested already, and coding agents iterating on a near-identical tree 30 times a day. For a lot of orgs, those are most of the minutes. The benchmark will tell us what fraction.

## Build vs reuse
Five components are new. Everything else is a mature open source project wired in. The rule: build only what encodes the idea (observe, key, replay, prove), rent everything that is commodity.

 | 
Component | 
Decision | 
Use or reference | 
Why
 | 
Actions runner (job protocol, step executor, expression engine, contexts) | 
Build in Rust | 
actions/runner (C#, the spec), Velnor and act as references | 
You must own execution to observe and skip steps. The official runner has no per-step hooks. Large, well-specified, tedious: exactly the job agents make tractable now.
 | 
Observer (host CAS filesystem + access log) | 
Build | 
virtiofsd or the fuser crate for transport | 
The novel piece. No prior art serves a CI workspace from a CAS and treats the serve log as the dependency manifest.
 | 
Guest attribution (which process read what, which opened a socket) | 
Build, thin | 
aya (Rust eBPF) | 
Needed for process-level granularity in Phase 3. Small program, well-trodden tracepoints.
 | 
Step key canonicalization + taint classifier | 
Build | 
ccache direct mode, tup, Rattle (design references) | 
Encodes the correctness argument. Must be ours.
 | 
Replay engine (lazy materialization, log replay, outputs, receipts) | 
Build | 
none comparable | 
Novel. The receipt format is the product surface.
 | 
Audit sampler + per-key trust policy | 
Build | 
none | 
The trust model. Small code, high leverage.
 | 
Test splitters (Jest, Vitest, pytest, RSpec) | 
Build, per framework | 
each framework's CLI | 
Adapters that run one process per file. Go and Cargo need none.
 | 
Orchestrator (claim jobs, pool VMs, stream logs) | 
Build, thin | 
GitHub JIT runner API; RunsOn and Ubicloud as references | 
Commodity shape, but ours has to hand the observer its mount and the runner its ledger.
 | 
MicroVM + snapshots | 
Reuse | 
Firecracker | 
Sub-second restore from a warm snapshot; battle-tested at AWS.
 | 
Guest kernel | 
Reuse | 
minimal Linux, our config | 
Ours to configure (frozen clock, no vDSO fast path if needed), not to write.
 | 
Content-addressed blob store | 
Reuse | 
Bazel Remote Execution API via NativeLink or bazel-remote | 
The ActionCache + CAS contract is exactly the memo table. Years of hardening for free.
 | 
Object storage | 
Reuse | 
S3, R2, MinIO | 
Behind the CAS. Self-hosters pick.
 | 
Receipt and manifest index | 
Reuse | 
Postgres (SQLite for local mode) | 
Boring on purpose.
 | 
Hashing | 
Reuse | 
BLAKE3 | 
Fast enough to hash every blob served without being the bottleneck.
 | 
JS action runtime | 
Reuse | 
Node.js | 
Actions ship as JS; we run them as GitHub does.
 | 
Docker actions and container jobs | 
Reuse | 
Docker or BuildKit inside the VM | 
Layer caching is a solved problem; borrow it.
 | 
GitHub integration | 
Reuse | 
GitHub App, JIT runner registration, Checks API, job summary | 
GitHub keeps YAML, scheduling and UI. We never fork the interface.
 | 
Local CLI | 
Build | 
same runner and observer in a local Firecracker (Linux) or a reduced-hermeticity container (macOS) | 
One codebase. macOS loses some guarantees and says so.
What we deliberately don't build: a YAML dialect, a UI that replaces GitHub's, a hosted-only service, or a new build system. The moment this needs a migration, it's Earthly.

## Phases
Each phase ends at a gate that is a measurement, not a feature list. Phase 1 ships publicly even though it skips nothing: a free, accurate report of how much of your CI is redundant is useful on its own, and it is the proof that funds trust in Phase 2.
The spike is deliberately narrow. One Go repo, one run: step, no GitHub integration, no UI. It answers the only question that could kill the project early: can a host-side CAS filesystem log serve as a complete input manifest with acceptable overhead? If yes, everything after it is engineering. If no, we learned that in six weeks.
Phase 1 is the big build: the Rust runner. It is also the part most suited to agent-assisted development, because the official runner is open source and the behavior is fully specified by it. The target is bit-for-bit log compatibility on a corpus of real public workflows before any memoization is attempted.
Durations are estimates for a small team working with coding agents. They assume the runner protocol work goes as Velnor's existence suggests it can.

## Success metrics
The benchmark is the product's spine. It runs in observation mode against real history, so it can be published before anyone trusts replay, and it is what every gate in the roadmap reads from.
The corpus. 20 public repos with real GitHub Actions workflows, spread across Go, Rust, Node, Python and Ruby, small to monorepo. For each, the last 200 commits on the default branch plus their PR branches. Every workflow is run under Quod in observation mode, in commit order, so the ledger fills the way it would in production.

 | 
Metric | 
Definition | 
Target | 
Kills the project if
 | 
Per-commit saving | 
Wall-clock under replay vs baseline, median across commits | 
90% by Phase 3, 98% stretch | 
Below 70% after splitters
 | 
Aggregate saving | 
Total step-seconds replayed over the corpus history | 
98% | 
Below 90%
 | 
False-hit rate | 
Audit re-executions whose output differs from the replayed result | 
0 | 
Any nonzero that taint rules can't explain
 | 
Taint share | 
Step-seconds in tainted steps that can never replay | 
Under 10% after Phase 2 | 
Over 30%: the model doesn't fit real CI
 | 
Cold overhead | 
Observed execution time vs plain runner, same VM | 
Under 10% | 
Over 25%
 | 
Replay latency | 
Time from step start to step end on a hit, p50 | 
Under 500 ms | 
Over 5 s, which eats the win on small steps
 | 
Runner fidelity | 
Workflows whose logs and results match the official runner | 
100% on the corpus | 
Any silent divergence
Two numbers are published, not one. Per-commit median is the honest developer-experience number. Aggregate is the honest billing number. The 98% hypothesis is stated against aggregate; if per-commit median lands at 90% on monolithic compile pipelines, the hypothesis as written is still true and the spec says why.
Every number comes with the receipts that produced it. The benchmark results are a public page: repo, commit, step, hit or miss, audited or not. Anyone can rerun it.

## Risks and open problems
The technical risks are known and bounded. The adoption risk is the one that killed the predecessors, and the trust model is the answer to it.

 | 
Risk | 
Why it's real | 
Mitigation | 
Residual
 | 
Clock reads are invisible | 
vDSO serves time without a syscall; a test that branches on the date can replay wrong | 
Frozen deterministic clock in the guest kernel; audit sampling catches what leaks | 
Low: a step that depends on the real date is already flaky
 | 
Network non-determinism | 
A live API call can return anything; the observer sees the socket, not the semantics | 
Pinned registries go through a recording proxy and hash; everything else taints | 
Medium: taint share on API-heavy integration suites could be high
 | 
Hidden inputs through the guest | 
/proc, /sys, hardware counters, CPU feature flags, locale | 
Stable kernel config, pinned image, these surfaces hashed into the step key as environment | 
Low
 | 
Observer overhead | 
Every file served through virtiofs plus hashing; a read-heavy compile could slow | 
BLAKE3 on the host, blob hashes cached by CAS identity so repeat reads are free, snapshot warm pools | 
Medium until the spike measures it; this is what Phase 0 exists for
 | 
GitHub protocol churn | 
The runner protocol is undocumented and changes with releases | 
Track actions/runner releases; a compatibility corpus in CI; Velnor as a second reference | 
Medium, ongoing cost
 | 
Secrets in the ledger | 
Env values and outputs may contain secrets | 
Secrets salted into the key, never stored; logs stored post-masking; blobs encrypted at rest; org-scoped stores | 
Low with discipline, needs a review
 | 
Incremental tool bugs | 
Warm state trusts tsc, cargo, bundlers to be correct incrementally | 
Audit sampling on warm steps at a higher rate; one-click cold rebuild | 
Low, and surfacing it is a feature
 | 
Granularity ceiling | 
Compile steps can't be skipped on code changes at any granularity | 
Warm state; honest 90% per-commit framing on monolithic pipelines | 
Accepted
 | 
Test splitting changes behavior | 
Running one process per test file can hide order-dependent tests, or expose them | 
Observation mode compares split vs unsplit results before enabling | 
Low, and exposing them is a feature
 | 
Nobody flips replay on | 
Observation mode is comfortable; the default branch stays conservative forever | 
Receipts, public divergence rate, per-step earned trust, savings dashboard that nags | 
High. This is the real bet.
 | 
Competitors copy it | 
Blacksmith, Depot and Namespace have the infrastructure and the customers | 
Open source first; the ledger format and receipts become the standard they adopt | 
Accepted. If they copy it, CI got better.
The open research problem is network. A general answer to "would this request return the same thing" doesn't exist. The pragmatic answer (pinned content-addressed sources are clean, everything else taints) will cover package installs and most build steps. Integration tests against live services stay un-replayable until teams record them, and the taint report tells them exactly which ones.

## Out of scope for v1
- 
macOS and Windows runners. Linux x86_64 and arm64 only. The local CLI runs on macOS with reduced hermeticity and says so.
- 
Any CI system other than GitHub Actions. GitLab and Buildkite are natural later targets because the ledger and observer are CI-agnostic; the runner is not.
- 
A hosted service. v1 is self-hosted and open source. Hosting is a business decision for after the benchmark is public.
- 
A new workflow syntax, a UI that replaces GitHub's, or a build system. Any of these turns a drop-in into a migration.
- 
GPU jobs, nested virtualization, and jobs that need privileged host access.
- 
Cross-organization ledger sharing. One org, one ledger. Public-repo proof sharing is an interesting later idea with real security questions.
- 
Predictive or ML-based test selection. Quod skips only what it has proven. Guessing is a different product.

## Decisions needed before the spike
- 
Name: Quod (pronounced "kwod", from Q.E.D.), handle `quodci`, site quodci.com. Decided 2026-10-31, the rename recorded in DECISIONS.md.
- 
Observer transport: virtiofsd with a custom backend, or a FUSE server via the fuser crate exposed over virtio. Virtiofs is faster; FUSE is simpler to prototype. The spike should try virtiofs first and fall back.
- 
CAS backend for the spike: NativeLink or bazel-remote. Either speaks REAPI; pick the one with the cleaner Rust client.
- 
Spike repo: one public Go project with a plain Actions workflow and a test suite over a minute long.
- 
License. Apache 2.0 keeps it adoptable by the companies most likely to contribute. AGPL protects against a hosted clone. Decide before the first commit.
- 
Whether to fork Velnor for the runner protocol layer or treat it as a reference only. Depends on its license and code shape.
- 
Threshold defaults: how many identical proofs before a step key is replayable, and the audit percentage. Conservative first; the benchmark tunes them.
