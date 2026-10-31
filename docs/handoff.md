

# Stillpass: Phase 0 engineering handoff
2026-10-30 · @julio
This tab is the brief for the coding agent. It covers the six-week spike only. The Spec tab is the why; this is the what, in what order, and how we know it worked.

## Read this first
You are building a six-week spike, not the product. The spike answers one question: can a host-side filesystem access log serve as a complete, replayable input manifest for a real CI step, at under 10% overhead? Everything in this tab exists to answer that. Anything that doesn't is out of scope, even if the Spec tab describes it.
The spike has two halves. 0a proves memoization on the host: a FUSE-served workspace, a sandboxed go test, a manifest, a replay. It needs only Linux. 0b moves the same step into a Firecracker microVM and measures overhead. It needs /dev/kvm. Finish 0a completely before touching 0b. If 0a fails, 0b doesn't matter.
One safety invariant governs every design choice: over-record, never under-record. Logging a read the process didn't strictly need costs a cache miss. Missing a read it did need produces a false hit, and a false hit is the one bug this project cannot have. When in doubt, record it.
Work the tickets in the Build order section, one at a time, one PR each, tests included. Don't start a ticket whose dependencies aren't merged. Don't widen a ticket. If something is ambiguous, pick the conservative option, write the question in the Open questions table at the bottom, and keep moving. Don't stall waiting for an answer.
The Spec tab is context. If it and this tab disagree, this tab wins for the spike.

## Environment
Linux only. macOS cannot run this spike; use a Linux box or VM. Check the host before writing code; most "bugs" in the first week will be the machine.

 | 
Requirement | 
For | 
Check | 
If it fails
 | 
Linux x86_64, kernel 5.11 or newer | 
Unprivileged overlayfs inside user namespaces | 
uname -r | 
Older kernel: run integration tests under sudo
 | 
/dev/fuse and fuse3 installed | 
The host CAS filesystem (0a and 0b) | 
ls /dev/fuse && fusermount3 -V | 
Install fuse3; in containers, needs --device /dev/fuse
 | 
user_allow_other in /etc/fuse.conf | 
Sandboxed process (different uid) reading the FUSE mount | 
grep user_allow_other /etc/fuse.conf | 
Add the line
 | 
bubblewrap (bwrap) | 
Host sandbox with blocked egress (0a) | 
bwrap --version | 
Install bubblewrap
 | 
Unprivileged user namespaces enabled | 
bubblewrap and unprivileged overlay | 
sysctl kernel.unprivileged_userns_clone (Debian) or unshare -Ur true | 
Enable, or run with sudo
 | 
Rust stable, pinned in rust-toolchain.toml | 
Everything | 
cargo --version | 
rustup
 | 
Go, pinned to the spike repo's go.mod toolchain | 
The step under test | 
go version | 
Install the matching release
 | 
sqlite3 | 
Ledger inspection | 
sqlite3 --version | 
Install
 | 
/dev/kvm writable by your user | 
Firecracker (0b only) | 
ls -l /dev/kvm && [ -w /dev/kvm ] | 
Bare metal or a nested-virt instance; add user to kvm group
 | 
Firecracker binary, pinned | 
0b | 
firecracker --version | 
Pin the current release at project start, record in versions.toml
 | 
virtiofsd (rust-vmm), pinned | 
Exporting the workspace into the VM (0b) | 
virtiofsd --version | 
Same
 | 
Guest kernel and rootfs | 
0b | 
built by scripts/build-guest.sh | 
See ticket T13
All pinned versions live in one file, versions.toml, at the repo root. Nothing else states a version. The guest rootfs digest is recorded there too, because it's part of every step key.
For 0b, a standard cloud VM will not work unless it exposes nested virtualization. Plan on bare metal (Hetzner, Equinix, a home box) or an instance type that documents KVM support.

## Spike goal and acceptance criteria
The spike is done when every row below is green on the spike repo, from a clean checkout, by running one script: scripts/acceptance.sh.
The spike repo. Default: cli/cli (the GitHub CLI). Go, a test suite that runs in minutes, tests that mock HTTP rather than call it, real Actions workflows in the repo. Before committing to it, verify that go test ./... passes offline after go mod download. If it doesn't, fall back to any Go repo meeting those criteria and record the choice in Open questions.
The two steps. Step A: go mod download. Network allowed, never memoized in the spike, exists only to fill the module cache. Step B: go test ./.... Egress blocked. This is the step the spike memoizes.

 | 
# | 
Criterion | 
How it's checked | 
Pass
 | 
A1 | 
First run of step B on commit N executes and records a manifest and result | 
stillpass run prints MISS; stillpass inspect shows one manifest | 
Yes/no
 | 
A2 | 
Second run on the same tree replays | 
Prints HIT with a receipt; exit code identical; stdout byte-identical to the recorded run | 
Yes/no
 | 
A3 | 
A commit touching only README.md replays | 
HIT on commit N+1 | 
Yes/no
 | 
A4 | 
A commit changing a Go file read by the tests misses | 
MISS on commit N+2; receipt explains which path differed | 
Yes/no
 | 
A5 | 
Adding a file to a directory the step listed misses | 
MISS; the mismatch names the directory listing | 
Yes/no
 | 
A6 | 
Changing an env var in the step key misses | 
Different key, no manifests found | 
Yes/no
 | 
A7 | 
The manifest is complete | 
Mutation test: for every file the step read, mutate it and assert MISS. Zero false hits across the whole read set | 
0 false hits
 | 
A8 | 
Observer overhead on the host (0a) | 
Median of 10 cold runs under FUSE + sandbox vs 10 plain runs | 
Under 10%
 | 
A9 | 
The same A1 to A7 pass inside Firecracker (0b) | 
stillpass run --vm | 
Yes/no
 | 
A10 | 
Observer overhead in the VM (0b) | 
Same protocol as A8, plain VM vs observed VM | 
Under 10%, or a written breakdown of where the time goes
 | 
A11 | 
Hit rate across history | 
Last 50 commits of the spike repo, in order | 
Reported, no target
A7 is the one that matters. It is the direct test of the hypothesis. If A7 fails and the cause can't be fixed by recording more, the spike has falsified the approach and that's a successful spike too.
A10 is allowed to fail on the number as long as it succeeds on the explanation. The FUSE-under-virtiofsd double hop is a known cost we chose for speed of building; the measurement tells Phase 1 whether logging has to move into virtiofsd.

## Codebase architecture
One Cargo workspace, nine crates, strict layering. Dependencies point down only. If a crate in a lower layer needs something from a higher one, the design is wrong; stop and fix the boundary rather than adding the edge.

 | 
Crate | 
Owns | 
Must not know about
 | 
sp-cas | 
Blob store on local disk keyed by BLAKE3. Import a directory into a tree (path to hash map), materialize a tree to a directory, get and put blobs. | 
Steps, manifests, FUSE, VMs
 | 
sp-key | 
Canonical step key: the exact JSON form, env normalization and denylist, hashing. Pure functions. | 
Filesystems, storage
 | 
sp-manifest | 
Manifest and result types, serialization, match(manifest, tree) with a structured mismatch reason. Pure functions. | 
Storage, execution
 | 
sp-proto | 
Message types for host to guest communication over vsock. Serde only. 0b. | 
Everything else
 | 
sp-fs | 
The observer. A FUSE filesystem that serves a CAS tree read-only and logs every lookup, getattr, readdir, open and read as manifest entries. Sets up the kernel overlay (FUSE lower, host upper) and diffs the upper dir into outputs at step end. | 
Ledger, replay, GitHub
 | 
sp-sandbox | 
Runs a command with a given cwd, env and mounts. 0a: bubblewrap with the network namespace unshared. 0b: Firecracker lifecycle, virtiofsd, vsock client. Returns stdout, stderr, exit code, wall time. | 
Manifests, the ledger
 | 
sp-ledger | 
SQLite. Stores manifests and results by step key, runs, and the blob index. Lookup returns candidate manifests newest first; verify checks one against a tree via sp-manifest. | 
FUSE, sandboxes
 | 
sp-replay | 
Given a matched result: write outputs into the workspace, print recorded stdout and stderr with a REPLAYED banner, return the recorded exit code, print the receipt. | 
How the result was produced
 | 
sp-cli | 
Subcommands, flags, wiring, exit codes. Thin. | 
Anything that could be tested without a terminal
 | 
sp-guest-agent | 
Static binary baked into the rootfs. Listens on vsock, receives a step, runs it, streams io, reports exit. 0b. | 
Everything except sp-proto
Two of these crates carry all the risk: sp-fs, because it is the hypothesis, and sp-sandbox in 0b, because Firecracker plumbing is where weeks disappear. Everything else is deliberately boring.

## Schemas
These are the contract. Change them only with a schema version bump, updated golden tests, and an edit to this section in the same PR. Every hash is BLAKE3, written as b3: followed by 64 lowercase hex characters. Every JSON document is canonical: keys sorted bytewise, no whitespace, UTF-8, integers only.
Step key. The identity of "this command in this environment," before any inputs are known.

Rules: env is sorted by name, exactly the list above in the spike (no inherited host env, ever). image is the environment pin, an object since key v3 (2026-10-31): {"digest","kind","mounts","platform"}. kind rootfs: digest is the BLAKE3 of the imported root filesystem's canonical tree document (`stillpass rootfs import`; the step ran in it and nothing of the host's userland was visible). kind host: digest is the BLAKE3 of the output of scripts/host-fingerprint.sh (the fallback, --host-root; not portable). mounts is a list of [destination, tree digest] pairs sorted by destination: read-only trees bound into the root, pinned by content (empty for the CLI; a destination at, under or above an observed root is an error). platform is {"arch","cpu_features","cpus","host","kernel","page_size"}: uname -m, the CPU features user space dispatches on, the CPUs in the affinity mask, the host class, uname -r reduced to major.minor, the page size; what the kernel and CPU expose that no root filesystem pins. host (since key v3) is a list of [name, value] pairs sorted by name, unique: what is constant about the host and observable by, or in force for, a step, read as the step sees it in a sandbox with a fresh procfs (sp_sandbox::host_class_inside: full kernel release, build, command line and config, the effective rlimits and fixed process state, online and configured CPUs, /proc/cpuinfo without clock lines, exact MemTotal, an allowlist of sysctls, security context without MCS categories, seccomp mode and filter count, no_new_privs, personality, nice, NIS domain name) and from outside (host_class_outside: cgroup pids.max and memory.max, binfmt_misc registrations, the LSM list, the filesystem types of the store and run directories, the sandbox's version) plus the root tmpfs limits; not keyed and documented as gaps: seccomp filter contents, LSM policy rules, the workspace upper's filesystem hash seed; an audit re-runs on the same host and can never see a dependence on these, so they are keyed, and any relaxation must be an explicit opt-in recorded in the key. v is 3; warm marks a warm-state step. cmd is the script text verbatim. step_key is b3: + BLAKE3 of this canonical JSON.
Manifest. What one execution of a step actually read. Paths are relative to a root, sorted bytewise, unique.

Entry kinds: f file (content hash plus mode masked to 0o777), d directory (hash of the listing: one line per child, name<TAB>kind<LF>, sorted, hashed), a absent (a lookup returned ENOENT), l symlink (target string). A stat records f even if the content was never read. A readdir records d. Never record mtime, size or inode. manifest_id is b3: + BLAKE3 of the canonical JSON.
Tree. The materialized workspace at step start: the commit's files plus outputs of earlier steps. Same entry shape as a manifest, covering every path. Produced by sp-cas import. Matching is for each manifest entry, the tree agrees: same hash and mode for f, same listing hash for d, no entry for a, same target for l. The first disagreement is returned as Mismatch { path, expected, actual } with a total count.
Step result. What the step produced.

outputs are the overlay upper dir diffed against the tree: files written or modified, plus x for deleted paths (whiteouts). stdout and stderr are blobs. run_id is a ULID. Since result v2 (2026-10-31), root_outputs (present only when non-empty) are the same diff of the per-step overlay over the whole root filesystem in rootfs mode, paths relative to /: what the step wrote outside the workspace, except /tmp (scratch, never an output) and sockets and fifos (no content; one that replaced an image file is recorded as that file's deletion).
Receipt. Plain text, printed before the replayed log. Hashes shown as the first 8 hex characters.

On a miss the banner says EXECUTED and, when candidate manifests existed, names the first mismatch: closest manifest differed at pkg/cmd/root.go.
Ledger (SQLite).

Lookup: the 50 newest manifests for the step key, verified in order against the tree; the first match wins and its newest result is returned. Blobs live on disk at <cas>/<first 2 hex>/<hash>; the table is an index, not the store. Ledger v2 (2026-10-31) records each executed run's observer transport; v3 adds step_keys (step_key, json): the canonical JSON behind every key a run computed, so a result's environment pin can be read back (`stillpass inspect <step_key>`).
Ledger trust schema (exp/trust, see DECISIONS.md; v4, after the transport columns (v2) and step_keys (v3)): runs gain step_key, repo, repo_path, mode, outcome, manifest_id, exit_code, wall_ms, saved_ms; new tables trust (one row per step key: state, counters, quarantine), proofs (one row per key and manifest: streak, result fingerprint, last run) and audits (every comparison of an execution with a recorded result). The exact schema is the golden fixtures/ledger_schema.sql; v1 ledgers migrate in place (fixtures/ledger_v1.sql). Lookup is unchanged: whether a matched result may replay is the trust layer's decision (sp-trust), not the lookup's.

## Component specs
Behavior and invariants per crate. Public API sketches are Rust signatures; names can change, shapes shouldn't.
sp-cas
- 
Cas::open(dir) -> Cas. Blobs at <dir>/<hh>/<hash>, written to a temp file and renamed, so a crash never leaves a partial blob under its hash.
- 
put(bytes) -> Hash, get(hash) -> Option<Bytes>, has(hash) -> bool.
- 
import_dir(path) -> Tree: walks a directory, hashes every regular file, records mode and symlink targets, skips nothing (.git included; the sandbox mounts a tree without it, but the import is complete). Parallel hashing; files over 1 MiB streamed.
- 
materialize(tree, path): writes a tree to a directory with modes and symlinks. Used for the workspace at step start and for outputs on replay.
- 
Invariant: a hash in a manifest or result always resolves in the CAS. Results are written only after all their blobs are.
sp-key
- 
StepSpec { image, cmd, cwd, env, roots } and canonical_json(&StepSpec) -> String, step_key(&StepSpec) -> Hash.
- 
env is accepted in any order and sorted; duplicate names are an error, not a silent last-wins.
- 
Golden test file fixtures/step_keys.jsonl: spec in, key out. A changed golden is a schema bump.
sp-manifest
- 
Manifest, Entry, Tree, StepResult, Mismatch. Serde with exact field names from the Schemas section (p, k, h, m, t).
- 
Manifest::from_log(roots, log) -> Manifest: builds entries from the sp-fs access log, dedups, sorts, and resolves conflicts conservatively (a path both read and listed yields two entries).
- 
matches(&Manifest, &Tree) -> Result<(), Mismatch>.
- 
manifest_id(&Manifest) -> Hash.
sp-fs
- 
serve(tree, cas, mountpoint) -> Served. A read-only FUSE filesystem (fuser crate) presenting a Tree. Content is fetched from the CAS on first read and kept in memory per step. The log is an append-only channel of Access { path, kind } events, drained by the caller at step end.
- 
Logged ops and what they record: lookup success → f, d or l by kind; lookup ENOENT → a; getattr → f, d or l; readdir → d; open and read → f; readlink → l. Nothing else is needed; statfs and access are not inputs.
- 
overlay(served, upper, work, merged) -> Overlay: mounts kernel overlayfs with the FUSE mount as lower. Overlay::finish() -> Vec<Output>: walks the upper dir, hashes new and modified files into the CAS, converts whiteouts to x entries, unmounts.
- 
Mount options: allow_other (the sandbox runs as a different uid), default_permissions, auto_unmount. Kernel caching is allowed; it only reduces repeat reads, and repeat reads carry no new information.
- 
Invariant: over-record. If an op's classification is unclear, record the most specific kind that is certainly true, or record both.
sp-sandbox
- 
run(RunSpec { cmd, cwd, env, mounts, egress: bool }) -> RunResult { stdout, stderr, exit_code, wall }.
- 
0a backend: bubblewrap. --unshare-all, --share-net only when egress is true, --bind merged /workspace, --ro-bind /usr /usr and the other read-only system paths, --dev /dev, --proc /proc, --tmpfs /tmp, --clearenv then --setenv for each key env entry, --chdir. No host env leaks in.
- 
0b backend: Firecracker. Boot from the pinned kernel and rootfs, attach virtiofsd exporting merged as /workspace, open a vsock to the guest agent, send RunSpec, stream Chunk { stream, bytes } messages, receive Exit { code, wall_ms }. No network device attached unless egress is true.
- 
stdout and stderr are captured in full and streamed to the terminal at the same time.
sp-ledger
- 
Ledger::open(path) runs migrations. put_manifest, put_result, candidates(step_key, limit) -> Vec<Manifest> newest first, latest_result(step_key, manifest_id) -> Option<StepResult>, record_run.
- 
find_match(step_key, &Tree) -> Option<(Manifest, StepResult)> with the first mismatch of the newest candidate kept for the receipt.
- 
Single writer. SQLite in WAL mode. The spike has one process; no further concurrency work.
sp-replay
- 
replay(result, cas, merged_dir) -> ExitCode: materialize outputs into the merged dir (eager in the spike; lazy is Phase 2), apply deletions, write recorded stdout and stderr to the real stdout and stderr after the receipt banner, return the recorded exit code.
- 
Receipt text exactly as in Schemas. Durations formatted as 4m12s, 0.4s.
sp-cli
- 
stillpass run --repo <dir> [--commit <sha>] --step <script> [--allow-net] [--vm] [--ledger <path>] [--cas <path>]. Imports the tree, computes the key, consults the ledger, executes or replays, prints the banner and the log, exits with the step's exit code.
- 
stillpass inspect <step_key | manifest_id | run_id>: pretty prints the stored JSON and, for a step key, lists its manifests with entry counts.
- 
stillpass bench --repo <dir> --commits <N> --step <script> [--vm] --out report.md: see Benchmark protocol.
- 
Exit codes: the step's own code on run; 2 for Stillpass errors, clearly distinguished in output.
sp-guest-agent
- 
Statically linked (musl), started by the rootfs init as PID 1's only child. Listens on vsock port 5000. One request per boot. Runs the command with /bin/sh -e -c, under the given cwd and env, with no inherited env. Streams output as it arrives in 64 KiB chunks. Reports exit code and wall time. Exits; the host tears the VM down.

## Build order
Sixteen tickets, in this order. One PR each. A ticket is done when its definition of done passes in CI, not when the code exists. T01 through T12 are 0a and need no KVM. T13 through T16 are 0b.

 | 
# | 
Ticket | 
Depends on | 
Definition of done
 | 
T01 | 
Workspace scaffold: nine crates, rust-toolchain.toml, versions.toml, CI running fmt, clippy with -D warnings, and cargo test | 
none | 
CI green on an empty workspace; scripts/check-host.sh reports every Environment row
 | 
T02 | 
sp-cas: blob store, import_dir, materialize | 
T01 | 
Round-trip property test (import then materialize yields an identical tree); importing the spike repo completes and reports file count and bytes
 | 
T03 | 
sp-key: StepSpec, canonical JSON, step_key, golden fixtures | 
T01 | 
Env permutation yields the same key; duplicate env name is an error; 10 goldens committed
 | 
T04 | 
sp-manifest: types, matches, from_log, manifest_id | 
T01 | 
Property tests: identical tree matches; any single entry perturbation (hash, mode, listing, absent now present, present now absent) mismatches and names the path; changes to paths not in the manifest still match
 | 
T05 | 
sp-fs read-only FUSE serve with access log | 
T02, T04 | 
Mount the spike repo tree; find . -type f -exec cat {} + produces a log whose f set equals the tree's file set; ls of a dir yields one d; stat of a missing path yields one a; unmount is clean
 | 
T06 | 
sp-fs overlay and output diff | 
T05 | 
Create, modify, delete, and rename files in the merged view; finish() returns exactly those as outputs with correct kinds; the lower FUSE log is unaffected by writes
 | 
T07 | 
sp-sandbox 0a backend (bubblewrap) | 
T01 | 
go test ./... passes inside the sandbox on the spike repo with the Schemas env and a pre-filled module cache; curl https://example.com inside the sandbox fails with no network; no host env variable is visible inside
 | 
T08 | 
sp-ledger: schema, migrations, put and lookup | 
T03, T04 | 
Migration test from empty; candidates returns newest first; find_match returns the first matching candidate and the first mismatch of the newest candidate
 | 
T09 | 
Execute path end to end in sp-cli: import, key, serve, overlay, sandbox, record | 
T05, T06, T07, T08 | 
stillpass run on commit N prints EXECUTED, stores one manifest and one result; stillpass inspect shows both; the manifest has more than 1,000 entries
 | 
T10 | 
sp-replay and the hit path | 
T09 | 
Second stillpass run on the same tree prints REPLAYED with the receipt; exit code and stdout bytes equal the recorded run; wall time under 2 s
 | 
T11 | 
Mutation and acceptance suite (A1 to A7) as integration tests | 
T10 | 
All of A1 to A7 pass in CI; A7 iterates every f entry in the recorded manifest, mutates it, and asserts a miss; zero false hits
 | 
T12 | 
Bench harness and 0a numbers (A8, A11) | 
T11 | 
stillpass bench produces report.md with the overhead median and p95 and the hit rate over 50 commits; numbers pasted into this doc's Benchmark section
 | 
T13 | 
Guest image: scripts/build-guest.sh producing a pinned kernel and a rootfs with Go, the agent, and a minimal init; digest recorded in versions.toml | 
T01 | 
Firecracker boots the image and the agent accepts a vsock connection; build is reproducible (same digest twice)
 | 
T14 | 
sp-proto and sp-guest-agent | 
T13 | 
echo hi round trip over vsock returns stdout hi and exit 0; a 100 MB stdout stream arrives intact
 | 
T15 | 
sp-sandbox 0b backend: Firecracker lifecycle plus virtiofsd exporting the merged dir | 
T06, T14 | 
stillpass run --vm passes A1 to A7; the FUSE log inside the VM path has the same f set as the 0a path for the same step
 | 
T16 | 
0b numbers and the gate memo | 
T15 | 
stillpass bench --vm report; a one-page memo in this doc: overhead number, where the time goes, and a recommendation for Phase 1 (keep the FUSE hop or move logging into virtiofsd)
T05 is where the spike lives or dies, and it should land by the end of week two. If the FUSE log is incomplete in ways that recording more doesn't fix, stop and write the memo early.
Two tickets are allowed to run in parallel when two agents are available: T03 with T04, and T07 with T05/T06. Everything else is serial.

## Test strategy
Six layers. The bottom three run on every PR on a stock GitHub runner. The top three need FUSE, user namespaces, or KVM, and run in a privileged job or on a self-hosted box. A PR that touches sp-fs or sp-sandbox without an integration test is incomplete.

 | 
Layer | 
What | 
Where it runs | 
Tooling
 | 
Unit | 
Pure functions in sp-cas, sp-key, sp-manifest, sp-ledger, sp-replay formatting | 
Every PR, ubuntu-latest | 
cargo test, insta for golden snapshots
 | 
Property | 
Key permutation invariance; manifest matching under random perturbation; CAS round trip; canonical JSON idempotence | 
Every PR | 
proptest, 256 cases per property, seed logged on failure
 | 
Golden | 
Step key fixtures, manifest fixtures, receipt text | 
Every PR | 
Committed JSONL and text files; a diff fails the build
 | 
Integration (0a) | 
FUSE serve and log completeness, overlay diff, sandbox isolation, execute and replay end to end | 
Every PR, privileged job (--device /dev/fuse, --cap-add SYS_ADMIN, or sudo) | 
cargo test --test integration, gated on STILLPASS_INTEGRATION=1
 | 
Mutation and acceptance | 
A1 to A7 against the real spike repo | 
Nightly and on PRs that touch sp-fs, sp-manifest or sp-replay | 
scripts/acceptance.sh, checked-out spike repo at a pinned commit
 | 
VM (0b) | 
A1 to A7 under --vm, guest agent round trips, overhead bench | 
Self-hosted runner with /dev/kvm, nightly | 
Same tests, gated on STILLPASS_KVM=1; #[ignore] without it
The mutation test is the hypothesis test. A7 takes the manifest recorded by one execution of go test ./... and, for each f entry, flips one byte in that file, re-runs lookup, and asserts a miss. Then for each d entry it adds a file to that directory and asserts a miss. Then for each a entry it creates the file and asserts a miss. Every one of those must miss. One false hit means the observer under-recorded, and the fix is in sp-fs, never in the test. Run it with the full read set, not a sample; it is slow and that's fine for nightly.
Fidelity has one wrinkle. go test prints per-package durations (ok  pkg/cmd  0.412s). A replayed log shows the recorded durations, which is correct behavior and also means stdout isn't byte-identical to a fresh run. A2 compares the replayed stdout to the recorded stdout, byte for byte. A separate fidelity check compares a sandboxed run to a plain run with duration tokens normalized, to prove the sandbox itself changes nothing.
Completeness cross-check (T15). The same step, run through 0a and 0b on the same tree, must produce manifests with the same f and a sets. Differences mean one path is under-recording. d sets may differ slightly due to guest readahead; that is over-recording and acceptable.
Sandbox isolation tests. Inside the sandbox: outbound TCP fails, env shows exactly the Schemas env, writes outside /workspace and /tmp fail, the host's home directory is not visible. These run on every PR.
What is not tested in the spike. Concurrency (one process), crash recovery beyond atomic blob writes, large repos beyond the spike repo, non-Go toolchains.

## Benchmark protocol
Two numbers come out of the spike: overhead and hit rate. Both are produced by stillpass bench and written to report.md with the machine description, so anyone can rerun them.
Overhead (A8 and A10). Measures what the observer costs on a cold execution.
- 
Fix the tree to one commit of the spike repo. Fill the module cache once with step A.
- 
Plain baseline: run step B 10 times with no FUSE and no sandbox, in a fresh copy of the tree each time, with GOCACHE emptied before each run. Record wall time.
- 
Observed: run step B 10 times through the full path (import, serve, overlay, sandbox, record) with the ledger cleared before each run so every run is a miss. Record wall time, and separately record time spent in import, mount, execution, output diff and ledger write.
- 
Report median and p95 of both, the ratio, and the phase breakdown. Overhead is observed_median / plain_median - 1.
- 
For 0b, the baseline is a plain Firecracker VM with the tree on a block device, not the host. Compare VM to VM; the VM's own cost is not the observer's.
Drop the first run of each series as warm-up for page cache. State the CPU, RAM, disk type and kernel in the report. Run nothing else on the box.
Hit rate (A11). Measures how much real history would have replayed.
- 
Take the last 50 commits on the spike repo's default branch, oldest first.
- 
For each commit: import the tree, run step A (always executes), run step B through the full path with the ledger persisting across commits.
- 
Record per commit: hit or miss, the first mismatch path on a miss, step B wall time, and the recorded duration of the matched result on a hit.
- 
Report: hit count out of 50, total wall time with Stillpass vs sum of recorded durations (the time a plain runner would have spent), and the percentage saved. List the miss commits with their mismatch paths; that list is the beginning of the taint and granularity analysis for Phase 1.
Step-level hit rate on a real repo will be modest, because most commits touch Go files that go test ./... reads. That's expected and the spec says so. The spike reports the number; it doesn't have a target. The number that does have a target is false hits, and it's zero.
Report format. report.md has four tables: machine, overhead (plain vs observed, median and p95, phase breakdown), hit rate (per commit), and a totals line. T12 and T16 paste the totals into this section when done.

 | 
Result | 
0a (host) | 
0b (VM)
 | 
Observer overhead, median | 
pending T12 | 
pending T16
 | 
Observer overhead, p95 | 
pending T12 | 
pending T16
 | 
Hits over 50 commits | 
pending T12 | 
pending T16
 | 
Time saved over 50 commits | 
pending T12 | 
pending T16
 | 
False hits | 
pending T11 | 
pending T15

## Hard parts and the spike's answer to each
These are the places an agent will be tempted to improvise. The answer for the spike is already decided; follow it and note anything that doesn't hold.
Go writes caches outside the workspace. By default GOCACHE and GOMODCACHE live under the home directory, outside the served tree, so reads from them would be invisible and writes to them would be lost. Answer: the Schemas env points GOCACHE, GOMODCACHE and HOME under /workspace/.stillpass/. Everything Go reads or writes is then on the observed mount. Step A fills the module cache into the tree; step B reads it as ordinary inputs. The build cache starts empty on every fresh tree, which is correct and makes the manifest stable.
The toolchain isn't in the tree. /usr/local/go is on the rootfs (0b) or the host (0a), not on the FUSE mount, so reads of it aren't logged. Answer: it is pinned by the image field of the step key. 0b: the rootfs digest. 0a: the host fingerprint. A different toolchain is a different key, never a false hit. Don't try to serve /usr through FUSE in the spike. (Since 2026-10-31 the 0a path too runs in a pinned rootfs under bubblewrap, --rootfs; the host fingerprint is the fallback.)
Network. Step B must not reach the network, or the manifest is incomplete by definition. Answer: GOPROXY=off, GOFLAGS=-mod=mod, GOTOOLCHAIN=local, and the sandbox has no network namespace. If step B fails for lack of a module, step A didn't fill the cache; fix step A, don't open the network. Step A is the only step allowed egress and is never memoized.
Time. go test reads the clock, and some tests depend on it. The spike doesn't observe time. Answer: accept it. Set TZ=UTC, record the known gap, and let the mutation test tell us whether anything in the spike repo actually branches on the date. If A7 passes, time is not an input for this suite. Frozen clocks are Phase 2.
Overlayfs over FUSE. Kernel overlayfs with a FUSE lower works but has rules: the FUSE filesystem must report stable inode numbers across lookups, support getxattr returning ENOTSUP cleanly, and the mount needs allow_other. Answer: assign inode numbers from the tree index at serve time (stable by construction), return ENOTSUP for xattrs, mount with allow_other,default_permissions. If overlayfs refuses the lower, the fallback is to implement copy-on-write inside sp-fs (writes to an upper map, reads check upper then tree). Try the kernel overlay first; it is far less code.
Guest readahead and the page cache. The kernel reads more than the process asked for and serves repeats from cache. Answer: both are fine. Readahead over-records, which is safe. The cache hides repeat reads, which carry no new information. Never try to defeat either.
Renames and deletes in outputs. A rename shows up in the upper dir as a whiteout plus a new file. Answer: that is exactly what the result stores (x for the old path, f for the new). Don't reconstruct renames.
Process attribution. The spike logs reads per step, not per process. go test ./... runs many compilers and test binaries; the log is their union. Answer: that's the design for the spike. Per-process attribution is Phase 3 and needs guest eBPF. Don't start it.
Large stdout. Test suites can print megabytes. Answer: stream to the terminal and to a temp file simultaneously; hash and store the file as a blob at step end. Never buffer the whole stream in memory.
Firecracker setup time. Booting a VM and starting virtiofsd per run adds seconds that look like observer overhead. Answer: the 0b baseline is also a VM, so the boot cost cancels. Snapshot restore is Phase 1; the spike boots cold every time and says so in the report.

## Non-goals for the spike
If you find yourself building any of these, stop. They are all real and all later.
- 
Parsing workflow YAML. The step is a string on the CLI.
- 
Any GitHub integration: App, JIT runner registration, Checks API, log upload, job summary.
- 
JavaScript, Docker or composite actions. Only run: style shell steps.
- 
Lazy output materialization. Eager copy is fine at spike scale.
- 
Audit sampling, taint classification, per-key trust thresholds, branch policy.
- 
Per-process attribution, eBPF, test splitters.
- 
Firecracker snapshots, warm pools, an orchestrator, more than one VM at a time.
- 
Bazel Remote Execution API, NativeLink, object storage. The CAS is a directory and the ledger is a SQLite file.
- 
Secrets handling of any kind. The spike env has none.
- 
Frozen clocks, network recording proxies, pinned-registry allowlists.
- 
A web UI, a dashboard, metrics export. report.md is the dashboard.
- 
Non-Go toolchains. One repo, one language.
- 
macOS support, Windows anything, arm64 (unless the only available box is arm64, in which case say so in the report).

## Working agreements for the coding agent
- 
One ticket per branch, one PR per ticket, in Build order. A PR is mergeable when CI is green, the definition of done is demonstrated in the PR description, and no file outside the ticket's crates changed without a stated reason.
- 
Every PR adds or changes tests. A PR to sp-fs or sp-sandbox without an integration test is returned.
- 
cargo fmt, cargo clippy --all-targets -- -D warnings, cargo test pass locally before pushing. No #[allow] without a one-line justification beside it.
- 
unsafe is permitted only in sp-fs and sp-sandbox, each block with a // SAFETY: comment. Nowhere else.
- 
Schemas are frozen. A change to any JSON shape, canonicalization rule, or ledger table requires a version bump, updated goldens, and an edit to the Schemas section in the same PR.
- 
Over-record, never under-record. When unsure whether something is an input, record it.
- 
Logging via tracing. Every stillpass run emits one structured event with step key, manifest id, hit or miss, phase durations, and the first mismatch if any. Logs are how the bench harness gets its numbers; don't print numbers that aren't also events.
- 
Errors are anyhow at the CLI boundary and typed thiserror enums inside crates. A Stillpass failure never masquerades as a step failure: exit code 2 and a clear banner.
- 
No new dependencies without a line in the PR saying why. Prefer the standard library. The expected set: fuser, blake3, rusqlite, serde, serde_json, clap, tracing, proptest, insta, anyhow, thiserror, ulid, and for 0b vsock plus Firecracker's API via plain HTTP over a Unix socket.
- 
When blocked by an ambiguity: pick the conservative option, add a row to Open questions with what you picked, keep moving. Never stall and never widen scope to avoid the question.
- 
Keep DECISIONS.md in the repo root. Every decision in this tab and every new one you make goes in it, dated, one line each. The doc is the plan; the repo is the record.
- 
Commit messages: T05: serve CAS tree over FUSE with access log. Ticket id first, imperative mood, under 72 characters.

## Open questions for Julio
Decisions the agent should not make alone. The agent appends rows as it goes, with the conservative pick it took in the meantime.

 | 
Question | 
Why it matters | 
Agent's interim pick | 
Decision
 | 
Where does 0b run? A bare-metal box with /dev/kvm, or do we buy nested-virt cloud time? | 
Blocks T13 to T16 entirely | 
Finish 0a first; assume bare metal | 

 | 
Confirm cli/cli as the spike repo, or name another | 
T02 onward imports it; switching later redoes the bench | 
Proceed with cli/cli; verify offline tests in T07 | 

 | 
License for the repo | 
Must be in the first commit | 
Apache 2.0 placeholder in LICENSE, easy to swap before anything is public | 

 | 
Fork Velnor for runner protocol reference later, or treat it as read-only reference? | 
Not a spike question, but affects how T01 lays out the workspace for Phase 1 | 
Reference only; no code copied | 

 | 
Where does stillpass CI for the spike itself run? Stock GitHub runners can't do KVM | 
Determines whether 0b tests are nightly on a self-hosted box or manual | 
Privileged job for FUSE on GitHub; 0b manual until a box exists | 

 | 
Is an arm64 box acceptable if that's what is available? | 
Changes the rootfs build and the Go toolchain pin | 
x86_64 assumed | 

