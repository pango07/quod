//! Runs a command with a given cwd, env and mounts, and returns its exit code
//! and wall time. stdout and stderr are streamed to files (never buffered
//! whole) and, optionally, to the terminal at the same time.
//!
//! 0a backend: bubblewrap, with the network namespace unshared unless egress
//! is allowed. 0b backend (Firecracker): T15.

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
    /// Set by `run`: where the generated passwd and group files live.
    #[doc(hidden)]
    pub identity: Option<PathBuf>,
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

/// Read-only system paths visible inside the sandbox. The toolchain under
/// /usr is pinned by the step key's image field, not observed.
const SYSTEM_PATHS: &[&str] = &["/usr", "/bin", "/sbin", "/lib", "/lib64", "/etc"];

/// Inside the sandbox: the file whose content proves the step started.
const STARTED: &str = "/.stillpass-started";

/// The uid and gid a step runs as inside the sandbox.
pub const STEP_UID: &str = "1001";

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
        "--die-with-parent",
        "--new-session",
    ]);
    if spec.egress {
        push(&["--share-net"]);
    }
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
    push(&["--dev", "/dev", "--proc", "/proc", "--tmpfs", "/tmp"]);
    for m in &spec.mounts {
        let flag = if m.writable { "--bind" } else { "--ro-bind" };
        push(&[flag, &m.src.to_string_lossy(), &m.dst.to_string_lossy()]);
    }
    if let Some(dir) = &spec.identity {
        // Proof the step started: written by the wrapper right before it
        // execs the step's command.
        push(&["--bind", &dir.join("started").to_string_lossy(), STARTED]);
    }
    // Everything not mounted above lives on bwrap's root tmpfs; freeze it.
    push(&["--remount-ro", "/", "--clearenv"]);
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
