//! Runs a command with a given cwd, env and mounts, and returns its exit code
//! and wall time. stdout and stderr are streamed to files (never buffered
//! whole) and, optionally, to the terminal at the same time.
//!
//! Backend: bubblewrap, with the network namespace unshared unless egress is
//! allowed. The root is either a pinned rootfs (`RunSpec::rootfs`: a
//! directory bound at `/`, typically a per-step overlay over a sealed
//! content-addressed snapshot, nothing of the host visible) or, as the
//! fallback, the host's own system paths bound read-only.
//!
//! In rootfs mode the step sees no host file at all: `/` is the rootfs,
//! `/dev` a fresh tmpfs with bubblewrap's standard nodes, `/proc` a procfs of
//! the step's own pid namespace, `/sys` the rootfs's empty directory, `/tmp`
//! part of the root (so what a step leaves there is an output), the
//! hostname `stillpass`, the user `runner` (uid 1001) owning every file of
//! the rootfs. What it can still learn about the host, through the kernel:
//! - pinned by the key (`host_platform`): `uname -m`, kernel major.minor,
//!   the CPU features user space dispatches on, the CPU count (affinity),
//!   the page size;
//! - not pinned, left to audit like the clock: the kernel's patch level
//!   and build string (`uname -r`/`-v`, /proc/version, /proc/sys/kernel),
//!   /proc/cmdline, CPU model, frequency and topology (/proc/cpuinfo,
//!   /proc/stat), memory size and load (/proc/meminfo, /proc/loadavg,
//!   /proc/pressure), uptime and boot id, /proc/self/mountinfo (names the
//!   run directory), free space on /workspace, sysctls under /proc/sys, the
//!   clock, pids and randomness.
//!
//! A change to what the sandbox shows a step (its mounts, /dev, identity,
//! hostname) is a change of environment that no input records: it ships
//! with a step key version bump.

use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
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
}

/// A directory to use as the step's `/`. It must already hold the mount
/// points the sandbox needs: `dev`, `proc`, `tmp`, the destination of every
/// mount (`/workspace`), and `etc/passwd` and `etc/group` (replaced by the
/// step identity); a missing one would be created in it. A sealed rootfs
/// has them all.
#[derive(Clone, Debug)]
pub struct RootFs {
    pub dir: PathBuf,
    /// Bind read-write: `dir` is a per-step overlay whose upper dir captures
    /// everything the step writes outside the workspace, /tmp included.
    /// Read-only otherwise (a shared snapshot must never be bound
    /// writable: the step runs as the caller's real uid, which mode bits do
    /// not stop), with a tmpfs on /tmp and every other write failing.
    pub writable: bool,
}

const PASSWD: &str =
    "root:x:0:0:root:/root:/bin/sh\nrunner:x:1001:1001:runner:/workspace/.stillpass/home:/bin/sh\n";
const GROUP: &str = "root:x:0:\nrunner:x:1001:\n";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunResult {
    pub exit_code: i32,
    /// The signal that killed the step, if any (exit_code is then 128+sig).
    pub signal: Option<i32>,
    pub wall: Duration,
}

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
const STARTED: &str = "/dev/.stillpass-started";

/// The uid and gid a step runs as inside the sandbox.
pub const STEP_UID: &str = "1001";

/// The uid the sandbox maps to the step user (bubblewrap maps the caller's
/// real uid): files it owns belong to `runner` inside, any other uid's to
/// nobody.
pub fn caller_uid() -> u32 {
    // SAFETY: getuid(2) takes no arguments, cannot fail and touches no memory.
    #[allow(unsafe_code)] // no safe std wrapper for getuid
    unsafe {
        libc::getuid()
    }
}

/// The hostname every step sees. `--unshare-all` gives the sandbox its own
/// UTS namespace, but the name in it is inherited from the host unless set.
pub const HOSTNAME: &str = "stillpass";

/// The bubblewrap argument list for `spec` (exposed for tests).
pub fn bwrap_args(spec: &RunSpec) -> Vec<String> {
    let mut a: Vec<String> = Vec::new();
    let mut push = |xs: &[&str]| a.extend(xs.iter().map(|s| s.to_string()));
    // Steps run unprivileged, as on a GitHub runner: root would bypass the
    // permission checks some test suites exercise.
    push(&[
        "--unshare-all",
        "--unshare-user",
        "--uid",
        STEP_UID,
        "--gid",
        STEP_UID,
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
    if let Some(dir) = &spec.identity {
        // A fixed passwd and group for the step user, independent of the host.
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
        let wrapper = format!("printf 1 > {STARTED} && exec \"$@\"");
        push(&["/bin/sh", "-c", &wrapper, "sh"]);
    }
    a.extend(spec.cmd.iter().cloned());
    a
}

/// Runs `spec` under bubblewrap.
pub fn run(spec: &RunSpec) -> Result<RunResult, SandboxError> {
    let bwrap = which("bwrap").ok_or(SandboxError::NoBwrap)?;
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let ident = std::env::temp_dir().join(format!("stillpass-ident-{}-{seq}", std::process::id()));
    std::fs::create_dir_all(&ident).map_err(io_err("create identity dir"))?;
    std::fs::write(ident.join("passwd"), PASSWD).map_err(io_err("write passwd"))?;
    std::fs::write(ident.join("group"), GROUP).map_err(io_err("write group"))?;
    std::fs::write(ident.join("started"), "").map_err(io_err("write start sentinel"))?;
    let result = run_inner(spec, bwrap, &ident);
    let _ = std::fs::remove_dir_all(&ident);
    result
}

fn run_inner(spec: &RunSpec, bwrap: PathBuf, ident: &Path) -> Result<RunResult, SandboxError> {
    let ident = ident.to_path_buf();
    let spec = &RunSpec {
        identity: Some(ident),
        ..spec.clone()
    };
    let mut cmd = Command::new(bwrap);
    cmd.args(bwrap_args(spec))
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
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
    let status = child.wait().map_err(io_err("wait for bwrap"))?;
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
    })
}

/// What the kernel and CPU show a sandboxed step whatever its root
/// filesystem, as the step key pins it (`sp_key::Platform`). The sandbox
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
    Ok((field(&u.machine), field(&u.release)))
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
