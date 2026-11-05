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

## Addendum B (2026-11-05, after S1–S4 on exp/whole-job-impl b13d8a0..c3c2fd1, run 41979901669): S5 without the import, the npm environment rule, the last 15 s

Measured hit job 27 s: `test` 11 (test:release's `node --test` ~10 +
render ~1), build 2, action main 2, setup-node 5, boot 1, post 1.
runner-seconds 0.51x with timing-sensitive replay, wrong 0. The floor is
~12 s. Three answers below; each is a decision, with the number it moves.

### B1. S5: the wall is verification, not observation

The 9.7 s the FUSE engine spends "importing" convex's 29,225 inputs is not
a mounting cost. It is the hit path's input check: on a fresh runner there
is no stat cache (every mtime is new after checkout and `npm ci`), so
every input is re-hashed on every run. An in-place `quod-fs` backend
(option a) removes the import from *recording*; on a hit the engine still
has to decide whether 29,225 recorded hashes still hold, and that is the
same seconds. A week plus `sudo` for `/etc/fuse.conf` buys nothing on the
number that matters. **Decision: no FUSE for units on hosted runners.** The
engine stays for self-hosted and dev where a warm stat cache exists.

**Decision: split a unit's record into the trace (what was read) and the
keyer (how a read is keyed), and give units vitest's keying.** The trace
is a stream of path events; the keyer turns it into the keys `quod vitest`
already hashes in ≤ 0.3 s on convex:

| a path the unit read | key | hit-path cost |
|---|---|---|
| under the checkout, in git's index and unmodified | `f:` keyed by the index OID (`git ls-files -s`, racy-index rule for the stat check) | one `git` call, 0.1 s |
| under the checkout, not in the index or modified (an earlier unit's output: `dist/`) | `f:` content hash | the few files that are |
| under a `node_modules` package root | `p:<pkg>` (package.json name and version, plus the lockfile's `integrity` for that package); files with no package root `f:` | package.json reads for the packages touched (hundreds) |
| outside the checkout (`/usr/bin/git`, `/bin/sh`, node, `/etc/*`, `/opt/hostedtoolcache/...`) | `f:` content hash | a few hundred small files; plus the image identity (`ImageOS`, `ImageVersion`, `/imagegeneration/imagedata.json`) as a shared input |
| a name that did not exist, a directory listed | `e:`, `d:` | stat, readdir |

**Record transport: strace, as the implementer proposed (b), with these
changes.** Recording cost is paid once per unit key (first witness only;
the second witness runs plain, S1's structure, and agreement of stdout,
stderr, exit code and the output tree promotes), so it is amortized, and
the spec's "too slow for the critical path" no longer applies to it.

1. **`--seccomp-bpf`.** The probe (ci-bench probe/runner,
   `probe-strace.yml:31-33`) ran `strace -f -qq -y -e trace=%file,...`
   without it, so every syscall of every thread (futex, epoll_wait, read,
   write: node's hot loop) stopped the tracee twice. That is the 2.7x. With
   `-f --seccomp-bpf`, untraced syscalls run at full speed and only the
   traced set stops. Expect 1.3–1.6x on `test:release`; **measure it on
   probe/runner first (one hour), before writing the parser.**
2. Trace set: `%file,%process,getdents64,%network,io_uring_setup,
   io_uring_enter,io_uring_register`, with `-yy --decode-fds=path -s 4096`
   so `*at` calls and `getdents64` carry the directory's path. The parser
   tracks cwd per pid (`chdir`, `fchdir`, inherited across `clone`/`fork`,
   kept across `execve`) to absolutize relative paths. `execve` keys the
   binary (`f:`); a `#!` interpreter shows as the kernel's second open.
3. **io_uring.** Quod sets `UV_USE_IO_URING=0` for the traced run only (a
   libuv backend switch, no result changes; Quod's own variable, not keyed),
   and any `io_uring_setup` in the trace makes the unit unobservable: fails
   closed. The 920 setups the probe saw go to zero with the variable set.
4. Both witnesses run in bubblewrap with `--unshare-net` (loopback only),
   `--unshare-pid`, a private `/tmp`, the checkout bound at its real path
   (unprivileged bwrap works on the runner, probe-runner.yml:23). A
   `connect()` to a non-loopback address marks the unit "network: always
   runs". `test:release`'s `git` runs in temp repos under the private
   `/tmp`: scratch, no outputs.
5. **Outputs** are writes under the checkout seen in the trace (`openat`
   with `O_WRONLY|O_RDWR|O_CREAT|O_TRUNC`, `rename`, `unlink`, `mkdir`,
   `symlink`, `truncate`): their content is snapshotted into the CAS after
   the run and restored on a hit. `test:release`: none; `build`: `dist/`.
6. The ptrace pointer race (a second thread swapping a path buffer between
   strace's read and the kernel's) is a policy concern, not a recording one:
   test code is not adversarial, and the JS hooks the vitest recorder relies
   on are far easier to bypass. Same standard; say so in the code.

**Rejected, with the reason that decides it.** *eBPF now*: the sound form
reads the kernel's resolved `struct path` after lookup (`security_file_open`,
`vfs_getattr`, `security_inode_permission`), which needs either BPF LSM
(not in Ubuntu's `lsm=` boot list on the image) or kprobes + BTF with a
per-image struct-layout risk, plus an io-wq worker attribution problem;
3–5 days for a cost that lands only on first witnesses. The trace/keyer
boundary makes it a drop-in transport later. *LD_PRELOAD for node only*:
libc's own calls (`fopen`, `opendir`, `scandir`, `dlopen`, nss reading
`/etc/passwd`) bypass the PLT, static Go binaries (esbuild) bypass
everything, and the only detector (a seccomp filter on the instruction
pointer) would then kill libc's internal opens. *seccomp-notify*,
*fanotify*: as in section 1. *A coarser grain* (key `test:release` on
`scripts/*.mjs` + package.json + node + git statically): unknown reads;
the trace is cheap enough once per key.

**Expected.** `test:release` answered in ≤ 0.3 s (keyer + print), `build`
in ≤ 0.5 s (keyer + restore of `dist/`). The hit `test` step 11 → ~1 s
(render), build 2 → 0.5. **Hit job 27 → 15–17 s.** Recording: once per
unit key, +3–6 s on `test:release`'s 10 s with `--seccomp-bpf`, +1 s on
build. Acceptance on convex over 12 commits: zero wrong units; an edit to
`scripts/x.test.mjs` misses `test:release` and nothing else; a `src/`
edit misses `build` and misses `test:release` only if the trace shows it
reads `dist/` (let the trace say, not the spec).

### B2. Soundness of the environment rule (5b7a5a4, 92bab3e, ff6ce1f)

Read: `record.mjs` `toolCaller` (:266), `NPM_HELPERS` (:265), `patchEnv`
`rec` (:586–595), `recPresence` (:605–612), `ownKeys` (:627–629),
`ciVarying` (:574); `child.mjs` (:12–27); `vitest.rs` `VALUE_KEYED`
(:129), the `env` hash (:2527), `volatile_env` (:3303);
`quod-key/src/lib.rs` `DENYLIST`, `VOLATILE`, and :310–317.

**Finding 1, wrong-capable: the npm allowance is by stack frame, not by
process.** For every `ciVarying` name, `toolCaller(true)` treats a stack
whose non-node frames are all under `/node_modules/npm/`,
`/node_modules/@npmcli/` or `/node_modules/ci-info/` as a tool's read and
records *nothing* (`if (c.tool) return`, before `env:` or `envp:`).
`ci-info` is an ordinary dependency (`is-ci`, jest vendor it; repos import
it directly). A test file that imports it evaluates it under node's ESM
loader, whose frames `skipFrame` drops, so ci-info's module-init reads of
`GITHUB_ACTIONS` and `GITHUB_EVENT_NAME` have only ci-info frames on the
stack: tool, unkeyed. ci-info exports `isPR = (GITHUB_EVENT_NAME ===
'pull_request')`. A file with `test.skipIf(ci.isPR)('needs secrets', ...)`
recorded on a push (ran, passed) replays on the pull request as passed
where plain prints skipped; recorded on a pull request as skipped, it
replays on main as skipped where plain runs it and may fail. **Yes, this
produces a wrong replay.** quod-key keys `GITHUB_EVENT_NAME` by value for
exactly this reason (lib.rs:310–317, naming ci-info's `isPR`); record.mjs's
`ciVarying` lumps it in with `GITHUB_*`, and 5b7a5a4 made the lump
unkeyed through ci-info's frames. The same shape exists, dormant, for
`std-env` in `HELPERS`: it exports `provider`, derived from
`GITHUB_ACTIONS` presence, constant on hosted runners (its `isCI` reads
`CI`, keyed regardless of caller). Not wrong today; the fix below covers it.

Fix:

a. **The allowance is a property of the process.** `child.mjs` sets
   `r.npm = true` iff the real path of `process.argv[1]` ends in
   `/npm/bin/npm-cli.js` or `/npm/bin/npx-cli.js` (the `#!` shims resolve
   to these; `isNode`'s head check already reads them). `rec` and
   `recPresence` call `toolCaller(r.npm && ciVarying(k))`; `ownKeys` calls
   `toolCaller(r.npm)`, not `toolCaller(true)`. Drop the free-standing
   `/node_modules/ci-info/` entry: npm's bundled copy lives under
   `/node_modules/npm/node_modules/ci-info/`, which `/node_modules/npm/`
   matches.
b. **In npm's process nothing is unkeyed; it is presence-keyed.** npm's
   reads of `ciVarying` names record `envp:<name>` (set or unset), never a
   value. Audit of npm 10/11 (bundled with Node 20–24): its behaviour
   depends on ci-info's `isCI` and `name` (the yes-prompt in libnpmexec,
   update-notifier, the `user-agent` default's `ci/github-actions`), both
   from the *presence* of `GITHUB_ACTIONS`, and on `CI`, which is keyed by
   value for every caller; `isPR` is never consulted. Presence is constant
   on hosted runners, so nothing stops replaying; a run without GitHub is a
   miss. A future npm that reads `GITHUB_REF` by value gets `env:GITHUB_REF`
   (keyed, a branch key) and one that reads `GITHUB_SHA` is refused as
   per-run: fail safe, and the summary says why.
c. npm's copy of the whole environment for the program it runs keys every
   non-CI, non-tool name by value from npm frames today (`JAVA_HOME`,
   `ImageVersion`, ...): over-keyed, a miss on each weekly image roll.
   Acceptable, but the traced child records its own reads, so treat the
   spawn-time copy as esbuild's is treated (`c.child`): with
   `node:child_process` frames below npm frames, record `envtool:` only.
   Only in a process where `traceChild` succeeded; otherwise the file is
   unobservable anyway.
d. Tests: a fixture test file `import ci from 'ci-info'; test.skipIf(ci.isPR)`
   recorded with `GITHUB_EVENT_NAME=push` must carry `env:GITHUB_EVENT_NAME`
   and not replay under `pull_request`; the `npx` fixture must still
   record with `envp:GITHUB_ACTIONS` and no `env:GITHUB_*`.
e. **Bump `VERSION` (`quod-vitest-1` → `-2`).** A record made under
   5b7a5a4 for a file that imports ci-info carries no mark of the missing
   key; every record re-proves once. One window on the bench.

**Finding 2, not wrong on hosted runners, but the stated guarantee is
false: `VALUE_KEYED` names are not keyed by value.** `vitest.rs:2527`
hashes `"env" if volatile_env(p) => "volatile"` and all three names are
volatile (`GITHUB_EVENT_PATH`, `GITHUB_ACTION_PATH` ∈ `VOLATILE`;
`RUNNER_TEMP` ∈ `DENYLIST`), so `env:RUNNER_TEMP` hashes to a constant;
`VALUE_KEYED` only lifts the refusal at :1334. A file that reads
`RUNNER_TEMP` and writes or prints under it therefore replays on a runner
whose `RUNNER_TEMP` differs (self-hosted, `/tmp/runner`), showing hosted
paths: the "a runner where it differs is a miss" in the comment does not
hold. The bench cannot show it because the path is fixed on hosted runners.
Fix, one line: `"env" if volatile_env(p) && !VALUE_KEYED.contains(&p)`.
`GITHUB_EVENT_PATH`'s content is sound as is: a read is
`f:/home/runner/work/_temp/_github_workflow/event.json`, hashed from the
file at key time; even a `schedule` payload carries `repository.pushed_at`.
Only a native child reading it (`jq`) escapes, and that file is already
unobservable.

**Note 3, not a hole, write it down.** A traced child's `env:` keys are
hashed in `quod vitest`'s own environment (`std::env::var_os`), so a name
only npm sets for its child (`npm_lifecycle_event`, `npm_config_user_agent`,
`INIT_CWD`) hashes as "unset" at record and at replay alike. Every such
value derives from keyed inputs (the spawning test's code, `package.json`,
npm's own files as `f:`, node's version), and `npm_config_user_agent`'s
`ci/github-actions` is covered by fix 1b. Say so in record.mjs's header so
nobody "fixes" it into a per-run key.

Verdict: one wrong-capable rule (Finding 1) and one false comment with a
dormant cross-runner hole (Finding 2). Half a day, before the next bench.

### B3. The remaining 15 s above the floor

| in the 27 s hit | s | cut | how | after |
|---|---:|---:|---|---:|
| `test:release` | 10 | −9.7 | B1 | 0.3 |
| `build` | 2 | −1.5 | B1 (keyer + `dist/` restore) | 0.5 |
| action main | 2 | −1.3 | S4: static quod in `dist/`, manifest GET by tree OID, no 3.5 MB ledger restore | 0.7 |
| post | 1 | −0.7 | a hit saves nothing (section 5.3) | 0.3 |
| render | ~1 | −0.5 | confirm it is a print, not a vitest boot; the 3.5 MB ledger parse goes with S3's content-addressed records | 0.5 |
| setup-node 5, checkout 1–2, boot 1, bookkeeping 2–4 | ~10–12 | 0 | the customer's steps and GitHub's; no pipeline edits | — |

**Hit job after B1 + S4 trims: 14–17 s.** The ≤ 20 s pass mark holds
with ~3 s of GitHub's own variance to spare; report the floor beside it.

Window runner-seconds, from the run's 0.51x with timing-sensitive replay
on (plain window ≈ 12 × 140 s = 1680 s, Quod ≈ 860 s): six hits × −12 s
≈ −72 s → 0.47x; first-witness strace on units, once or twice in the
window at +4–8 s each → **≈ 0.48x**. Two conditions, both to be stated in
the report rather than tuned around: timing-sensitive replay (A1 (b)) must
be on, since with the default 0.5 margin `stream.property` runs 98 s on
every hit and no unit work gets under 0.5x; and misses stay at ≈ 1.1x
(S1's plain second witness). S6 (first-witness overhead) is what turns a
0.48 into a comfortable 0.42.

### B4. Order

1. B2 fixes and the `VERSION` bump (half a day); one bench window,
   wrong = 0 must hold before anything else lands on it.
2. B1: measure `--seccomp-bpf` on probe/runner (one hour); then the trace
   parser and keyer (3–4 days), `hook.cjs` unit recognition as in section
   1, bwrap for both witnesses.
3. S4 trims to action main and post if not already complete.
