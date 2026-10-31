# Decisions

The Phase 0 handoff is the plan; this file is the record. One line each, dated.

- 2026-10-30: Scope is the Phase 0 spike only (0a then 0b). The handoff wins over the spec where they disagree.
- 2026-10-30: Safety invariant: over-record, never under-record.
- 2026-10-30: License: Apache 2.0 placeholder in LICENSE (handoff interim pick; open question).
- 2026-10-30: Spike repo: cli/cli, pinned in versions.toml (handoff interim pick).
- 2026-10-30: Velnor: reference only, no code copied.
- 2026-10-30: The handoff says "nine crates" but its table lists ten (sp-proto and sp-guest-agent are both 0b). All ten exist.
- 2026-10-30: rust-toolchain.toml mirrors versions.toml [rust] because rustup only reads that file.
- 2026-10-30: Dev box is Docker Desktop on Apple Silicon: Linux aarch64, kernel 7.0, no /dev/kvm. 0a runs here (arm64, not x86_64); 0b is blocked until a KVM host exists.
- 2026-10-30: No remote yet, so each ticket is one commit on main with the definition of done in its message, standing in for one PR.
- 2026-10-30: Dev/integration runs go through scripts/dev.sh (privileged container, /dev/fuse); the target dir and spike repo live in Docker volumes.
- 2026-10-30: The tree data model (Hash, Entry, Kind, Tree) lives in sp-cas, the lowest layer, so sp-key and sp-manifest share it without depending on each other. sp-cas gains a canonical JSON helper for the same reason.
- 2026-10-30: The tree root is spelled `.` in manifests; directory listings hash `name<TAB>kind<LF>` lines sorted bytewise.
- 2026-10-30: sp-key refuses (errors on) volatile env names (GITHUB_RUN_ID and similar) rather than normalizing them out silently.
- 2026-10-30: sp-fs serves file content by pread from the CAS blob file instead of holding it in memory per step; the page cache does the holding. Same observable behavior, bounded memory.
- 2026-10-30: Manifest::from_log errors on an access whose kind contradicts the tree, and keeps both entries if a lookup was ENOENT for a path the tree has (cannot match, so cannot false-hit).
- 2026-10-30: Kernel overlayfs over the FUSE lower works on kernel 7.0 (Docker Desktop linuxkit); no copy-on-write fallback needed. The upper dir must not itself be on overlayfs, so dev runs set TMPDIR to an ext4 volume.
- 2026-10-30: Outputs include `d` entries for directories the step created (so empty directories survive replay) and `x` then contents for opaque or type-replaced directories.
- 2026-10-30: cli/cli verified as the spike repo: after go mod download, go test ./... passes offline in the sandbox (259 packages, ~15s on 18 arm64 cores).
- 2026-10-30: Steps run as uid/gid 1001 (`runner`) inside the sandbox, with a fixed passwd/group bound over /etc: as root, cli/cli's permission tests fail, and ssh-keygen needs a passwd entry.
- 2026-10-30: The host fingerprint includes a hash of the installed package list (dpkg), because host binaries outside the served tree (ssh, git, coreutils) are inputs the observer cannot see. /etc is bound read-only from the host and is a known unobserved gap for the spike.
- 2026-10-30: bwrap's root tmpfs is remounted read-only, so writes outside /workspace and /tmp fail.
- 2026-10-30: No auto_unmount: with it, fuser 0.18 mounts through fusermount3 and umount_and_join hangs on this kernel. Served's Drop unmounts on normal exit; a SIGKILLed run can leave a stale mount (fusermount3 -u).
- 2026-10-30: Review checkpoint 1 (Fable, T01-T05): a mount serves exactly one step (no drain; kernel caches would hide a second step's reads); the log is read after the session joins; a file replacing a directory drops the directory's contents in Tree::with_outputs, and a path that is both a leaf and a directory panics; names containing tab or newline are refused at import and in outputs so listing hashes stay unambiguous; large files are hashed while being streamed into the store.
- 2026-10-30: State (CAS, ledger, run dirs, events.jsonl) lives in --state, else $STILLPASS_HOME, else ~/.cache/stillpass. Structured run events go to events.jsonl so they never mix with step output.
- 2026-10-30: --commit is a check, not a checkout: Stillpass refuses if HEAD differs, and never moves the user's checkout.
- 2026-10-30: --allow-net steps (step A) run directly on the repo with egress, so their writes persist (the module cache), and are never memoized or observed.
- 2026-10-30: The workspace a memoized step writes into is ephemeral in the spike: outputs are recorded in the CAS and the result, and the run dir is removed. Nothing later in the spike reads step B's outputs.
- 2026-10-30: Banners: EXECUTED is a miss and REPLAYED is a hit (the acceptance table's MISS/HIT). Banners and receipts go to stderr; stdout is always exactly the step's stdout.
- 2026-10-30: --env NAME=VALUE adds to the step key's env (needed for A6).
- 2026-10-30: --outputs <dir> writes a step's outputs out on both execute and replay (eager); without it the ephemeral workspace is discarded, so a replay does not copy cli/cli's ~900 MB build cache.
- 2026-10-30: A7 mutates only recorded entries, so it proves the matcher rejects every recorded change; it cannot detect a read the observer missed. Added A7b (beyond the handoff): mutate files the observer did not record, expect a replay, and re-execute under a fresh key to confirm the result is unchanged (go test durations normalized).
- 2026-10-30: A7 checks each mutated tree against the ledger's candidates through sp_ledger::first_match, the same decision find_match makes, without re-reading SQLite per mutation.
- 2026-10-30: Review checkpoint 2 (Fable, T06-T11). A sandbox that fails before the step starts (no start sentinel) and any FUSE I/O error served to the step are Stillpass errors, never recorded results. Signal deaths, including 128+N exits that bwrap reports for its child, are not memoized. The host fingerprint runs with a cleared env and the sandbox PATH, hashes the go binary, and hashes /etc minus per-container files. Overlay is mounted with redirect_dir=off,metacopy=off,index=off; the diff refuses redirect/metacopy markers and checks both opaque xattr namespaces. A7b corrupts files instead of appending. The bench baseline runs in the same sandbox without FUSE/overlay and must pass; step A's gocache/home are cleared before step B.
- 2026-10-30: T12 0a numbers (Docker Desktop, arm64, 18 cores): overhead +25.7% median (target 10%); FUSE during execution is ~+11% and the output diff (storing ~900 MB of Go build cache per miss) another ~+11%. Hit rate 12/50 commits, 10.8% saved. Full report: docs/bench/0a-cli-cli-arm64.md.
- 2026-10-30: exp/misses, the output tax. Overlay::finish moves output files into the CAS instead of copying them (Cas::take_files): hash every file in place, one syncfs(2) of the store's filesystem, then rename each new file to its blob path. The flush precedes the renames, so a blob name still never points at bytes that are not on disk (the guarantee put_file gets from fsync-then-rename), and the result is written only after that: one flush instead of one fsync per blob (~7k per cli/cli miss: Go rewrites every cache entry file). Blobs get mode 0644 however they arrived; across filesystems a file falls back to put_file. Measured on a loaded box with the base and new binaries interleaved: output diff 3.4-3.6s -> 0.8-1.4s with the CAS populated (steady state, ~3 MB new per miss), 2.6s -> 0.6s with it empty (840 MB new). What remains is the overlay unmount's own writeback of the ~840 MB Go wrote (0.3-0.5s, inherent to writing it) and hashing it (~0.3s); not fsyncs.
