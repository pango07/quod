# Through the wall: convex-helpers from 27–30 s to ≤ 16 s, and misses at ≤ 1.05x

2026-11-05. For a fresh implementer. Builds on `whole-job-replay.md` and
`beyond-the-floor.md`; code at `exp/supported-v1` (32b630c) unless noted.
Facts below are from the jobs API (run 41964655209), the drop-in bench logs
(`convex-helpers/logs/plain-2.log`, `check-2.log`) and the repo.

## 0. What the wall is made of

The floor of a convex job is 11–13 s (queue, Set up job, checkout,
`setup-node` 4 s, post). A hit today is 27–30 s. The difference:

| in the hit job | s | root cause |
|---|---:|---|
| `stream.property.test.ts` | up to 61 | near-timeout rule trips: **the file is 98 s plain, 115 s under recording**; its slowest test 16.6 s of 30 s *under hooks*, ~10 s plain. The margin is measured on inflated durations. |
| `npm run test:release` (`node --test scripts/*.test.mjs`) | 8 | spawns `git` and `sh`: node hooks cannot see their reads |
| `openApiSpec.test.ts` (`npx redocly lint`) + a vitest start | 5.5 | redocly phones home (telemetry/update check): "network, always runs" |
| action main + post | 2–6 + 1–9 | Node bundle, download, ledger restore; 3.5 MB ledger save, HTTP 500s force a full re-witness |
| `npm run build` (tsc + esbuild) | 2 | esbuild is native |
| misses | ×1.14–1.5 | recording overhead on the critical path |

Note that `stream.property.test.ts` alone is ~60% of the plain test step
(98 of ~109 s): on this repo, replaying that one file *is* the product.

## 1. Syscall-level proofs for native tools: the FUSE view, in place

**Choice: Quod's own observer (`crates/quod-fs`, bubblewrap sandbox in
`crates/quod-sandbox`), serving the workspace at its real path.** Rejected:

- *ptrace*: a stop per syscall; 2–10x on `git`/`esbuild`-class processes,
  and threads multiply it. Too slow for a critical path.
- *seccomp user-notify*: path arguments are pointers into the tracee; a
  second thread can swap them after the check (the kernel's own docs say
  not to use it for policy on pointer arguments). An under-record risk, so
  not a proof.
- *fanotify*: no `stat`, `readdir`, `readlink` or negative lookups, which
  are exactly what `git`, `node` resolution and `esbuild` decide on.
- *eBPF*: sound in principle, needs root (available: `sudo` is
  passwordless on hosted runners), but path resolution (`dfd`-relative
  opens, 30 tracepoints) and ring-buffer drops under load mean weeks of
  work plus a fail-closed drop detector. Keep as a later transport.

The observer already exists and is proven: it logs every lookup, getattr,
readdir, open, read and readlink (`quod-fs/src/lib.rs:1-7`, "over-record,
never under-record"), the kernel overlay captures writes as the step's
outputs (`run.rs:92-93`, `:132`), negative lookups are inputs, and the
engine replayed a 17 s `go test` with 30,000 inputs and zero false hits
(README "Status"). Overhead measured: +20% on a syscall-heavy `go test`
(66–70 vs 55–59 s, DECISIONS 2026-11-02 exp/observer-speed), less on
anything that computes. The runner has fuse3 and bubblewrap, and
`ci-gate.sh setup` already turns on FUSE-over-io_uring there (docs/ci.md:111-119).

**The one change:** the sandbox binds the workspace at `/workspace`
(`quod-sandbox/src/lib.rs:112`, passwd at `:135`). Add `workspace_at: PathBuf`
so the observed tree is bound at the checkout's real path
(`/home/runner/work/<repo>/<repo>`); then paths in outputs, logs and
sourcemaps are identical to plain, and `cwd` is preserved. Everything else
(`--host-root`, `run.rs:102-107`; sealed read-only binds of system prefixes,
`run.rs:1078`; the upper moved into the CAS at step end, `run.rs:92`) is used as is.

**A unit.** The shim (`quod-action/hook.cjs`) already sees every `node`
process. A **unit** is an npm script invocation the shim recognizes
(`npm run <script>`, `npx <bin>`, `node <script>` at a step's top level:
`npm run build`, `npm run test:release`) that is not vitest. For a unit the
hook execs `quod run --host-root --workspace-at "$PWD" --mode auto -- <argv>`
inside the real node: on a trusted record the engine restores the outputs
(overlay upper from the CAS) into the workspace, prints the recorded stdout
and stderr and exits with the recorded code; otherwise it runs the command
under the observer and records. vitest keeps its own per-file recorder
(module-graph attribution needs the process); it is *also* run under the
observer for section 2.

**Keying the system.** Reads outside the workspace go through sealed
read-only binds and are keyed by path and content like any input
(`run.rs:1078`); the host class (kernel, CPU, sysctls, seccomp mode,
`quod-key/src/lib.rs:165`) is in the key already. Add the runner image
identity (`ImageOS`, `ImageVersion` from the environment, and
`/imagegeneration/imagedata.json`) as a shared input of every unit, so an
image roll re-proves everything once. node_modules is keyed by the blobs the
unit read (content-addressed; no package-closure approximation).

**Network.** The sandbox has no network unless `--allow-net`
(README). A unit that opens a non-loopback socket fails closed: the hook
re-runs it plain with network, marks it "network: always runs", and it is
never recorded (section 4 refines this for tests).

**`test:release`:** its `git` runs in temp repos under a private `/tmp`
(sandbox default); nothing escapes, so the unit has no outputs and replays
to its stdout and exit code. **`npm run build`:** `tsc` + `esbuild` outputs
(`dist/`) are the upper, restored on hit (~0.2 s for convex's dist).

Expected on a convex hit: −8 s (`test:release`), −2 s (build).

## 2. Recording overhead ≤ 1.05x

**The fact that reorders the suspects:** `stream.property.test.ts` is
CPU-bound (fast-check, no I/O) and still runs 17% slower under recording,
on a first-witness run where coverage is *not* taken ("71 files without
function-level data: a first witness (coverage not taken)"). So the cost is
not coverage and not fs hooks; it is what touches hot code paths:

1. `preload.cjs` patching `Function.prototype.toString` (every call site
   that stringifies functions, and possibly a V8 deopt of callers);
2. the `process.env` Proxy (`record.mjs:576`), hot if a library reads
   `process.env.NODE_ENV` in a loop (zod, fast-check and convex-test do);
3. `module.registerHooks` (`record.mjs:629`, `:687`) on every import;
4. the stack capture per attributed read (`new Error().stack`).

**Profile first:** run that one file with `--cpu-prof` in the worker under
four settings (plain; preload only; env Proxy only; all hooks) using
`scripts/bench-vitest/breakdown.sh`'s harness. One hour of work; it decides
which of the two fixes below carries the weight.

**Fix A, structural: the second witness runs plain.** A record needs two
agreeing runs. Today both record. The second run only has to confirm the
*results* for the *same key*: when the first witness's key still matches at
the time of the second run (same inputs by construction), run the second
witness as **plain vitest with `--report` only**, no setup file, no preload,
no hooks (the critical-path mode of `vitest-shadow.md`, `quod vitest
--no-record`). Agreement of per-test states promotes the record. Steady
state, half of all recording runs become plain, and every promoted record
carries **plain durations**, which section 3 needs. Expected: misses from
1.14–1.5x to ≈1.07–1.25x on the first witness and 1.0x on the second.

**Fix B, mechanical:** replace the `process.env` Proxy by a getter-less
snapshot plus `envp:` presence keys (the recorder already keys presence,
not value, for most names); make `toString` capture lazy (install the patch
only when a file's module graph includes a known offender list, else record
"toString unknown" and take no function-level data); capture a stack only
for env and `package.json` reads. Expected: the remaining first-witness
overhead to ≤ 1.05x. With vitest under the observer (section 1), the fs
hooks can go entirely: per-worker attribution comes from the observer's pid
(forks pool) or stays graph-based (threads pool), and the observer's own
cost is shared with the plain file I/O it replaces.

Acceptance: bench-dropin all-run commits, convex and mcp-sdk,
`quod_ms/plain_ms ≤ 1.05` for the second witness and ≤ 1.10 for the first.

## 3. Near-timeout: measure the margin on plain time

Opus's proposal, re-timing near-timeout files plain on misses (~10 s per
miss), is right in intent and unnecessary as a separate run once the second
witness is plain (Fix A): that run *is* the plain timing, and it had to run
anyway. Rule, restated: a record is trusted when both witnesses agree and
the **plain** witness saw every test at ≤ 0.5x its effective timeout
(`stream.property`'s slowest: ~10 s of 30 s, 0.33). A hooked duration never
decides. A file whose plain witness is above the margin stays untrusted and
re-tries on each miss (it runs anyway). Expected on a convex hit: the 61 s
case disappears; the test step on a full hit is the render only.

## 4. Network, kept never-wrong

**Pinning recorded responses is not sound** and the answer is no: replaying
a response means asserting the server would answer the same today, which
Quod cannot prove without asking. Two sound options:

1. **Fail closed** (today): any non-loopback socket makes the file or unit
   unrecordable. Loopback servers a test starts itself stay recordable
   (their reads are in-process).
2. **The egress-denied witness.** Run the second witness with no network
   (bubblewrap `--unshare-net`, the sandbox default). If its per-test
   results equal the first witness's, the recorded result is reproducible
   from local inputs alone, which is exactly what a replay asserts; the
   network was not an input *to this outcome*. If anything differs or
   fails (a test asserting reachability, a fetch that drives the result),
   the file is unrecordable, as today. This is the same standard every
   replay already meets: two agreeing witnesses from the recorded inputs.
   redocly's telemetry fails silently and the lint result is the same, so
   `openApiSpec.test.ts` becomes replayable; a test that needs
   `registry.npmjs.org` never does.

Expected on a convex hit: −5.5 s (the file and the vitest start it forced).

## 5. Ledger and storage

The 3.5 MB ledger is uploaded whole (6–9 s) and a failed upload drops the
record, forcing a full re-witness. Decisions:

1. **Content-addressed records.** Each file's record (keys, states,
   witness, render) is a blob by hash; the ledger is a manifest of record
   ids. A save uploads only new blobs (a typical miss changes a few dozen
   records: tens of KB), 8 PUTs in parallel, exponential retry. The server's
   HTTP 500 on upload is a bug to fix (`crates/quod-server/src/blobs.rs`),
   and until then the client must **always** save the manifest and blobs to
   the Actions cache when the Cloud save fails (today the cache is the
   fallback for *restore* only, `main.js:340-342`).
2. **Verdict by one GET.** With git-OID keys (`beyond-the-floor.md` idea 2),
   the manifest is keyed by `(repo, tree OID, env fingerprint)`; the action
   fetches a few KB and the records it needs, lazily. No whole-ledger
   restore ("194 files, 3485 KB").
3. **A hit saves nothing**, so post is a stat and exit. Saving is
   synchronous in post only on recording jobs, where it is now small.

Expected: post from 1–9 s to ≤ 0.5 s on hits and ≤ 2 s on misses; no
re-witness storms. Storage cost: S3 PUTs and GBs at this scale are cents.

## 6. Sequence to ≤ 16 s, with the number to watch after each step

Order chosen for the largest measured effect per week, all on the public
bench-job workflow (jobs API, hit and miss commits, wall, runner-seconds and
billed minutes), convex-helpers window as now.

| step | work | watch | expected |
|---|---|---|---|
| S1 plain second witness + plain-time margin (sections 2A, 3) | `vitest.rs` witness path (`:489-507`, `:932-1152`), `driver.mjs` durations, `Rec.near_timeout` | hit test step; miss ratio | `stream.property` replays: hit test step → render only; misses 1.14–1.5x → ≤ 1.25x |
| S2 egress-denied second witness (section 4) | sandbox `--unshare-net` for the witness run; `hook.cjs` | files "always run: network" | `openApiSpec` replays: −5.5 s on hits |
| S3 content-addressed ledger, cache fallback on failure, retry (section 5.1) | `quod-ledger`, `quod-cli` cloud save, `blobs.rs`, `post.js` | post step; re-witness count | post ≤ 2 s; no forced re-witness |
| S4 near-zero action + git-OID verdict (`beyond-the-floor.md` ideas 1–2, section 5.2) | `quod-action` main/post → stub + static quod in `dist/`; OID keys; manifest GET | `quod` step, `Post quod` | main ≤ 1 s, post ≤ 0.5 s on hits: −4 to −8 s |
| S5 units under the in-place FUSE view (section 1) | `quod-sandbox` `workspace_at`; `hook.cjs` unit detection; `run.rs` restore path; image identity key | `build`, `test` steps on hits | −8 s (`test:release`), −2 s (build) |
| S6 hooks → observer, env/toString fixes (section 2B) | `record.mjs`, `preload.cjs`, `setup.mjs` | miss ratio | first witness ≤ 1.10x, second 1.0x |

After S5 a convex hit is: floor 11–13 s + action ≤ 1.5 s + three shim
answers ≤ 1 s + render ≤ 0.1 s = **13–16 s wall, ~13 runner-seconds
(0.08x), 1 billed minute (0.36x)**. Window cost at the observed 6/12 hits
with misses at 1.05x: 0.57x runner-seconds; ≤ 0.5x needs S6 to land
(misses ≈ 1.0x) or a hit rate ≥ 56%. Say so in the reports rather than
tuning the window.

## 7. Risks

- FUSE-over-io_uring needs the sysctl on; classic `/dev/fuse` is the
  fallback (docs/ci.md:117) at higher overhead. Units are off the hit path
  anyway; the overhead lands on misses only.
- Property tests (fast-check) draw random cases per run; two agreeing
  witnesses is the same standard as for any other test, and a file that
  disagrees is quarantined as today.
- The in-place bind changes the sandbox's assumptions about `/workspace`
  (passwd home at `lib.rs:135`, `.quod` state dir): audit every literal.
- `--unshare-net` for the second witness changes DNS behaviour inside the
  file (ENOTFOUND instead of a hang); a test that *passes* only because of
  the fast failure still passes plain, and a test that fails is
  unrecordable: the asymmetry is safe.

## Addendum (2026-11-05, after S1, run 41968463523): timing-dependent tests, devDependency bumps, the honest floor

### A1. Tests whose plain outcome is timing-dependent

`stream.property.test.ts`'s slowest test runs 11.3–25.6 s of a 30 s timeout
in plain vitest (median 0.69, one run 0.91). Plain itself cannot reproduce
its own result; "wrong against plain" is then a comparison with a random
variable. The options:

- **(a) keep 0.5; the file always runs.** Convex hit job ≥ 60–120 s;
  the product does not exist on this repo. No risk.
- **(b) the timeout is environment.** With every input unchanged the test's
  computation is identical; only the machine differs. Two plain witnesses
  proved the code passes; replay the pass and print
  `timing-sensitive: passed at 0.69 of its timeout when recorded`.
  Convex: the file replays on every hit (test step → render). Risk, the
  mistral case (0.46, later timed out under load): plain would have been red
  that day, Quod green with a label. Nothing about the code changed, so the
  green is the answer the code deserves; what is hidden is the runner's load.
- **(c) re-run plain only when inputs change.** That is (b): any changed
  input already runs the file.
- **(d) (b) plus a fixed audit.** Timing-sensitive files are replayed, and one
  in N hits (N = 5) runs the file plain as an audit (the existing audit
  path, `--audits`, today only when workers are idle); a timeout in the
  audit quarantines the file until it re-proves. Convex: 4 of 5 hits replay,
  1 of 5 pays 98 s. Window runner-seconds at 6/12 hits ≈ 0.55x against
  0.49x for (b).

**Recommendation: (b), with (d)'s audit as a per-repo switch, off by
default**, and the label in the report and the check summary. The founder
decides one sentence: *is Quod's contract "the result these inputs
deserve, proven twice" or "what plain would have printed on this run"?*
The second is not computable (0.38–0.91 on identical inputs), and choosing
it means (a). The timing-sensitive label keeps the first honest: the user
sees which green results rest on a run that was once near its timeout.

### A2. A devDependency bump must not invalidate every file

Two shared inputs do it today: `p:node_modules/chalk` (in vitest's package
closure, docs/vitest.md:56-60) and the `devDependencies` field of
`package.json` (read by tool code, keyed as a field). The sound line:

1. **Split vitest's closure into main-process and worker.** Reporters, the
   CLI and their dependencies (chalk, commander, cac, tinyrainbow) load only
   in vitest's main process and shape output, not results. A package in the
   closure is a shared input only if a worker loads it before the setup
   file: compute that set statically from vitest's worker entries
   (`node_modules/vitest/dist/workers/*.js`, their import graph, plus the
   environment package: jsdom) and key those packages for every file; the
   rest are keyed only through the render (the printed text they produced).
   A chalk bump re-renders nothing wrong: the recorded output is the old
   output, and vitest's own version is keyed.
2. **A manifest entry is an input of a file iff that package is in the
   file's graph or in the worker set.** Key `dependencies`/`devDependencies`
   per entry, joined with the module graph (`p:` keys), not as a whole field.
   A bump of a package no file imports changes no key.
3. The lockfile stays out (docs/vitest.md:55-63); the install still runs
   (`npm ci`, 12–18 s), because the tree really changed. Later (S7, weeks):
   materialize the new tree from the lockfile out of the CAS (every blob of
   the previous tree is in the store) in ~2 s instead of `npm ci`.

Convex: the two renovate commits go from full re-runs to install-only
misses; the window's miss cost drops by ~2 × 100 s of test time.

### A3. Is ≤ 16 s reachable on convex?

Per step on a hosted runner, hit job, after S2–S5 (measured ranges from the
jobs API, not targets): queue 2–3 (wall only), Set up job 1–3, checkout 1–2,
`setup-node` 4–5 (Node 24.18 download; a customer step), action main
0.5–1, `npm ci` answered 0.3, build answered 0.3, test (npm start + render)
0.5, `test:release` answered 0.3, post 0.3, runner bookkeeping between
steps 2–4. **Sum: 13–19 s wall from job start, 1 billed minute.** ≤ 16 s
happens on the low side of GitHub's own variance (a 4 s `setup-node`, 2 s
of bookkeeping), not by design. The honest target is **floor + 2 s, with
the floor at 12–16 s on this image**; state ≤ 20 s as the pass mark and
report the floor beside it. Runner-seconds on hits ≈ 0.08x; the window
needs misses at ≈ 1.0x (S6) or ≥ 56% hits to reach ≤ 0.5x.
