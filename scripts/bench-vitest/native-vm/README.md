# Diskless native VM investigation

This profile runs the original c11 `npm run build` and `npm run test:release`
commands inside a real Linux guest. It is an investigation of repeat-image
agreement. It issues no receipt, admits no cache hit, and makes no claim of
ordinary-runner equivalence, complete host-read tracing, or a 12-commit result.

The fixed source is `5eaa7e5160736054d53f8b71e2c522835eb427e0`.

## Authorized GitHub runner preparation

Run only in the dedicated, explicitly dispatched Linux gate. Do not build,
install or execute the VM on a developer machine.

1. Install `qemu-system-x86 linux-image-virtual gcc libc6-dev binutils cpio gzip`.
2. Check out the original source into `profile-project` with persisted checkout
   credentials disabled, select Node 24, and run its unchanged locked `npm ci`.
3. Make the installed `/boot/vmlinuz-*` readable for the unprivileged builder.
   The builder hashes the selected actual bytes; a missing kernel is a failure.
4. Run:

   ```sh
   python3 scripts/bench-vitest/native-vm/build.py profile-project "$RUNNER_TEMP/native-vm-image"
   python3 scripts/bench-vitest/native-vm/probe.py "$RUNNER_TEMP/native-vm-image" "$RUNNER_TEMP/native-vm-evidence" --watchdog 180 --release-watchdog 360
   python3 scripts/bench-vitest/native-vm/audit.py "$RUNNER_TEMP/native-vm-image" "$RUNNER_TEMP/native-vm-evidence" > "$RUNNER_TEMP/native-vm-audit.json"
   ```

Upload the image directory, evidence directory and auditor output, including
failure evidence. Retain the kernel and initramfs bytes, not just their hashes.
The diagnostic workflow has a 25-minute global timeout. Each boundary VM has a
60-second host watchdog; each build VM keeps its 180-second host watchdog. The
release-only option permits at most 360 seconds and is separately recorded in
the keyed policy and each actual invocation. Without that option, release uses
the unchanged default command limit of 180 seconds. A timeout is incomplete and
fails the gate.

## Declared boundary

`build.py` exports tracked source only, the installed dependency tree, the
actual Node/npm tools, Git/local Git helpers, and the dynamic-library closure.
It never exports checkout `.git`, runner environment, home, npm cache, Action
cache credentials or private authentication configuration. Authentication-bearing
package configuration and private metadata paths fail image preparation.

All image entries have canonical root ownership, modes and mtime, including the
archive root. The image inventory records each file, symlink and directory.
This metadata normalization is an explicit managed contract. The kernel,
initramfs, trusted supervisor source/binary, native fixture, Node, QEMU, firmware,
module/library bytes and loader configuration presence/bytes are retained or
hashed. The exact installed versioned i440fx machine is recorded. Emulator inputs
are verified unchanged before and after each run. This inventory is not an
exhaustive trace of every host file QEMU might open, nor does it close all
negative module, firmware or configuration lookups against newly added files.

The launcher uses one vCPU, single-thread TCG, fixed `Haswell-noTSX` CPU with RDRAND,
RDSEED, RDTSCP and PMU disabled, a fixed UUID, seed, RTC base and VM clock, and
fixed instruction-count timing. It supplies no NIC, block device, host mount,
QMP, monitor, guest agent, interactive serial source or stdin. The UART backend
is an output-only regular file. The guest has real owned loopback networking.

This fixed CPU includes x86-64-v3 instructions needed by the actual Ubuntu 26
guest tools and static supervisor. The first retained image's GNU ISA-needed
notes require v3 for `/init`, `env` and libc. The builder records actual guest
ELF identities and raw `readelf` note output; no host CPU passthrough is used.

Trusted root PID 1 enables loopback, disables ASLR and new user namespaces, seeds
the kernel PRNG from fixed bytes before unprivileged execution, and records an
actual nonblocking `getrandom` sample. Its executable, policy and UART are
unreadable by UID 1000. Child stdout/stderr go through guest pipes. The child
receives an explicit environment, UID/GID 1000, no supplementary groups, no new
privileges, and no inherited trusted file descriptors.

`RNDADDENTROPY` mixes a fixed seed into the kernel pool; it does not reset that
pool. Deterministic behavior must therefore be observed in the repeated actual
boots and commands. The fixed seed alone is not accepted as proof.

`/project` is an actual read-only bind mount. Its initially empty `dist` output
is a separate writable tmpfs. `/tmp`, `/run` and `/dev/shm` are declared ephemeral
scratch; only `/tmp` and `/dev/shm` are writable by the command. Guest source,
tools and other root-owned directories are not writable. The output snapshot
records the complete `dist` tree, including its root, file bytes, symlink targets,
modes, owners, links, inode/device and observed timestamps. Deleting an output
is represented by its absence from this initially empty tree; source deletions
and other workspace writes cannot succeed. Captured command output is never
normalized or masked. Output and entry bounds fail closed.

After the main process exits, PID 1 scans, kills if necessary, and reaps all
remaining UID 1000 processes. A background descendant makes the run incomplete,
even if the main process exited zero. Only after capture and quiescence does
root emit one length- and SHA-256-bound JSON envelope through the UART. Root's
SHA implementation is self-tested during remote image preparation. Spawn,
boot, command, output, cleanup, envelope and watchdog failures never become a
successful repeat result.

The diagnostic variant emits separately marked, length- and SHA-256-bound
progress records from root while the command runs. These records never count as
completion. At most 64 records and 1 MiB of JSON bodies are emitted, each at most
16 KiB. Records sample the guest monotonic clock, main process state, up to eight
processes from at most 256 directory entries, captured byte counters, and at most
512 bytes from each output prefix/tail. Sampling occurs at one guest second, or
after captured output changes with a 250 ms guest minimum. The root UART is
opened only after fork and its descriptor never reaches the child. The final
completion envelope still requires the existing quiescence checks.

Both readers reject malformed, truncated, overlapping, out-of-order or excess
frames and extra completion envelopes. Valid bounded diagnostics are retained
even when the host watchdog kills an incomplete VM. Twelve remote parser fixtures
exercise this distinction. The bounded release-only extension changes no guest
image, CPU, progress budget, command or completion requirement. It follows
retained evidence of changing release subprocesses before the earlier host
cutoff; it does not establish that either release run will complete.

## Fixed experiment

The six sequential phases are:

1. Real native negative probes for UART/init/policy/PID1 access, signal/ptrace,
   mount/user namespace, source write, host mount and external networking; a
   positive owned-loopback socket exchange must work.
2. A main process exits zero while a detached descendant remains. The supervisor
   must reject it, kill it and establish final quiescence.
3. The unchanged actual build, twice from identical image and command inputs.
4. The unchanged actual release tests, twice from identical image and command
   inputs, including their real Node/npm/Git/subprocess/fs/loopback operations.

The two actual builds and two actual release runs must each agree on raw output,
the full recorded `dist` tree and the kernel entropy sample. Different command
selectors are separate boot inputs; their entropy samples need not match each
other. The release runs must each finish all 20 original tests. A mismatch is a
failed investigation; two observations alone never establish future replay
authority.

`audit.py` is an independent data-only reader. It parses the raw gzip/newc image
(including hardlinks), hashes the actual entries, validates the fixed QEMU
invocations, reads the UART bytes and verifies every envelope, output file and
repeat comparison. It does not import artifact modules or execute image code.

The relevant QEMU semantics are documented in the official
[invocation manual](https://www.qemu.org/docs/master/system/invocation.html),
[instruction-counting design](https://www.qemu.org/docs/master/devel/tcg-icount.html),
and [replay design](https://www.qemu.org/docs/master/devel/replay.html). The gate
must establish observed agreement from its actual retained bytes.
