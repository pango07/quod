//! Runs a command with a given cwd, env and mounts, and returns its exit code
//! and wall time. stdout and stderr are streamed to files (never buffered
//! whole) and, optionally, to the terminal at the same time.
//!
//! Backend: bubblewrap, with the network namespace unshared unless egress is
//! allowed. The root is either a pinned rootfs (`RunSpec::rootfs`: a
//! directory bound at `/`, typically a per-step overlay over the rootfs the
//! caller serves from its store, nothing of the host's userland visible) or,
//! as the fallback, the host's own system paths bound read-only.
//!
//! In rootfs mode the step sees no host file: `/` is the rootfs, `/dev` a
//! fresh tmpfs with bubblewrap's standard nodes, `/proc` a procfs of the
//! step's own pid namespace, `/sys` the rootfs's empty directory, the
//! hostname `quod`, the user `runner` (uid 1001) owning every file of
//! the rootfs.
//!
//! The step's process state is fixed, not inherited (`PROCESS_STATE`): umask
//! 022, nice 0 (where the caller may lower its nice value), the rlimits of
//! `RLIMITS`, every signal at its default disposition and none blocked, and
//! no file descriptor but 0, 1 and 2.
//!
//! What it can still learn about the host goes through the kernel. The key
//! pins what is constant for a host and observable or in force: the
//! `host_platform` fields (arch, kernel major.minor, CPU features, CPU
//! count, page size) and the host class, read as the step sees it
//! (`host_class_inside`, run in the sandbox by `probe_in_sandbox`: full
//! kernel release, build and command line, exact MemTotal, online and
//! configured CPUs, /proc/cpuinfo without its clock lines, an allowlist of
//! sysctls, the effective rlimits, umask, signal state and descriptors,
//! seccomp mode and filter count, no_new_privs, security context without
//! MCS categories, personality, nice, NIS domain name) and as only the host
//! sees it (`host_class_outside`: cgroup pids.max and memory.max,
//! binfmt_misc registrations, the LSM list, the filesystem types of the
//! store and run directories, bubblewrap's version). Known gaps, constant
//! for a host and not keyed: the contents of a seccomp filter (mode and
//! count are keyed; no unprivileged interface exports the program) and of
//! the AppArmor or SELinux policy (the label is keyed, not its rules), and
//! the directory hash seed of the filesystem under the workspace overlay's
//! upper (the type is keyed), which orders listings of directories a step
//! creates in the workspace. What varies from run to run or boot to boot is
//! left to audit, which sees it vary: the clock, load and pressure, free
//! memory and disk, uptime and the per-boot random boot id, pids, device
//! and inode numbers of the FUSE mounts, /proc/self/mountinfo and
//! /proc/1/cmdline (they name the run directory), randomness.
//!
//! A change to what the sandbox shows a step (its mounts, /dev, identity,
//! hostname, process state) is a change of environment that no input
//! records: it ships with a step key version bump.

use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct Mount {
    pub src: PathBuf,
    pub dst: PathBuf,
    pub writable: bool,
}

#[derive(Clone, Debug)]
pub struct RunSpec {
    pub cmd: Vec<String>,
    pub cwd: String,
    /// The complete environment. Nothing from the host is inherited.
    pub env: Vec<(String, String)>,
    pub mounts: Vec<Mount>,
    pub egress: bool,
    pub stdout: PathBuf,
    pub stderr: PathBuf,
    /// Also copy output to this process's stdout and stderr as it arrives.
    pub echo: bool,
    /// The root filesystem to run in, bound at `/` in place of the host's
    /// system paths, so nothing of the host's userland is visible. `None`
    /// runs on the host's /usr, /etc and friends (`SYSTEM_PATHS`), read-only.
    pub rootfs: Option<RootFs>,
    /// Set by `run`: where the generated passwd and group files live.
    #[doc(hidden)]
    pub identity: Option<PathBuf>,
    /// The step's uid and gid inside its user namespace. `None`: `STEP_UID`
    /// (1001), as on a GitHub runner. Root (`Some(0)`) keeps the root
    /// filesystem's own passwd and group; any other uid gets
    /// `identity_files`, bound over them. The gid defaults to the uid.
    pub uid: Option<u32>,
    pub gid: Option<u32>,
    /// Count the step's attempts to reach the network (no egress only):
    /// the sandbox's own network namespace has nothing but loopback, so
    /// every connect, send or DNS query aimed elsewhere fails with
    /// ENETUNREACH, which the kernel counts (Ip OutNoRoutes, Ip6OutNoRoutes)
    /// per namespace. The wrapper reads the counters after the command
    /// exits. DNS needs a resolv.conf naming an address that is not local
    /// (the caller binds one), or a lookup goes to loopback, uncounted.
    ///
    /// Limit: this counts only packets with no route (OutNoRoutes), the
    /// signal that a process reached for an address the namespace cannot
    /// carry. It does not count a connect to loopback (127.0.0.1 or ::1):
    /// those have a route, so they fail with ECONNREFUSED, not a route
    /// error, and are not counted. In the offline sandbox loopback has no
    /// listener and reaches no service, so a loopback attempt performs no
    /// side effect; a step that must taint on *any* socket attempt, rather
    /// than only unroutable ones, needs a seccomp-notify or eBPF probe,
    /// which this does not yet have.
    pub net_probe: bool,
}

/// A directory to use as the step's `/`. It must already hold the mount
/// points the sandbox needs: `dev`, `proc`, `tmp`, the destination of every
/// mount (`/workspace`), and `etc/passwd` and `etc/group` (replaced by the
/// step identity); a missing one would be created in it. An imported
/// rootfs has them all.
#[derive(Clone, Debug)]
pub struct RootFs {
    pub dir: PathBuf,
    /// Bind read-write: `dir` is a per-step overlay whose upper dir captures
    /// everything the step writes outside the workspace, /tmp included (the
    /// caller keeps that upper on a tmpfs). Read-only otherwise, with a
    /// tmpfs on /tmp and every other write failing.
    pub writable: bool,
}

/// `STEP_UID` as a number.
const DEFAULT_UID: u32 = 1001;

/// The passwd and group bound over the root's for a step running as
/// `uid`:`gid` (exposed so callers can key exactly what the step sees).
/// For the default uid these are the fixed files every step has always had.
pub fn identity_files(uid: u32, gid: u32) -> (String, String) {
    let mut passwd = "root:x:0:0:root:/root:/bin/sh\n".to_string();
    let mut group = "root:x:0:\n".to_string();
    if uid != 0 {
        passwd += &format!("runner:x:{uid}:{gid}:runner:/workspace/.quod/home:/bin/sh\n");
    }
    if gid != 0 {
        group += &format!("runner:x:{gid}:\n");
    }
    (passwd, group)
}

/// The uid and gid a step given `RunSpec::uid` and `RunSpec::gid` runs as,
/// which is also the owner it sees of every file the observer serves (the
/// sandbox maps the caller's ids onto them): what a served index records.
pub fn step_ids(uid: Option<u32>, gid: Option<u32>) -> (u32, u32) {
    let uid = uid.unwrap_or(DEFAULT_UID);
    (uid, gid.unwrap_or(uid))
}

impl RunSpec {
    fn uid_gid(&self) -> (u32, u32) {
        step_ids(self.uid, self.gid)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunResult {
    pub exit_code: i32,
    /// The signal that killed the step, if any (exit_code is then 128+sig).
    pub signal: Option<i32>,
    pub wall: Duration,
    /// Set when `run_with` killed the step itself (a limit, not the step's
    /// own doing). Such a run has no result worth recording.
    pub killed: Option<Killed>,
    /// Processes the kernel killed for memory while the step ran: the
    /// larger of the step's own cgroup count (`memory.events`, with a step
    /// cgroup, see `cgroup_root`) and the machine-wide `/proc/vmstat` delta.
    /// The step cgroup usually has no memory limit, so a kill under global
    /// pressure shows only in vmstat; either signal counts (on a shared
    /// machine vmstat also counts other tenants' kills, which only ever
    /// keeps a step from being memoized). A step one of whose processes was
    /// killed (a linker, say) can still exit 1 rather than die of the
    /// signal, and that exit is the machine's: callers must not memoize it.
    pub oom_kills: u64,
    /// With `net_probe`: how many times the step tried to reach a network
    /// it did not have. `None` when not probed, or when the counters could
    /// not be read (callers that asked must assume an attempt).
    pub net_attempts: Option<u64>,
    /// Why the outcome may be the machine's rather than the step's: OOM
    /// kills, the step cgroup's fork limit, a full disk, a fork that failed
    /// for lack of resources (from the step's own error messages). Callers
    /// must not memoize such a run.
    pub resource_failure: Option<String>,
}

/// Why `run_with` killed a step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Killed {
    /// `Limits::timeout` elapsed.
    Timeout,
    /// `Limits::abort` was raised (the caller's observer failed).
    Aborted,
}

/// Limits `run_with` enforces on a step from outside. Either one kills
/// bubblewrap with SIGKILL; as pid 1 of the sandbox's pid namespace, its
/// death takes every process the step started with it.
#[derive(Clone, Debug, Default)]
pub struct Limits {
    /// Kill the step once it has run this long.
    pub timeout: Option<Duration>,
    /// Kill the step as soon as this is set (checked every poll interval).
    pub abort: Option<Arc<AtomicBool>>,
}

/// How often `run_with` checks its limits while the step runs.
const POLL: Duration = Duration::from_millis(25);

#[derive(Debug, thiserror::Error)]
pub enum SandboxError {
    #[error("{what}: {source}")]
    Io { what: String, source: io::Error },
    #[error("bubblewrap (bwrap) not found on PATH")]
    NoBwrap,
    /// The sandbox failed before the step's command started, so its exit
    /// code is bubblewrap's, not the step's. Never a step result.
    #[error("the sandbox did not start the step (bwrap exit {exit_code}): {stderr}")]
    DidNotStart { exit_code: i32, stderr: String },
}

fn io_err(what: impl Into<String>) -> impl FnOnce(io::Error) -> SandboxError {
    let what = what.into();
    move |source| SandboxError::Io { what, source }
}

/// Read-only system paths visible inside the sandbox in host mode. The
/// toolchain under /usr is pinned by the step key's image field, not
/// observed.
const SYSTEM_PATHS: &[&str] = &["/usr", "/bin", "/sbin", "/lib", "/lib64", "/etc"];

/// Inside the sandbox: the file whose content proves the step started. Under
/// /dev (a tmpfs the sandbox makes) so that creating its mount point leaves
/// no trace in a rootfs overlay's upper dir.
const STARTED: &str = "/dev/.quod-started";
/// Where the network-probing wrapper writes the namespace's counters.
const NET: &str = "/dev/.quod-net";

/// The uid and gid a step runs as inside the sandbox.
pub const STEP_UID: &str = "1001";

/// The umask every step starts with, whatever the caller's.
pub const UMASK: u32 = 0o022;

/// The rlimits every step starts with: (resource, name, soft, hard), `None`
/// meaning unlimited. Hard limits can only be lowered, so on a host whose
/// hard limit is below one of these the step gets that host's (and soft no
/// higher): `effective_rlimits` is what is applied and what the key pins.
/// NOFILE 1024/65536 is a common runner default (Go raises its soft limit to
/// the hard one); STACK 8 MiB is glibc's default thread stack size; CORE 0
/// keeps core files out of the outputs; MEMLOCK 64 KiB is the lowest common
/// default; NPROC 16384 is a constant because the usual default (systemd's
/// threads-max/2) follows the exact amount of RAM, and the limit counts every
/// task of the caller's real uid, so it must stay well above what a parallel
/// build starts; the rest are the kernel's defaults.
pub const RLIMITS: &[(i32, &str, Option<u64>, Option<u64>)] = &[
    (libc::RLIMIT_AS as i32, "AS", None, None),
    (libc::RLIMIT_CORE as i32, "CORE", Some(0), Some(0)),
    (libc::RLIMIT_CPU as i32, "CPU", None, None),
    (libc::RLIMIT_DATA as i32, "DATA", None, None),
    (libc::RLIMIT_FSIZE as i32, "FSIZE", None, None),
    (libc::RLIMIT_LOCKS as i32, "LOCKS", None, None),
    (
        libc::RLIMIT_MEMLOCK as i32,
        "MEMLOCK",
        Some(65536),
        Some(65536),
    ),
    (
        libc::RLIMIT_MSGQUEUE as i32,
        "MSGQUEUE",
        Some(819200),
        Some(819200),
    ),
    (libc::RLIMIT_NICE as i32, "NICE", Some(0), Some(0)),
    (
        libc::RLIMIT_NOFILE as i32,
        "NOFILE",
        Some(1024),
        Some(65536),
    ),
    (libc::RLIMIT_NPROC as i32, "NPROC", Some(16384), Some(16384)),
    (libc::RLIMIT_RSS as i32, "RSS", None, None),
    (libc::RLIMIT_RTPRIO as i32, "RTPRIO", Some(0), Some(0)),
    (libc::RLIMIT_RTTIME as i32, "RTTIME", None, None),
    (
        libc::RLIMIT_SIGPENDING as i32,
        "SIGPENDING",
        Some(8192),
        Some(8192),
    ),
    (libc::RLIMIT_STACK as i32, "STACK", Some(8 << 20), None),
];

/// The rlimits a step gets on this host (see `RLIMITS`): (name, soft, hard)
/// with `u64::MAX` for unlimited.
pub fn effective_rlimits() -> Vec<(&'static str, u64, u64)> {
    let unl = |v: Option<u64>| v.unwrap_or(libc::RLIM_INFINITY);
    RLIMITS
        .iter()
        .map(|&(res, name, soft, hard)| {
            let mut cur = libc::rlimit {
                rlim_cur: 0,
                rlim_max: 0,
            };
            // SAFETY: `cur` is a valid, writable rlimit for the call.
            #[allow(unsafe_code)] // getrlimit(2) has no safe std wrapper
            let rc = unsafe { libc::getrlimit(res as _, &mut cur) };
            let host_hard = if rc == 0 { cur.rlim_max } else { 0 };
            let hard = unl(hard).min(host_hard);
            (name, unl(soft).min(hard), hard)
        })
        .collect()
}

/// The fixed process state, in words, for the key: changing it is a change
/// of environment.
pub const PROCESS_STATE: &str =
    "umask 0022; nice 0; signals default, none blocked; fds 0-2 only; rlimits as listed";

/// The hostname every step sees. `--unshare-all` gives the sandbox its own
/// UTS namespace, but the name in it is inherited from the host unless set.
pub const HOSTNAME: &str = "quod";

/// The bubblewrap argument list for `spec` (exposed for tests).
pub fn bwrap_args(spec: &RunSpec) -> Vec<String> {
    let mut a: Vec<String> = Vec::new();
    let mut push = |xs: &[&str]| a.extend(xs.iter().map(|s| s.to_string()));
    // Steps run unprivileged, as on a GitHub runner, unless the caller asks
    // for another identity (a job container's `--user`): root would bypass
    // the permission checks some test suites exercise.
    let (uid, gid) = spec.uid_gid();
    let (uid_s, gid_s) = (uid.to_string(), gid.to_string());
    push(&[
        "--unshare-all",
        "--unshare-user",
        "--uid",
        &uid_s,
        "--gid",
        &gid_s,
        "--hostname",
        HOSTNAME,
        "--die-with-parent",
        "--new-session",
    ]);
    if spec.egress {
        push(&["--share-net"]);
    }
    match &spec.rootfs {
        Some(root) => {
            let flag = if root.writable { "--bind" } else { "--ro-bind" };
            push(&[flag, &root.dir.to_string_lossy(), "/"]);
            // A step with network needs the host's resolver, and only such
            // steps (never memoized) see anything of the host.
            if spec.egress && Path::new("/etc/resolv.conf").is_file() {
                push(&["--ro-bind", "/etc/resolv.conf", "/etc/resolv.conf"]);
            }
        }
        None => {
            for p in SYSTEM_PATHS {
                let path = Path::new(p);
                match std::fs::symlink_metadata(path) {
                    Ok(m) if m.file_type().is_symlink() => {
                        let target = std::fs::read_link(path).unwrap_or_default();
                        push(&["--symlink", &target.to_string_lossy(), p]);
                    }
                    Ok(_) => push(&["--ro-bind", p, p]),
                    Err(_) => {}
                }
            }
        }
    }
    if let (Some(dir), true) = (&spec.identity, uid != 0) {
        // A fixed passwd and group for the step user, independent of the
        // host and of the root filesystem (root keeps the root's own).
        for f in ["passwd", "group"] {
            push(&[
                "--ro-bind",
                &dir.join(f).to_string_lossy(),
                &format!("/etc/{f}"),
            ]);
        }
    }
    push(&["--dev", "/dev", "--proc", "/proc"]);
    // A writable rootfs keeps /tmp: a step's leftovers there are outputs
    // like any other write outside the workspace. Elsewhere it is scratch.
    if !spec.rootfs.as_ref().is_some_and(|r| r.writable) {
        push(&["--tmpfs", "/tmp"]);
    }
    for m in &spec.mounts {
        let flag = if m.writable { "--bind" } else { "--ro-bind" };
        push(&[flag, &m.src.to_string_lossy(), &m.dst.to_string_lossy()]);
    }
    if let Some(dir) = &spec.identity {
        // Proof the step started: written by the wrapper right before it
        // execs the step's command.
        push(&["--bind", &dir.join("started").to_string_lossy(), STARTED]);
        if spec.net_probe && !spec.egress {
            push(&["--bind", &dir.join("net").to_string_lossy(), NET]);
        }
    }
    if spec.rootfs.is_none() {
        // Everything not mounted above lives on bwrap's root tmpfs; freeze
        // it. (A rootfs is bound at `/` with the mode its caller chose.)
        push(&["--remount-ro", "/"]);
    }
    push(&["--clearenv"]);
    for (k, v) in &spec.env {
        push(&["--setenv", k, v]);
    }
    push(&["--chdir", &spec.cwd, "--"]);
    if spec.identity.is_some() {
        let wrapper = if spec.net_probe && !spec.egress {
            // Not exec: the counters are read after the command, in its
            // network namespace, with shell builtins only. A signal death
            // comes back as 128+N, as bwrap reports it anyway.
            format!(
                "printf 1 > {STARTED} || exit 125; \"$@\"; rc=$?; \
                 {{ while IFS= read -r l; do printf '%s\\n' \"$l\"; done < /proc/net/snmp; \
                 while IFS= read -r l; do printf '%s\\n' \"$l\"; done < /proc/net/snmp6; }} > {NET} 2>/dev/null; \
                 exit $rc"
            )
        } else {
            format!("printf 1 > {STARTED} && exec \"$@\"")
        };
        push(&["/bin/sh", "-c", &wrapper, "sh"]);
        // A POSIX shell passes on only the variables whose names are shell
        // identifiers: dash drops INPUT_GO-VERSION-FILE and every other
        // action input with a dash in its name. env(1) puts them back.
        let odd: Vec<String> = spec
            .env
            .iter()
            .filter(|(k, _)| !is_identifier(k))
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        if !odd.is_empty() {
            a.push("env".into());
            a.extend(odd);
        }
    }
    a.extend(spec.cmd.iter().cloned());
    a
}

/// `[A-Za-z_][A-Za-z0-9_]*`: a name a POSIX shell keeps in the environment.
fn is_identifier(name: &str) -> bool {
    let mut b = name.bytes();
    b.next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == b'_')
        && b.all(|c| c.is_ascii_alphanumeric() || c == b'_')
}

/// Runs `spec` under bubblewrap with no limits.
pub fn run(spec: &RunSpec) -> Result<RunResult, SandboxError> {
    run_with(spec, &Limits::default())
}

/// Runs `spec` under bubblewrap, killing it if a limit is hit.
pub fn run_with(spec: &RunSpec, limits: &Limits) -> Result<RunResult, SandboxError> {
    let bwrap = which("bwrap").ok_or(SandboxError::NoBwrap)?;
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let ident = std::env::temp_dir().join(format!("quod-ident-{}-{seq}", std::process::id()));
    std::fs::create_dir_all(&ident).map_err(io_err("create identity dir"))?;
    let (uid, gid) = spec.uid_gid();
    let (passwd, group) = identity_files(uid, gid);
    std::fs::write(ident.join("passwd"), passwd).map_err(io_err("write passwd"))?;
    std::fs::write(ident.join("group"), group).map_err(io_err("write group"))?;
    std::fs::write(ident.join("started"), "").map_err(io_err("write start sentinel"))?;
    std::fs::write(ident.join("net"), "").map_err(io_err("write network counters"))?;
    let result = run_inner(spec, bwrap, &ident, limits);
    let _ = std::fs::remove_dir_all(&ident);
    result
}

/// Waits for bwrap, killing it when a limit is hit. Returns the exit status
/// and why the step was killed, if it was.
fn wait_within(
    child: &mut std::process::Child,
    start: Instant,
    limits: &Limits,
) -> io::Result<(std::process::ExitStatus, Option<Killed>)> {
    let mut killed = None;
    loop {
        if let Some(st) = child.try_wait()? {
            return Ok((st, killed));
        }
        if killed.is_none() {
            let aborted = limits
                .abort
                .as_ref()
                .is_some_and(|a| a.load(Ordering::SeqCst));
            let timed_out = limits.timeout.is_some_and(|t| start.elapsed() >= t);
            if aborted || timed_out {
                killed = Some(if aborted {
                    Killed::Aborted
                } else {
                    Killed::Timeout
                });
                // Already gone is fine; try_wait reports it next round.
                let _ = child.kill();
            }
        }
        std::thread::sleep(POLL);
    }
}

fn run_inner(
    spec: &RunSpec,
    bwrap: PathBuf,
    ident: &Path,
    limits: &Limits,
) -> Result<RunResult, SandboxError> {
    let ident = ident.to_path_buf();
    let spec = &RunSpec {
        identity: Some(ident),
        ..spec.clone()
    };
    let cgroup = StepCgroup::create();
    let mut cmd = match &cgroup {
        // The shell joins the step cgroup before it execs bwrap, so nothing
        // the step starts escapes it.
        Some(cg) => {
            let mut c = Command::new("/bin/sh");
            c.args([
                "-c",
                "printf '%s' $$ > \"$1/cgroup.procs\" && shift && exec \"$@\"",
                "sh",
            ])
            .arg(&cg.dir)
            .arg(&bwrap);
            c
        }
        None => Command::new(&bwrap),
    };
    cmd.args(bwrap_args(spec))
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    fix_process_state(&mut cmd);
    let oom_before = oom_kills();
    let start = Instant::now();
    let mut child = cmd.spawn().map_err(io_err("spawn bwrap"))?;
    let out =
        File::create(&spec.stdout).map_err(io_err(format!("create {}", spec.stdout.display())))?;
    let err =
        File::create(&spec.stderr).map_err(io_err(format!("create {}", spec.stderr.display())))?;
    let (child_out, child_err) = (child.stdout.take().unwrap(), child.stderr.take().unwrap());
    let echo = spec.echo;
    let t_out = std::thread::spawn(move || tee(child_out, out, echo.then(io::stdout)));
    let t_err = std::thread::spawn(move || tee(child_err, err, echo.then(io::stderr)));
    let (status, killed) =
        wait_within(&mut child, start, limits).map_err(io_err("wait for bwrap"))?;
    t_out
        .join()
        .expect("stdout tee panicked")
        .map_err(io_err("capture stdout"))?;
    t_err
        .join()
        .expect("stderr tee panicked")
        .map_err(io_err("capture stderr"))?;
    let wall = start.elapsed();
    use std::os::unix::process::ExitStatusExt;
    // bwrap is PID 1 and turns a child's signal death into exit 128+N, so
    // those codes count as signals too (conservative: a step exiting 137
    // on purpose is merely never memoized).
    let signal = status.signal().or(match status.code() {
        Some(c @ 129..=192) => Some(c - 128),
        _ => None,
    });
    let exit_code = status.code().unwrap_or_else(|| 128 + signal.unwrap_or(0));
    // The machine-wide /proc/vmstat delta: a kernel OOM kill the step's own
    // cgroup counter would miss (a kill under global pressure, the step
    // cgroup having no memory limit). c352f67's false hit was such a kill.
    let global_oom = match (oom_before, oom_kills()) {
        (Some(a), Some(b)) => Some(b.saturating_sub(a)),
        _ => None,
    };
    let (oom_kills, fork_limit) = match &cgroup {
        // Either signal counts: the precise cgroup counter for a cgroup-limit
        // kill, or the global one for a kill under machine-wide pressure.
        Some(cg) => (
            cg.events("memory.events", &["oom_kill", "oom_group_kill"])
                .max(global_oom.unwrap_or(0)),
            cg.events("pids.events", &["max"]),
        ),
        // No cgroup: the global counter alone, the worst case if unreadable.
        None => (global_oom.unwrap_or(1), 0),
    };
    drop(cgroup);
    let net_attempts = (spec.net_probe && !spec.egress)
        .then(|| {
            std::fs::read_to_string(spec.identity.as_ref().unwrap().join("net"))
                .ok()
                .and_then(|t| no_route_count(&t))
        })
        .flatten();
    let resource_failure = if oom_kills > 0 {
        Some(format!(
            "the kernel killed {oom_kills} process(es) for memory while it ran"
        ))
    } else if fork_limit > 0 {
        Some("it hit the process limit".to_string())
    } else {
        resource_message(&spec.stdout).or_else(|| resource_message(&spec.stderr))
    };
    if killed.is_some() {
        // Killed by a limit: whether the step had started is beside the point.
        return Ok(RunResult {
            exit_code,
            signal,
            wall,
            killed,
            oom_kills,
            net_attempts,
            resource_failure,
        });
    }
    let started = std::fs::read(spec.identity.as_ref().unwrap().join("started"))
        .map_err(io_err("read start sentinel"))?;
    if started != b"1" {
        let stderr = std::fs::read_to_string(&spec.stderr).unwrap_or_default();
        return Err(SandboxError::DidNotStart {
            exit_code,
            stderr: stderr.trim().chars().take(2000).collect(),
        });
    }
    Ok(RunResult {
        exit_code,
        signal,
        wall,
        killed: None,
        oom_kills,
        net_attempts,
        resource_failure,
    })
}

/// Ip OutNoRoutes plus Ip6OutNoRoutes from /proc/net/snmp and snmp6 text.
pub fn no_route_count(text: &str) -> Option<u64> {
    let mut v4 = None;
    let mut header: Option<Vec<&str>> = None;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("Ip: ") {
            let fields: Vec<&str> = rest.split_whitespace().collect();
            match &header {
                None => header = Some(fields),
                Some(h) => {
                    let i = h.iter().position(|f| *f == "OutNoRoutes")?;
                    v4 = fields.get(i).and_then(|n| n.parse::<u64>().ok());
                }
            }
        }
    }
    let v6 = text.lines().find_map(|l| {
        l.strip_prefix("Ip6OutNoRoutes")
            .and_then(|n| n.trim().parse::<u64>().ok())
    })?;
    Some(v4? + v6)
}

/// A message in the step's output saying the machine ran out of something:
/// disk, or the resources to start a process or thread. Matched line by
/// line in the last 1 MiB; deliberately narrow, since a test may print an
/// EAGAIN on purpose.
fn resource_message(path: &Path) -> Option<String> {
    let mut f = File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    use std::io::Seek;
    f.seek(io::SeekFrom::Start(len.saturating_sub(1 << 20)))
        .ok()?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf);
    text.lines().find_map(|l| {
        let low = l.to_ascii_lowercase();
        let why = if low.contains("no space left on device") || low.contains("disk quota exceeded")
        {
            "the disk was full"
        } else if low.contains("fork") && low.contains("resource temporarily unavailable") {
            "a fork failed for lack of resources"
        } else if low.contains("fork") && low.contains("cannot allocate memory") {
            "a fork failed for lack of memory"
        } else if low.contains("failed to create new os thread") {
            "a thread could not be created"
        } else {
            return None;
        };
        Some(format!(
            "{why} (its output says {:?})",
            l.trim().chars().take(160).collect::<String>()
        ))
    })
}

/// A cgroup for one step, under `cgroup_root()`, so the step's own OOM
/// kills and fork-limit hits are counted (memory.events, pids.events).
/// Removed when dropped.
struct StepCgroup {
    dir: PathBuf,
}

/// Where step cgroups go: `/sys/fs/cgroup/quod`, if it exists and
/// hands the memory and pids controllers to its children (the host sets it
/// up: under cgroup v2 a parent holding processes cannot delegate, see
/// scripts/runner/lab.sh). Without it the machine-wide counter is used.
pub fn cgroup_root() -> Option<PathBuf> {
    let root = PathBuf::from("/sys/fs/cgroup/quod");
    let ctl = std::fs::read_to_string(root.join("cgroup.subtree_control")).ok()?;
    (ctl.split_whitespace().any(|c| c == "memory") && ctl.split_whitespace().any(|c| c == "pids"))
        .then_some(root)
}

impl StepCgroup {
    fn create() -> Option<StepCgroup> {
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = cgroup_root()?.join(format!("step-{}-{seq}", std::process::id()));
        std::fs::create_dir(&dir).ok()?;
        if !dir.join("memory.events").is_file() || !dir.join("pids.events").is_file() {
            let _ = std::fs::remove_dir(&dir);
            return None;
        }
        Some(StepCgroup { dir })
    }

    /// The sum of the named counters in an events file; 1 if unreadable.
    fn events(&self, file: &str, names: &[&str]) -> u64 {
        let Ok(text) = std::fs::read_to_string(self.dir.join(file)) else {
            return 1;
        };
        text.lines()
            .filter_map(|l| l.split_once(' '))
            .filter(|(k, _)| names.contains(k))
            .filter_map(|(_, v)| v.trim().parse::<u64>().ok())
            .sum()
    }
}

impl Drop for StepCgroup {
    fn drop(&mut self) {
        for _ in 0..50 {
            if std::fs::remove_dir(&self.dir).is_ok() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        tracing::warn!(cgroup = %self.dir.display(), "step cgroup not removed");
    }
}

/// The kernel's count of OOM kills since boot.
fn oom_kills() -> Option<u64> {
    std::fs::read_to_string("/proc/vmstat")
        .ok()?
        .lines()
        .find_map(|l| l.strip_prefix("oom_kill "))
        .and_then(|n| n.trim().parse().ok())
}

/// What the kernel and CPU show a sandboxed step whatever its root
/// filesystem, as the step key pins it (`quod_key::Platform`). The sandbox
/// inherits this process's affinity mask and auxiliary vector, so this is
/// what the step sees.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostPlatform {
    /// `uname -m`.
    pub machine: String,
    /// `uname -r`, whole.
    pub release: String,
    /// The CPU features user space dispatches on: the distinct
    /// `flags`/`Features` lines of /proc/cpuinfo (each word-sorted), then
    /// the auxv HWCAP words. On aarch64 the HWCAP words are what glibc and
    /// Go read; on x86_64 programs execute CPUID directly, which cannot be
    /// intercepted, and the kernel's flags line is its record of it.
    pub cpu_features: String,
    /// CPUs in the affinity mask (what `nproc` and Go's NumCPU report).
    pub cpus: u32,
    /// AT_PAGESZ.
    pub page_size: u64,
}

/// Probes the platform (see `HostPlatform`).
pub fn host_platform() -> Result<HostPlatform, SandboxError> {
    let (machine, release) = uname()?;
    let status =
        std::fs::read_to_string("/proc/self/status").map_err(io_err("read /proc/self/status"))?;
    let cpus = status
        .lines()
        .find_map(|l| l.strip_prefix("Cpus_allowed_list:"))
        .map(|l| count_cpu_list(l.trim()))
        .ok_or_else(|| SandboxError::Io {
            what: "no Cpus_allowed_list in /proc/self/status".into(),
            source: io::ErrorKind::InvalidData.into(),
        })?;
    let auxv = std::fs::read("/proc/self/auxv").map_err(io_err("read /proc/self/auxv"))?;
    let aux = parse_auxv(&auxv);
    let cpuinfo = std::fs::read_to_string("/proc/cpuinfo").map_err(io_err("read /proc/cpuinfo"))?;
    Ok(HostPlatform {
        machine,
        release,
        cpu_features: cpu_features(&cpuinfo, &aux),
        cpus,
        page_size: aux.get(&AT_PAGESZ).copied().unwrap_or(0),
    })
}

const AT_PAGESZ: u64 = 6;
/// AT_HWCAP, AT_HWCAP2, AT_HWCAP3, AT_HWCAP4 (the last two since Linux 6.14,
/// absent before).
const AT_HWCAPS: [(u64, &str); 4] = [
    (16, "hwcap"),
    (26, "hwcap2"),
    (29, "hwcap3"),
    (30, "hwcap4"),
];

fn uname() -> Result<(String, String), SandboxError> {
    let (machine, release, _) = uname_all()?;
    Ok((machine, release))
}

/// `uname -m`, `-r` and `-v`.
fn uname_all() -> Result<(String, String, String), SandboxError> {
    // SAFETY: utsname is plain old data (arrays of c_char); all zeroes is a
    // valid value.
    #[allow(unsafe_code)] // uname(2) has no safe std wrapper
    let mut u: libc::utsname = unsafe { std::mem::zeroed() };
    // SAFETY: `u` is a valid, writable utsname for the duration of the call.
    #[allow(unsafe_code)]
    let rc = unsafe { libc::uname(&mut u) };
    if rc != 0 {
        return Err(io_err("uname")(io::Error::last_os_error()));
    }
    let field = |f: &[libc::c_char]| {
        // c_char is i8 or u8 by target; its byte either way.
        let bytes: Vec<u8> = f
            .iter()
            .take_while(|c| **c != 0)
            .map(|c| c.to_ne_bytes()[0])
            .collect();
        String::from_utf8_lossy(&bytes).into_owned()
    };
    Ok((field(&u.machine), field(&u.release), field(&u.version)))
}

/// `0-3,8,10-11` -> 7.
fn count_cpu_list(list: &str) -> u32 {
    list.split(',')
        .filter(|r| !r.is_empty())
        .map(|r| match r.split_once('-') {
            Some((a, b)) => match (a.parse::<u32>(), b.parse::<u32>()) {
                (Ok(a), Ok(b)) if b >= a => b - a + 1,
                _ => 0,
            },
            None => u32::from(r.parse::<u32>().is_ok()),
        })
        .sum()
}

/// The auxiliary vector: native-endian (key, value) words up to AT_NULL.
fn parse_auxv(raw: &[u8]) -> std::collections::BTreeMap<u64, u64> {
    const W: usize = std::mem::size_of::<usize>();
    let word = |b: &[u8]| {
        let mut a = [0u8; W];
        a.copy_from_slice(b);
        usize::from_ne_bytes(a) as u64
    };
    raw.as_chunks::<{ 2 * W }>()
        .0
        .iter()
        .map(|c| (word(&c[..W]), word(&c[W..])))
        .take_while(|(k, _)| *k != 0)
        .collect()
}

fn cpu_features(cpuinfo: &str, aux: &std::collections::BTreeMap<u64, u64>) -> String {
    let mut lines: Vec<String> = Vec::new();
    for l in cpuinfo.lines() {
        let Some((name, value)) = l.split_once(':') else {
            continue;
        };
        if matches!(name.trim(), "flags" | "Features") {
            let mut words: Vec<&str> = value.split_whitespace().collect();
            words.sort_unstable();
            words.dedup();
            let line = words.join(" ");
            if !lines.contains(&line) {
                lines.push(line);
            }
        }
    }
    let hwcaps: Vec<String> = AT_HWCAPS
        .iter()
        .filter_map(|(k, name)| aux.get(k).map(|v| format!("{name}={v:#x}")))
        .collect();
    lines.push(hwcaps.join(" "));
    lines.join("; ")
}

/// Makes the child start from `PROCESS_STATE` instead of this process's:
/// bubblewrap and the step inherit umask, rlimits, ignored signals, the
/// signal mask and any descriptor not marked close-on-exec, and a shell
/// cannot reset a signal ignored on entry. Everything is computed before the
/// fork; the child only makes async-signal-safe calls.
fn fix_process_state(cmd: &mut Command) {
    use std::os::unix::process::CommandExt;
    let limits: Vec<(i32, libc::rlimit)> = RLIMITS
        .iter()
        .zip(effective_rlimits())
        .map(|(&(res, ..), (_, soft, hard))| {
            (
                res,
                libc::rlimit {
                    rlim_cur: soft,
                    rlim_max: hard,
                },
            )
        })
        .collect();
    let sigmax = libc::SIGRTMAX();
    let fd_ceiling = effective_fd_ceiling();
    // SAFETY: the closure runs in the forked child before exec and only
    // calls umask, setpriority, setrlimit, rt_sigaction, sigprocmask,
    // close_range and fcntl, all async-signal-safe; it allocates nothing (the vector is
    // built above and only read), and the kernel reads the zeroed sigaction
    // buffer, large enough for every architecture's struct, only during
    // the call.
    #[allow(unsafe_code)] // pre_exec is unsafe; the calls have no safe std wrapper
    unsafe {
        cmd.pre_exec(move || {
            libc::umask(UMASK as libc::mode_t);
            // Nice 0. Lowering a raised nice value needs CAP_SYS_NICE; where
            // that is refused the value stays, and the host class (read in
            // the sandbox) records it.
            libc::setpriority(libc::PRIO_PROCESS, 0, 0);
            for (res, lim) in &limits {
                if libc::setrlimit(*res as _, lim) != 0 {
                    return Err(io::Error::last_os_error());
                }
            }
            // The raw syscall, not libc's sigaction: the C library refuses
            // to touch the signals it reserves (32 and 33 in glibc), and an
            // ignored one survives exec like any other. An all-zero kernel
            // sigaction is SIG_DFL, no flags, empty mask on every layout.
            let dfl = [0u64; 4];
            for sig in 1..=sigmax {
                if sig != libc::SIGKILL && sig != libc::SIGSTOP {
                    libc::syscall(
                        libc::SYS_rt_sigaction,
                        sig as libc::c_long,
                        dfl.as_ptr(),
                        std::ptr::null_mut::<u64>(),
                        8 as libc::c_long,
                    );
                }
            }
            let mut none: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut none);
            if libc::sigprocmask(libc::SIG_SETMASK, &none, std::ptr::null_mut()) != 0 {
                return Err(io::Error::last_os_error());
            }
            // Every descriptor above stderr closes on exec (Rust's own
            // exec-error pipe already does).
            const CLOSE_RANGE_CLOEXEC: libc::c_uint = 1 << 2;
            if libc::syscall(libc::SYS_close_range, 3u32, u32::MAX, CLOSE_RANGE_CLOEXEC) != 0 {
                for fd in 3..fd_ceiling {
                    libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
                }
            }
            Ok(())
        });
    }
}

/// Upper bound on descriptor numbers this process may hold (its NOFILE soft
/// limit, capped), for the fallback without close_range(2).
fn effective_fd_ceiling() -> i32 {
    let mut cur = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `cur` is a valid, writable rlimit for the call.
    #[allow(unsafe_code)]
    let rc = unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut cur) };
    if rc == 0 {
        cur.rlim_cur.min(1 << 20) as i32
    } else {
        1 << 16
    }
}

/// Global sysctls that change what a step can do. Read inside the sandbox
/// (`host_class_inside`), so a sysctl the sandbox's fresh namespaces give
/// their own value (net, IPC, UTS) shows the value the step sees. Those the
/// kernel derives from the amount of RAM are exact, like MemTotal.
pub const SYSCTLS: &[&str] = &[
    "fs/file-max",
    "fs/inotify/max_queued_events",
    "fs/inotify/max_user_instances",
    "fs/inotify/max_user_watches",
    "fs/mount-max",
    "fs/nr_open",
    "fs/pipe-max-size",
    "fs/pipe-user-pages-soft",
    "fs/protected_fifos",
    "fs/protected_hardlinks",
    "fs/protected_regular",
    "fs/protected_symlinks",
    "kernel/apparmor_restrict_unprivileged_userns",
    "kernel/core_pattern",
    "kernel/dmesg_restrict",
    "kernel/io_uring_disabled",
    "kernel/io_uring_group",
    "kernel/kptr_restrict",
    "kernel/overflowgid",
    "kernel/overflowuid",
    "kernel/perf_event_paranoid",
    "kernel/pid_max",
    "kernel/randomize_va_space",
    "kernel/threads-max",
    "kernel/unprivileged_bpf_disabled",
    "kernel/unprivileged_userns_clone",
    "kernel/yama/ptrace_scope",
    "user/max_user_namespaces",
    "vm/max_map_count",
    "vm/mmap_min_addr",
    "vm/nr_hugepages",
    "vm/overcommit_kbytes",
    "vm/overcommit_memory",
    "vm/overcommit_ratio",
    "vm/unprivileged_userfaultfd",
];

/// A whitespace-normalized file, or `-` when it cannot be read.
fn read_norm(p: &str) -> String {
    std::fs::read_to_string(p)
        .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
        .unwrap_or_else(|_| "-".into())
}

/// A security label without SELinux MCS categories (`...:s0:c12,c345` ->
/// `...:s0`): container runtimes draw them at random per container, so they
/// are noise between runs on one host, not a property of it.
fn strip_mcs(label: &str) -> String {
    let parts: Vec<&str> = label.split(':').collect();
    if parts.len() >= 5 && parts[4].starts_with('c') {
        parts[..4].join(":")
    } else {
        label.to_string()
    }
}

/// /proc/cpuinfo without what varies from boot to boot or read to read
/// (x86's current `cpu MHz`, the calibrated `bogomips`).
fn stable_cpuinfo(cpuinfo: &str) -> String {
    cpuinfo
        .lines()
        .filter(|l| {
            let k = l
                .split(':')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase();
            k != "cpu mhz" && k != "bogomips"
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The host class as a step sees it: call this inside the sandbox
/// (`probe_in_sandbox`), with its fresh procfs (not one a container runtime
/// virtualized, such as lxcfs's /proc/meminfo), its fresh namespaces and its
/// fixed process state. (name, value) pairs; see the module docs.
pub fn host_class_inside() -> Result<Vec<(String, String)>, SandboxError> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut put = |k: &str, v: String| out.push((k.to_string(), v));
    let (_, release, version) = uname_all()?;
    put("kernel release", release);
    put("kernel version", version);
    put("kernel cmdline", read_norm("/proc/cmdline"));
    put("domainname", read_norm("/proc/sys/kernel/domainname"));
    put(
        "security context",
        strip_mcs(&read_norm("/proc/self/attr/current")),
    );
    put("personality", read_norm("/proc/self/personality"));
    // The process state as the sandbox hands it over, read by a C program
    // (`probe_in_sandbox` has the shell run cat first): the probe itself is
    // a Rust binary, whose runtime ignores SIGPIPE before `main`.
    let status = std::fs::read_to_string(RECEIVED_STATUS)
        .or_else(|_| std::fs::read_to_string("/proc/self/status"))
        .unwrap_or_default();
    for field in [
        "Seccomp",
        "Seccomp_filters",
        "NoNewPrivs",
        "Umask",
        "SigIgn",
        "SigBlk",
    ] {
        let v = status
            .lines()
            .find_map(|l| l.strip_prefix(&format!("{field}:")))
            .map_or("-".to_string(), |v| v.trim().to_string());
        put(&format!("status {field}"), v);
    }
    let stat = std::fs::read_to_string("/proc/self/stat").unwrap_or_default();
    // Field 19 (nice) counts from the state after the closing parenthesis.
    let nice = stat
        .rsplit_once(')')
        .and_then(|(_, rest)| rest.split_whitespace().nth(16).map(String::from))
        .unwrap_or_else(|| "-".into());
    put("nice", nice);
    let meminfo = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let mem = meminfo
        .lines()
        .find_map(|l| l.strip_prefix("MemTotal:"))
        .map_or("-".to_string(), |v| {
            v.split_whitespace().collect::<Vec<_>>().join(" ")
        });
    put("memory", mem);
    let procstat = std::fs::read_to_string("/proc/stat").unwrap_or_default();
    let online = procstat
        .lines()
        .filter(|l| l.starts_with("cpu") && l[3..].starts_with(|c: char| c.is_ascii_digit()))
        .count();
    put("cpus online", online.to_string());
    let cpuinfo = std::fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    let configured = cpuinfo
        .lines()
        .filter(|l| l.starts_with("processor"))
        .count();
    put("cpus configured", configured.to_string());
    put("cpuinfo", stable_cpuinfo(&cpuinfo));
    for name in SYSCTLS {
        put(
            &format!("sysctl {}", name.replace('/', ".")),
            read_norm(&format!("/proc/sys/{name}")),
        );
    }
    let fds = std::fs::read_dir("/proc/self/fd").map_or(0, |d| d.count());
    put("open fds", fds.to_string());
    for &(res, name, ..) in RLIMITS {
        let mut cur = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        // SAFETY: `cur` is a valid, writable rlimit for the call.
        #[allow(unsafe_code)] // getrlimit(2) has no safe std wrapper
        let rc = unsafe { libc::getrlimit(res as _, &mut cur) };
        let v = |x: u64| {
            if x == libc::RLIM_INFINITY {
                "unlimited".to_string()
            } else {
                x.to_string()
            }
        };
        let val = if rc == 0 {
            format!("{} {}", v(cur.rlim_cur), v(cur.rlim_max))
        } else {
            "-".into()
        };
        put(&format!("rlimit {name}"), val);
    }
    put("process state", PROCESS_STATE.into());
    Ok(out)
}

/// The part of the host class a step cannot read but is subject to, from
/// this process (the step runs in its cgroup, under its kernel's binfmt_misc
/// registrations, on its filesystems, in its bubblewrap): cgroup limits,
/// binfmt_misc registrations, the LSM list, the filesystem types of
/// `filesystems`, the sandbox's version.
pub fn host_class_outside(
    filesystems: &[(&str, &Path)],
) -> Result<Vec<(String, String)>, SandboxError> {
    let mut out: Vec<(String, String)> = Vec::new();
    let (pids, mem) = cgroup_limits();
    out.push(("cgroup pids.max".into(), pids));
    out.push(("cgroup memory.max".into(), mem));
    out.push(("binfmt_misc".into(), binfmt_misc()));
    out.push(("lsm".into(), read_norm("/sys/kernel/security/lsm")));
    for (what, path) in filesystems {
        out.push((format!("fs {what}"), fs_type(path)?));
    }
    let bwrap = which("bwrap").ok_or(SandboxError::NoBwrap)?;
    let v = Command::new(&bwrap)
        .arg("--version")
        .env_clear()
        .output()
        .map_err(io_err("bwrap --version"))?;
    out.push((
        "sandbox".into(),
        String::from_utf8_lossy(&v.stdout).trim().to_string(),
    ));
    Ok(out)
}

/// The tightest `pids.max` and `memory.max` on the way from this process's
/// cgroup (v2) up to the root this process can see: a fork beyond the first
/// fails with EAGAIN, an allocation beyond the second ends in the OOM
/// killer, and the step runs in this cgroup. `max` when unlimited, `-` when
/// not readable (no cgroup v2).
fn cgroup_limits() -> (String, String) {
    let Some(rel) = std::fs::read_to_string("/proc/self/cgroup")
        .ok()
        .and_then(|s| {
            s.lines()
                .find_map(|l| l.strip_prefix("0::").map(String::from))
        })
    else {
        return ("-".into(), "-".into());
    };
    let base = Path::new("/sys/fs/cgroup");
    let mut dir = base.join(rel.trim_start_matches('/'));
    let (mut pids, mut mem): (Option<u64>, Option<u64>) = (None, None);
    let mut seen = false;
    loop {
        for (file, slot) in [("pids.max", &mut pids), ("memory.max", &mut mem)] {
            if let Ok(v) = std::fs::read_to_string(dir.join(file)) {
                seen = true;
                if let Ok(n) = v.trim().parse::<u64>() {
                    *slot = Some(slot.map_or(n, |m: u64| m.min(n)));
                }
            }
        }
        if dir == base || !dir.pop() || !dir.starts_with(base) {
            break;
        }
    }
    let show = |v: Option<u64>| {
        if !seen {
            "-".to_string()
        } else {
            v.map_or("max".into(), |n| n.to_string())
        }
    };
    (show(pids), show(mem))
}

/// The binfmt_misc registrations the kernel applies to a step's execs (a
/// registered emulator runs foreign binaries a host without it refuses):
/// every entry with its contents, from /proc/sys/fs/binfmt_misc, else from
/// a private binfmt_misc mount; `unreadable` when neither works.
fn binfmt_misc() -> String {
    let read_dir = |d: &Path| -> Option<String> {
        if !d.join("status").exists() {
            return None;
        }
        let mut names: Vec<String> = std::fs::read_dir(d)
            .ok()?
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n != "register")
            .collect();
        names.sort();
        Some(
            names
                .iter()
                .map(|n| format!("{n}: {}", read_norm(&d.join(n).to_string_lossy())))
                .collect::<Vec<_>>()
                .join("; "),
        )
    };
    if let Some(s) = read_dir(Path::new("/proc/sys/fs/binfmt_misc")) {
        return s;
    }
    let tmp = std::env::temp_dir().join(format!("quod-binfmt-{}", std::process::id()));
    if std::fs::create_dir_all(&tmp).is_err() {
        return "unreadable".into();
    }
    use std::os::unix::ffi::OsStrExt;
    let (Ok(src), Ok(target)) = (
        std::ffi::CString::new("binfmt_misc"),
        std::ffi::CString::new(tmp.as_os_str().as_bytes()),
    ) else {
        return "unreadable".into();
    };
    // SAFETY: valid C strings for the duration of the call; no data.
    #[allow(unsafe_code)] // mount(2) has no safe std wrapper
    let rc = unsafe {
        libc::mount(
            src.as_ptr(),
            target.as_ptr(),
            src.as_ptr(),
            libc::MS_NOSUID | libc::MS_NODEV | libc::MS_NOEXEC,
            std::ptr::null(),
        )
    };
    let s = if rc == 0 {
        let s = read_dir(&tmp).unwrap_or_else(|| "unreadable".into());
        // SAFETY: a valid C string; the mount was made just above.
        #[allow(unsafe_code)] // umount2(2) has no safe std wrapper
        unsafe {
            libc::umount2(target.as_ptr(), 0);
        }
        s
    } else {
        "unreadable".into()
    };
    let _ = std::fs::remove_dir(&tmp);
    s
}

/// Where `probe_in_sandbox` leaves /proc/self/status as a plain C program
/// in the sandbox sees it (on the sandbox's own /tmp).
pub const RECEIVED_STATUS: &str = "/tmp/quod-received-status";

/// Runs `program args` in a sandbox laid out as a host-root step's (fresh
/// pid, mount, IPC, UTS, net and user namespaces, fresh procfs, uid 1001,
/// the fixed process state), with `program` bound in read-only, and returns
/// its stdout: how the host class is read as a step would see it.
pub fn probe_in_sandbox(program: &Path, args: &[&str]) -> Result<String, SandboxError> {
    let dir = std::env::temp_dir().join(format!(
        "quod-probe-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).map_err(io_err("create probe dir"))?;
    // The binary goes in through the probe dir, not from where it is
    // installed: the sandbox may not be allowed to traverse that path (a
    // home directory of mode 0750, as on GitHub's hosted runners, read by
    // root inside a user namespace). A hard link where it can, else a copy.
    let staged = dir.join("quod-probe");
    if std::fs::hard_link(program, &staged).is_err() {
        std::fs::copy(program, &staged).map_err(io_err("stage the probe binary"))?;
    }
    let mut cmd: Vec<String> = [
        "/bin/sh",
        "-c",
        &format!("cat /proc/self/status > {RECEIVED_STATUS} && exec /quod-probe \"$@\""),
        "sh",
    ]
    .map(String::from)
    .to_vec();
    cmd.extend(args.iter().map(|a| a.to_string()));
    let spec = RunSpec {
        cmd,
        cwd: "/".into(),
        env: vec![("PATH".into(), "/usr/bin:/bin".into())],
        mounts: vec![Mount {
            src: staged,
            dst: "/quod-probe".into(),
            writable: false,
        }],
        egress: false,
        stdout: dir.join("stdout"),
        stderr: dir.join("stderr"),
        echo: false,
        rootfs: None,
        identity: None,
        uid: None,
        gid: None,
        net_probe: false,
    };
    let r = run(&spec);
    let out = std::fs::read_to_string(dir.join("stdout")).unwrap_or_default();
    let err = std::fs::read_to_string(dir.join("stderr")).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&dir);
    let r = r?;
    if r.exit_code != 0 {
        return Err(SandboxError::Io {
            what: format!("host probe exited {}: {}", r.exit_code, err.trim()),
            source: io::ErrorKind::Other.into(),
        });
    }
    Ok(out)
}

/// The filesystem type of `path` (statfs f_type), by name where known.
fn fs_type(path: &Path) -> Result<String, SandboxError> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io_err("statfs")(io::ErrorKind::InvalidInput.into()))?;
    // SAFETY: statfs is plain old data; all zeroes is a valid value.
    #[allow(unsafe_code)]
    let mut st: libc::statfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c` is a valid C string and `st` writable for the call.
    #[allow(unsafe_code)]
    let rc = unsafe { libc::statfs(c.as_ptr(), &mut st) };
    if rc != 0 {
        return Err(io_err(format!("statfs {}", path.display()))(
            io::Error::last_os_error(),
        ));
    }
    let magic = st.f_type as u64;
    Ok(match magic {
        0xEF53 => "ext4".into(),
        0x5846_5342 => "xfs".into(),
        0x9123_683E => "btrfs".into(),
        0x0102_1994 => "tmpfs".into(),
        0x794C_7630 => "overlay".into(),
        0x6573_5546 => "fuse".into(),
        0x2FC1_2FC1 => "zfs".into(),
        0xF2F5_2010 => "f2fs".into(),
        0x6969 => "nfs".into(),
        other => format!("{other:#x}"),
    })
}

fn tee(mut src: impl Read, mut file: File, mut echo: Option<impl Write>) -> io::Result<()> {
    let mut buf = vec![0u8; 64 << 10];
    loop {
        let n = match src.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        file.write_all(&buf[..n])?;
        if let Some(w) = echo.as_mut() {
            // The terminal going away must not lose the capture.
            let _ = w.write_all(&buf[..n]).and_then(|()| w.flush());
        }
    }
    file.sync_data()
}

fn which(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .chain(["/usr/bin".into(), "/bin".into()])
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_lists_count() {
        assert_eq!(count_cpu_list("0-17"), 18);
        assert_eq!(count_cpu_list("0-3,8,10-11"), 7);
        assert_eq!(count_cpu_list("5"), 1);
        assert_eq!(count_cpu_list(""), 0);
        assert_eq!(count_cpu_list("3-1,x"), 0);
    }

    #[test]
    fn auxv_parses_up_to_null() {
        let mut raw = Vec::new();
        for (k, v) in [
            (6usize, 4096usize),
            (16, 0xefff),
            (26, 0x3),
            (0, 0),
            (16, 1),
        ] {
            raw.extend_from_slice(&k.to_ne_bytes());
            raw.extend_from_slice(&v.to_ne_bytes());
        }
        let aux = parse_auxv(&raw);
        assert_eq!(aux.get(&AT_PAGESZ), Some(&4096));
        assert_eq!(aux.get(&16), Some(&0xefff), "entries after AT_NULL ignored");
        assert_eq!(aux.len(), 3);
    }

    #[test]
    fn cpu_features_are_order_independent_and_include_hwcaps() {
        let aux: std::collections::BTreeMap<u64, u64> = [(16, 0xff), (26, 0x2)].into();
        let a = "processor\t: 0\nFeatures\t: fp asimd aes\n\nprocessor\t: 1\nFeatures\t: aes fp asimd\n";
        assert_eq!(cpu_features(a, &aux), "aes asimd fp; hwcap=0xff hwcap2=0x2");
        let x86 =
            "processor\t: 0\nflags\t\t: sse2 fpu avx2\nvmx flags\t: ept\nbugs\t\t: spectre_v1\n";
        assert_eq!(
            cpu_features(x86, &Default::default()),
            "avx2 fpu sse2; ",
            "only the flags line, not vmx flags or bugs"
        );
        let mixed = "flags : a b\nflags : a c\n";
        assert_eq!(cpu_features(mixed, &Default::default()), "a b; a c; ");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn this_host_has_a_platform() {
        let p = host_platform().unwrap();
        assert_eq!(p.machine, std::env::consts::ARCH, "{p:?}");
        assert!(p.release.contains('.') && p.cpus >= 1, "{p:?}");
        assert!(
            p.page_size >= 4096 && p.page_size.is_power_of_two(),
            "{p:?}"
        );
        assert!(p.cpu_features.contains("hwcap="), "{p:?}");
        assert_eq!(host_platform().unwrap(), p, "stable");
    }

    #[test]
    fn labels_lose_their_mcs_categories_and_cpuinfo_its_clocks() {
        assert_eq!(
            strip_mcs("system_u:system_r:container_t:s0:c123,c456"),
            "system_u:system_r:container_t:s0"
        );
        assert_eq!(
            strip_mcs("docker-default (enforce)"),
            "docker-default (enforce)"
        );
        assert_eq!(
            strip_mcs("unconfined_u:unconfined_r:unconfined_t:s0-s0"),
            "unconfined_u:unconfined_r:unconfined_t:s0-s0"
        );
        assert_eq!(strip_mcs("-"), "-");
        let x86 =
            "processor\t: 0\ncpu MHz\t\t: 2994.374\nbogomips\t: 5988.74\nflags\t\t: fpu sse2\n";
        assert_eq!(stable_cpuinfo(x86), "processor\t: 0\nflags\t\t: fpu sse2");
        let arm = "processor\t: 0\nBogoMIPS\t: 48.00\nFeatures\t: fp asimd\n";
        assert_eq!(stable_cpuinfo(arm), "processor\t: 0\nFeatures\t: fp asimd");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_host_class_has_both_halves() {
        let inside = host_class_inside().unwrap();
        let names: Vec<&str> = inside.iter().map(|(k, _)| k.as_str()).collect();
        for k in [
            "kernel release",
            "memory",
            "cpuinfo",
            "status Umask",
            "rlimit NPROC",
            "sysctl kernel.threads-max",
            "sysctl vm.nr_hugepages",
        ] {
            assert!(names.contains(&k), "{k}: {names:?}");
        }
        let mem = &inside.iter().find(|(k, _)| k == "memory").unwrap().1;
        assert!(mem.ends_with(" kB"), "{mem}");
        if which("bwrap").is_some() {
            let outside = host_class_outside(&[("tmp", Path::new("/tmp"))]).unwrap();
            let names: Vec<&str> = outside.iter().map(|(k, _)| k.as_str()).collect();
            for k in [
                "cgroup pids.max",
                "cgroup memory.max",
                "binfmt_misc",
                "lsm",
                "fs tmp",
                "sandbox",
            ] {
                assert!(names.contains(&k), "{k}: {names:?}");
            }
        }
    }

    #[test]
    fn tmp_is_scratch_unless_the_root_is_a_writable_rootfs() {
        let spec = |rootfs: Option<RootFs>| RunSpec {
            cmd: vec!["true".into()],
            cwd: "/workspace".into(),
            env: vec![],
            mounts: vec![],
            egress: false,
            stdout: "/dev/null".into(),
            stderr: "/dev/null".into(),
            echo: false,
            rootfs,
            identity: None,
            uid: None,
            gid: None,
            net_probe: false,
        };
        let has_tmpfs = |s: &RunSpec| {
            bwrap_args(s)
                .windows(2)
                .any(|w| w[0] == "--tmpfs" && w[1] == "/tmp")
        };
        assert!(has_tmpfs(&spec(None)));
        assert!(has_tmpfs(&spec(Some(RootFs {
            dir: "/r".into(),
            writable: false
        }))));
        let rw = spec(Some(RootFs {
            dir: "/r".into(),
            writable: true,
        }));
        assert!(!has_tmpfs(&rw));
        let a = bwrap_args(&rw);
        assert!(a.windows(3).any(|w| w == ["--bind", "/r", "/"]), "{a:?}");
        assert!(
            !a.iter().any(|x| x == "/usr" || x == "--remount-ro"),
            "{a:?}"
        );
        assert!(a.windows(2).any(|w| w == ["--hostname", HOSTNAME]));
    }
}
