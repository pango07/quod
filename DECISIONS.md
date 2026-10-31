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
