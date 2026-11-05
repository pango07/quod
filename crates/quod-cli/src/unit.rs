//! `quod unit`: a package script (`npm run build`, `npm run test:release`)
//! answered from a record when every input it read is unchanged
//! (docs/through-the-wall.md, Addendum B1).
//!
//! A unit's record is split into the **trace** (what was read and written,
//! as path events) and the **keyer** (how each read is keyed): the trace
//! comes from `strace -f --seccomp-bpf` on the first witness; the keys are
//! `quod vitest`'s (`vitest::state`): `f:` content, `e:` existence and type,
//! `p:` an installed package (its package.json and lockfile entry), `d:` a
//! directory's names, `envall:` the environment but its per-run names, and
//! `host:` the kernel and the image. The hit path hashes those keys and
//! nothing else: no FUSE, no import, no process.
//!
//! Witnesses. The first runs traced; the second runs plain (no trace) and
//! must agree on the exit code, stdout, stderr (durations masked) and every
//! output; only then is the record trusted. Both run in bubblewrap with no
//! network but loopback, their own pid namespace and a private `/tmp`, at
//! the checkout's real path. A record is made only from a run that exited 0.
//!
//! Fails closed (the unit always runs, plain, and says why): any io_uring
//! ring (strace refuses `io_uring_setup` with ENOSYS, so libuv uses epoll;
//! a ring that is set up anyway is unobservable I/O), a connection or
//! datagram to anything but loopback (or to port 53), a write outside the
//! checkout and the scratch places, a write into `.git`, a file read and
//! then rewritten (its earlier content is not in the trace), a directory
//! listed and written into with an entry whose earlier existence is
//! unknown, and a vitest run inside the unit (vitest has its own recorder).
//!
//! The ptrace pointer race (another thread rewriting a path between
//! strace's read and the kernel's) is not a recording concern: test and
//! build code is not adversarial, and the JS hooks `quod vitest` relies on
//! are far easier to bypass. Same standard.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::Instant;

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::vitest;

/// Bumped when a record made before would be read differently.
const VERSION: u32 = 1;
/// Records kept per unit (newest first).
const KEEP: usize = 3;
/// Disagreeing second witnesses before a unit always runs.
const MAX_DISAGREE: u32 = 2;

#[derive(clap::Args)]
pub struct UnitArgs {
    /// Where the units' ledgers and output blobs live.
    #[arg(long)]
    dir: PathBuf,
    /// The checkout: paths under it are keyed relative to it, and only
    /// writes under it are outputs.
    #[arg(long)]
    root: PathBuf,
    /// Created when this run changed a ledger (the action saves them then).
    #[arg(long)]
    mark: Option<PathBuf>,
    /// Replay trusted records only; never witness.
    #[arg(long)]
    no_record: bool,
    /// The command, as the package manager was started.
    #[arg(last = true, required = true)]
    argv: Vec<String>,
}

#[derive(Serialize, Deserialize, Default)]
struct Ledger {
    version: u32,
    argv: Vec<String>,
    cwd: String,
    /// Never recorded, and why.
    #[serde(default)]
    always: Option<String>,
    #[serde(default)]
    disagree: u32,
    records: Vec<Rec>,
}

#[derive(Serialize, Deserialize, Clone)]
struct Rec {
    /// (key, its state when recorded)
    keys: Vec<(String, String)>,
    witnesses: u32,
    code: i32,
    stdout: String,
    stderr: String,
    outputs: Vec<Out>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
struct Out {
    /// relative to the root
    path: String,
    /// file | dir | link | absent
    kind: String,
    #[serde(default)]
    blob: Option<String>,
    #[serde(default)]
    mode: u32,
    #[serde(default)]
    target: Option<String>,
}

pub fn cli(a: UnitArgs) -> anyhow::Result<u8> {
    let t0 = Instant::now();
    let root = fs::canonicalize(&a.root).unwrap_or(a.root.clone());
    let cwd = std::env::current_dir()?;
    let rel_cwd = cwd
        .strip_prefix(&root)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| cwd.display().to_string());
    let id = {
        let mut h = blake3::Hasher::new();
        for x in &a.argv {
            h.update(x.as_bytes());
            h.update(b"\0");
        }
        h.update(rel_cwd.as_bytes());
        h.finalize().to_hex()[..16].to_string()
    };
    let name = label(&a.argv);
    let udir = a.dir.join(&id);
    let lpath = udir.join("ledger.json");
    let blobs = a.dir.join("blobs");
    let mut ledger: Ledger = fs::read(&lpath)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .filter(|l: &Ledger| l.version == VERSION)
        .unwrap_or_else(|| Ledger {
            version: VERSION,
            argv: a.argv.clone(),
            cwd: rel_cwd.clone(),
            ..Default::default()
        });

    if let Some(why) = &ledger.always {
        eprintln!("quod: {name} always runs ({why})");
        return Ok(code_u8(run_direct(&a.argv)?));
    }

    // The hit path: the newest record whose every key still holds.
    let mut matched = None;
    let mut changed: Vec<String> = Vec::new();
    for (i, r) in ledger.records.iter().enumerate() {
        let ks: Vec<&(String, String)> = r.keys.iter().collect();
        let now = vitest::par_map(&ks, |(k, _)| key_state(&root, k));
        if ks.iter().zip(&now).all(|((_, s), n)| s == n) {
            matched = Some(i);
            break;
        }
        if i == 0 {
            changed = ks
                .iter()
                .zip(&now)
                .filter(|((_, s), n)| s != *n)
                .map(|((k, _), _)| k.clone())
                .collect();
        }
    }
    if matched.is_none() && !changed.is_empty() {
        let n = changed.len();
        changed.truncate(6);
        eprintln!(
            "quod: {name} runs: {n} of its recorded inputs changed ({}{})",
            changed.join(", "),
            if n > 6 { ", ..." } else { "" }
        );
    }
    let checked = t0.elapsed().as_secs_f64();
    if let Some(i) = matched
        && ledger.records[i].witnesses >= 2
    {
        let r = &ledger.records[i];
        restore(&root, &blobs, &r.outputs)?;
        let out = fs::read(blobs.join(&r.stdout)).unwrap_or_default();
        let err = fs::read(blobs.join(&r.stderr)).unwrap_or_default();
        std::io::stdout().write_all(&out)?;
        std::io::stdout().flush()?;
        std::io::stderr().write_all(&err)?;
        eprintln!(
            "quod: {name} answered from its record ({} inputs unchanged, {} outputs restored; {:.2} s, checked in {:.2} s)",
            r.keys.len(),
            r.outputs.len(),
            t0.elapsed().as_secs_f64(),
            checked
        );
        return Ok(code_u8(r.code));
    }
    if a.no_record {
        return Ok(code_u8(run_direct(&a.argv)?));
    }
    let Some(bwrap) = sandbox_ok() else {
        eprintln!("quod: {name} runs plain (bubblewrap unusable here: not recorded)");
        return Ok(code_u8(run_direct(&a.argv)?));
    };

    if let Some(i) = matched {
        // The second witness: plain (no trace), same inputs by construction.
        let (code, out, err) = run_captured(&sandboxed(&bwrap, &cwd, &a.argv, None), &[])?;
        let rec = &ledger.records[i];
        let now: Vec<Out> = rec
            .outputs
            .iter()
            .map(|o| snapshot(&root, &o.path, None))
            .collect();
        let same_outs = now.len() == rec.outputs.len()
            && now.iter().zip(&rec.outputs).all(|(a, b)| same_out(a, b));
        let rec_out = fs::read(blobs.join(&rec.stdout)).unwrap_or_default();
        let rec_err = fs::read(blobs.join(&rec.stderr)).unwrap_or_default();
        let agree = code == rec.code
            && same_outs
            && mask(&out) == mask(&rec_out)
            && mask(&err) == mask(&rec_err);
        if agree {
            ledger.records[i].witnesses = 2;
            eprintln!("quod: {name}: the second witness agrees; its record is trusted");
        } else {
            let what = if code != rec.code {
                format!("exit {code}, recorded {}", rec.code)
            } else if !same_outs {
                "its outputs".to_string()
            } else {
                "its output text".to_string()
            };
            ledger.records.remove(i);
            ledger.disagree += 1;
            if ledger.disagree >= MAX_DISAGREE {
                ledger.always = Some(format!("its witnesses disagree: {what}"));
            }
            eprintln!("quod: {name}: the second witness disagrees ({what}); not trusted");
        }
        save(&lpath, &ledger, a.mark.as_deref())?;
        return Ok(code_u8(code));
    }

    // The first witness: traced.
    if !strace_ok() {
        eprintln!("quod: {name} runs plain (no strace here: not recorded)");
        return Ok(code_u8(run_direct(&a.argv)?));
    }
    fs::create_dir_all(&udir)?;
    let trace = udir.join("trace.log");
    let _ = fs::remove_file(&trace);
    let cmd = sandboxed(&bwrap, &cwd, &a.argv, Some(&trace));
    let (code, out, err) = run_captured(&cmd, &[("UV_USE_IO_URING", "0")])?;
    let t1 = Instant::now();
    if code != 0 {
        let _ = fs::remove_file(&trace);
        return Ok(code_u8(code));
    }
    let text = fs::read(&trace).unwrap_or_default();
    let _ = fs::remove_file(&trace);
    let ignore = Ignore::new(&root, &a.dir);
    let made = parse(&String::from_utf8_lossy(&text), &cwd).and_then(|t| keyer(&root, &t, &ignore));
    match made {
        Err(why) => {
            eprintln!("quod: {name} always runs from now on: {why}");
            ledger.always = Some(why);
            ledger.records.clear();
        }
        Ok((keys, writes)) => {
            let mut outputs = Vec::new();
            for w in &writes {
                outputs.push(snapshot(&root, w, Some(&blobs)));
            }
            let mut ks: Vec<(String, String)> = keys
                .into_iter()
                .map(|(k, pre)| {
                    let s = pre.unwrap_or_else(|| key_state(&root, &k));
                    (k, s)
                })
                .collect();
            for k in ["envall:", "host:", "f:/imagegeneration/imagedata.json"] {
                ks.push((k.to_string(), key_state(&root, k)));
            }
            ks.sort();
            ks.dedup();
            let rec = Rec {
                keys: ks,
                witnesses: 1,
                code,
                stdout: put_blob(&blobs, &out)?,
                stderr: put_blob(&blobs, &err)?,
                outputs,
            };
            eprintln!(
                "quod: {name} recorded (first witness: {} inputs, {} outputs; keyed in {:.2} s)",
                rec.keys.len(),
                rec.outputs.len(),
                t1.elapsed().as_secs_f64()
            );
            ledger.records.insert(0, rec);
            ledger.records.truncate(KEEP);
        }
    }
    save(&lpath, &ledger, a.mark.as_deref())?;
    Ok(code_u8(code))
}

fn label(argv: &[String]) -> String {
    // node .../npm-cli.js run build -> `npm run build`
    let rest: Vec<&str> = argv
        .iter()
        .skip_while(|a| !a.ends_with("-cli.js"))
        .skip(1)
        .map(String::as_str)
        .collect();
    if rest.is_empty() {
        format!("`{}`", argv.join(" "))
    } else {
        format!("`npm {}`", rest.join(" "))
    }
}

fn code_u8(c: i32) -> u8 {
    u8::try_from(c).unwrap_or(1)
}

fn save(p: &Path, l: &Ledger, mark: Option<&Path>) -> anyhow::Result<()> {
    fs::create_dir_all(p.parent().unwrap())?;
    let tmp = p.with_extension("tmp");
    fs::write(&tmp, serde_json::to_vec(l)?)?;
    fs::rename(&tmp, p)?;
    if let Some(m) = mark {
        let _ = fs::write(m, "");
    }
    Ok(())
}

fn put_blob(dir: &Path, b: &[u8]) -> anyhow::Result<String> {
    let h = blake3::hash(b).to_hex().to_string();
    let p = dir.join(&h);
    if !p.exists() {
        fs::create_dir_all(dir)?;
        let tmp = dir.join(format!("{h}.tmp"));
        fs::write(&tmp, b)?;
        fs::rename(&tmp, &p)?;
    }
    Ok(h)
}

/// A key's state now. `d:` is a directory's names and types; the rest are
/// `quod vitest`'s.
fn key_state(root: &Path, k: &str) -> String {
    let (kind, p) = k.split_once(':').unwrap_or((k, ""));
    match kind {
        "d" => listing(&abs(root, p), &[], &[]),
        "host" => host(),
        _ => vitest::state(root, k, None, 0, &std::sync::OnceLock::new(), None).0,
    }
}

fn abs(root: &Path, p: &str) -> PathBuf {
    if p.starts_with('/') {
        PathBuf::from(p)
    } else {
        root.join(p)
    }
}

/// The kernel, the architecture and the CPU count (the image is keyed by
/// ImageOS, ImageVersion (envall:) and /imagegeneration/imagedata.json).
fn host() -> String {
    let rel = fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default();
    let n = std::thread::available_parallelism().map_or(0, |n| n.get());
    format!("{}-{}-{n}", rel.trim(), std::env::consts::ARCH)
}

/// A directory's names and kinds, as it was before the unit ran: `add` the
/// names the unit removed, drop the names it made.
fn listing(p: &Path, add: &[(String, char)], drop: &[String]) -> String {
    let Ok(rd) = fs::read_dir(p) else {
        return if p.exists() { "notdir" } else { "absent" }.into();
    };
    let mut names: Vec<String> = rd
        .flatten()
        .filter(|e| !drop.iter().any(|d| e.file_name().to_string_lossy() == *d))
        .map(|e| {
            let kind = match e.file_type() {
                Ok(t) if t.is_dir() => 'd',
                Ok(t) if t.is_symlink() => 'l',
                Ok(t) if t.is_file() => 'f',
                _ => 'o',
            };
            format!("{}\0{kind}", e.file_name().to_string_lossy())
        })
        .collect();
    for (n, k) in add {
        names.push(format!("{n}\0{k}"));
    }
    names.sort_unstable();
    names.dedup();
    blake3::hash(names.join("\n").as_bytes()).to_hex()[..32].to_string()
}

fn snapshot(root: &Path, rel: &str, blobs: Option<&Path>) -> Out {
    let p = root.join(rel);
    let mut o = Out {
        path: rel.to_string(),
        kind: "absent".into(),
        blob: None,
        mode: 0,
        target: None,
    };
    let Ok(m) = fs::symlink_metadata(&p) else {
        return o;
    };
    o.mode = m.permissions().mode() & 0o7777;
    if m.file_type().is_symlink() {
        o.kind = "link".into();
        o.target = fs::read_link(&p).ok().map(|t| t.display().to_string());
    } else if m.is_dir() {
        o.kind = "dir".into();
    } else if m.is_file() {
        o.kind = "file".into();
        let b = fs::read(&p).unwrap_or_default();
        o.blob = Some(match blobs {
            Some(d) => put_blob(d, &b).unwrap_or_default(),
            None => blake3::hash(&b).to_hex().to_string(),
        });
    } else {
        o.kind = "other".into();
    }
    o
}

fn same_out(a: &Out, b: &Out) -> bool {
    a.kind == b.kind && a.blob == b.blob && a.target == b.target && a.mode == b.mode
}

/// The outputs, as the first witness left them: directories first.
fn restore(root: &Path, blobs: &Path, outs: &[Out]) -> anyhow::Result<()> {
    let mut v: Vec<&Out> = outs.iter().collect();
    v.sort_by_key(|o| (o.kind != "dir", o.path.matches('/').count(), o.path.clone()));
    for o in v {
        let p = root.join(&o.path);
        match o.kind.as_str() {
            "dir" => {
                fs::create_dir_all(&p)?;
                let _ = fs::set_permissions(&p, fs::Permissions::from_mode(o.mode));
            }
            "file" => {
                let b = o.blob.as_deref().context("a file output without content")?;
                if let Some(d) = p.parent() {
                    fs::create_dir_all(d)?;
                }
                if fs::symlink_metadata(&p).is_ok_and(|m| m.is_dir()) {
                    fs::remove_dir_all(&p)?;
                }
                let _ = fs::remove_file(&p);
                fs::copy(blobs.join(b), &p).with_context(|| format!("restoring {}", o.path))?;
                fs::set_permissions(&p, fs::Permissions::from_mode(o.mode))?;
            }
            "link" => {
                if let Some(d) = p.parent() {
                    fs::create_dir_all(d)?;
                }
                let _ = fs::remove_file(&p);
                std::os::unix::fs::symlink(o.target.as_deref().unwrap_or(""), &p)?;
            }
            "absent" => match fs::symlink_metadata(&p) {
                Ok(m) if m.is_dir() => fs::remove_dir_all(&p)?,
                Ok(_) => fs::remove_file(&p)?,
                Err(_) => {}
            },
            _ => {}
        }
    }
    Ok(())
}

/// Masks durations (`(12.3ms)`, `duration_ms 812.4`, `Done in 45ms`, `1.2s`)
/// for comparing two witnesses' text; the replay prints the first's as is.
fn mask(b: &[u8]) -> String {
    let s = String::from_utf8_lossy(b);
    let mut out = String::with_capacity(s.len());
    let c: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < c.len() {
        if c[i].is_ascii_digit() && (i == 0 || !c[i - 1].is_alphanumeric()) {
            let mut j = i;
            while j < c.len() && (c[j].is_ascii_digit() || c[j] == '.') {
                j += 1;
            }
            let unit = if c.get(j) == Some(&'m') && c.get(j + 1) == Some(&'s') {
                j + 2
            } else if c.get(j) == Some(&'s') {
                j + 1
            } else {
                0
            };
            let after = c.get(unit).is_none_or(|x| !x.is_alphanumeric());
            if unit > 0 && after {
                out.push('#');
                i = unit;
                continue;
            }
            // duration_ms 812.4
            if out.ends_with("duration_ms ") {
                out.push('#');
                i = j;
                continue;
            }
            out.extend(&c[i..j]);
            i = j;
            continue;
        }
        out.push(c[i]);
        i += 1;
    }
    out
}

// ---- running ----

fn exit_code(st: ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt;
    st.code().unwrap_or_else(|| 128 + st.signal().unwrap_or(0))
}

fn run_direct(argv: &[String]) -> anyhow::Result<i32> {
    let st = Command::new(&argv[0])
        .args(&argv[1..])
        .status()
        .with_context(|| format!("running {}", argv[0]))?;
    Ok(exit_code(st))
}

/// Runs `cmd`, its stdout and stderr shown as they come and kept.
fn run_captured(cmd: &[String], env: &[(&str, &str)]) -> anyhow::Result<(i32, Vec<u8>, Vec<u8>)> {
    let mut c = Command::new(&cmd[0]);
    c.args(&cmd[1..])
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in env {
        c.env(k, v);
    }
    let mut child = c.spawn().with_context(|| format!("running {}", cmd[0]))?;
    fn tee<R: Read + Send + 'static>(mut r: R, out: bool) -> std::thread::JoinHandle<Vec<u8>> {
        std::thread::spawn(move || {
            let mut all = Vec::new();
            let mut buf = [0u8; 65536];
            while let Ok(n) = r.read(&mut buf) {
                if n == 0 {
                    break;
                }
                all.extend_from_slice(&buf[..n]);
                if out {
                    let mut o = std::io::stdout().lock();
                    let _ = o.write_all(&buf[..n]);
                    let _ = o.flush();
                } else {
                    let _ = std::io::stderr().write_all(&buf[..n]);
                }
            }
            all
        })
    }
    let o = tee(child.stdout.take().unwrap(), true);
    let e = tee(child.stderr.take().unwrap(), false);
    let st = child.wait()?;
    let out = o.join().unwrap_or_default();
    let err = e.join().unwrap_or_default();
    Ok((exit_code(st), out, err))
}

fn sandbox_ok() -> Option<String> {
    let ok = Command::new("bwrap")
        .args(SANDBOX)
        .args(["--", "true"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    ok.then(|| "bwrap".to_string())
}

fn strace_ok() -> bool {
    Command::new("strace")
        .arg("-V")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// No network but loopback, a pid namespace, a private /tmp; the rest of
/// the host as is, at its real paths.
const SANDBOX: &[&str] = &[
    "--dev-bind",
    "/",
    "/",
    "--tmpfs",
    "/tmp",
    "--proc",
    "/proc",
    "--unshare-net",
    "--unshare-pid",
    "--die-with-parent",
];

const TRACE_SET: &str = "trace=%file,%process,getdents64,getdents,%network,io_uring_setup,io_uring_enter,io_uring_register";

fn sandboxed(bwrap: &str, cwd: &Path, argv: &[String], trace: Option<&Path>) -> Vec<String> {
    let mut v: Vec<String> = vec![bwrap.into()];
    v.extend(SANDBOX.iter().map(|s| s.to_string()));
    v.extend(["--chdir".into(), cwd.display().to_string(), "--".into()]);
    if let Some(t) = trace {
        v.extend(
            [
                "strace",
                "-f",
                "--seccomp-bpf",
                "-qq",
                "-yy",
                "-s",
                "4096",
                "-e",
                TRACE_SET,
                "-e",
                "inject=io_uring_setup:error=ENOSYS",
                "-o",
            ]
            .iter()
            .map(|s| s.to_string()),
        );
        v.push(t.display().to_string());
        v.push("--".into());
    }
    v.extend(argv.iter().cloned());
    v
}

// ---- the trace ----

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Op {
    /// opened for reading, or executed
    Read,
    /// stat, access, readlink, xattr, a watch: existence and type
    Stat,
    /// getdents
    List,
    /// mkdir, O_CREAT|O_EXCL, symlink, link, mknod: absent before, if it worked
    Create,
    /// O_TRUNC, or O_CREAT without O_EXCL, or a rename's target: content
    /// independent of what was there
    Trunc,
    /// written in place, chmod, utimes, truncate(2): depends on what was there
    Modify,
    /// unlink, rmdir
    Delete,
    /// a rename's source
    Move,
}

#[derive(Debug, Clone)]
struct Ev {
    path: String,
    op: Op,
    ok: bool,
    /// for a successful stat: "file" | "dir" | "link" | "other", when shown
    kind: Option<&'static str>,
}

#[derive(Debug, Default)]
struct Trace {
    evs: Vec<Ev>,
}

/// Parses `strace -f -yy -s 4096 -o` output into path events, absolute,
/// with each process's working directory followed across chdir and clone.
fn parse(text: &str, cwd0: &Path) -> Result<Trace, String> {
    // Join `<unfinished ...>` with its `<... resumed>`.
    let mut pending: HashMap<u32, String> = HashMap::new();
    let mut lines: Vec<(u32, String)> = Vec::new();
    for l in text.lines() {
        let Some((pid, rest)) = l.split_once(' ') else {
            continue;
        };
        let Ok(pid) = pid.parse::<u32>() else {
            continue;
        };
        let rest = rest.trim_start();
        if rest.starts_with("+++") || rest.starts_with("---") {
            continue;
        }
        if let Some(head) = rest.strip_suffix(" <unfinished ...>") {
            pending.insert(pid, head.to_string());
            continue;
        }
        if rest.starts_with("<... ") {
            let Some(at) = rest.find(" resumed>") else {
                continue;
            };
            let tail = &rest[at + " resumed>".len()..];
            let head = pending.remove(&pid).unwrap_or_default();
            lines.push((pid, format!("{head}{tail}")));
            continue;
        }
        lines.push((pid, rest.to_string()));
    }

    // Who cloned whom (a child's first line can come before its parent's
    // clone returns), and whether they share a working directory.
    let mut parent: HashMap<u32, (u32, bool)> = HashMap::new();
    for (pid, l) in &lines {
        let name = l.split('(').next().unwrap_or("");
        if matches!(name, "clone" | "clone3" | "fork" | "vfork")
            && let Some((_, ret)) = ret_of(l)
            && let Ok(child) = ret.trim().parse::<u32>()
        {
            parent.insert(child, (*pid, l.contains("CLONE_FS")));
        }
    }
    // Working directories, by group (CLONE_FS shares one).
    let mut group: HashMap<u32, u32> = HashMap::new();
    let mut cwd: HashMap<u32, String> = HashMap::new();
    let first = lines.first().map(|x| x.0);
    let mut t = Trace::default();
    for (pid, l) in &lines {
        let g = match group.get(pid) {
            Some(g) => *g,
            None => {
                let g = match parent.get(pid) {
                    Some((pp, true)) => *group.get(pp).unwrap_or(pp),
                    Some((pp, false)) => {
                        let pg = *group.get(pp).unwrap_or(pp);
                        let c = cwd
                            .get(&pg)
                            .cloned()
                            .unwrap_or_else(|| cwd0.display().to_string());
                        cwd.insert(*pid, c);
                        *pid
                    }
                    None => {
                        if Some(*pid) != first && !parent.is_empty() {
                            // A process the trace never saw start.
                            return Err(format!("the trace lost process {pid}'s parent"));
                        }
                        cwd.insert(*pid, cwd0.display().to_string());
                        *pid
                    }
                };
                group.insert(*pid, g);
                g
            }
        };
        let here = cwd
            .get(&g)
            .cloned()
            .unwrap_or_else(|| cwd0.display().to_string());
        let Some(open) = l.find('(') else {
            continue;
        };
        let name = &l[..open];
        let Some((end, ret)) = ret_of(l) else {
            continue;
        };
        if end <= open {
            continue;
        }
        let args_s = &l[open + 1..end];
        let args = split_args(args_s);
        let ok = !ret.starts_with('-') && !ret.starts_with('?');
        let ok = ok || (name == "execve" && ret.starts_with('?'));
        let at = |dirfd: &str, p: &str| -> String { join(&base(dirfd, &here), p) };
        let path0 = || args.first().and_then(|a| unquote(a));
        let mut push = |path: String, op: Op, ok: bool, kind: Option<&'static str>| {
            t.evs.push(Ev { path, op, ok, kind });
        };
        match name {
            "io_uring_setup" => {
                if ok {
                    return Err("it set up an io_uring ring (its I/O is not in the trace)".into());
                }
            }
            "io_uring_enter" | "io_uring_register" => {
                if ok {
                    return Err("it used an io_uring ring".into());
                }
            }
            "chdir" => {
                if ok && let Some(p) = path0() {
                    let n = join(&here, &p);
                    push(n.clone(), Op::Stat, true, Some("dir"));
                    cwd.insert(g, n);
                }
            }
            "fchdir" => {
                if ok && let Some(p) = args.first().and_then(|a| fd_path(a)) {
                    cwd.insert(g, p);
                }
            }
            "execve" | "execveat" => {
                let p = if name == "execve" {
                    path0().map(|p| join(&here, &p))
                } else {
                    args.get(1)
                        .and_then(|a| unquote(a))
                        .map(|p| at(&args[0], &p))
                };
                if let Some(p) = p {
                    if ok && (p.ends_with("/vitest/vitest.mjs") || p.ends_with("/.bin/vitest")) {
                        return Err("it runs vitest (vitest records itself)".into());
                    }
                    push(p, Op::Read, ok, None);
                }
            }
            "open" | "creat" | "openat" | "openat2" => {
                let (p, flags) = match name {
                    "open" => (path0().map(|p| join(&here, &p)), args.get(1).cloned()),
                    "creat" => (
                        path0().map(|p| join(&here, &p)),
                        Some("O_CREAT|O_WRONLY|O_TRUNC".into()),
                    ),
                    _ => (
                        args.get(1)
                            .and_then(|a| unquote(a))
                            .map(|p| at(&args[0], &p)),
                        args.get(2).cloned(),
                    ),
                };
                let Some(p) = p else { continue };
                let f = flags.unwrap_or_default();
                let write = f.contains("O_WRONLY") || f.contains("O_RDWR");
                // O_CREAT alone (append, write in place) keeps what was there.
                let op = if f.contains("O_CREAT") && f.contains("O_EXCL") {
                    Op::Create
                } else if f.contains("O_TRUNC") {
                    Op::Trunc
                } else if f.contains("O_CREAT") || write {
                    Op::Modify
                } else {
                    Op::Read
                };
                // O_DIRECTORY opens are lookups; their listing is a getdents.
                let op = if op == Op::Read && f.contains("O_DIRECTORY") {
                    Op::Stat
                } else {
                    op
                };
                let kind = (op == Op::Stat && ok).then_some("dir");
                push(p, op, ok, kind);
            }
            "stat" | "lstat" | "stat64" | "lstat64" | "access" | "readlink" | "statfs"
            | "getxattr" | "lgetxattr" | "listxattr" | "llistxattr" => {
                if let Some(p) = path0() {
                    let kind = stat_kind(&args);
                    push(join(&here, &p), Op::Stat, ok, kind);
                }
            }
            "newfstatat" | "fstatat64" | "statx" | "faccessat" | "faccessat2" | "readlinkat" => {
                let Some(p) = args.get(1).and_then(|a| unquote(a)) else {
                    continue;
                };
                if p.is_empty() {
                    continue; // AT_EMPTY_PATH: the fd itself, opened already
                }
                let kind = stat_kind(&args);
                push(at(&args[0], &p), Op::Stat, ok, kind);
            }
            "inotify_add_watch" => {
                if let Some(p) = args.get(1).and_then(|a| unquote(a)) {
                    push(join(&here, &p), Op::Stat, ok, None);
                }
            }
            "getdents64" | "getdents" => {
                if let Some(p) = args.first().and_then(|a| fd_path(a)) {
                    push(p, Op::List, ok, None);
                }
            }
            "mkdir" => {
                if let Some(p) = path0() {
                    let p = join(&here, &p);
                    if ok {
                        push(p, Op::Create, true, None);
                    } else if ret.contains("EEXIST") {
                        push(p, Op::Stat, true, None);
                    } else {
                        push(p, Op::Stat, false, None);
                    }
                }
            }
            "mkdirat" => {
                if let Some(p) = args.get(1).and_then(|a| unquote(a)) {
                    let p = at(&args[0], &p);
                    if ok {
                        push(p, Op::Create, true, None);
                    } else {
                        push(p, Op::Stat, ret.contains("EEXIST"), None);
                    }
                }
            }
            "symlink" | "link" => {
                if let Some(p) = args.get(1).and_then(|a| unquote(a)) {
                    push(join(&here, &p), Op::Create, ok, None);
                }
            }
            "symlinkat" => {
                if let (Some(d), Some(p)) = (args.get(1), args.get(2).and_then(|a| unquote(a))) {
                    push(at(d, &p), Op::Create, ok, None);
                }
            }
            "linkat" => {
                if let (Some(d), Some(p)) = (args.get(2), args.get(3).and_then(|a| unquote(a))) {
                    push(at(d, &p), Op::Create, ok, None);
                }
            }
            "mknod" | "mknodat" => {
                return Err("it makes device nodes or fifos".into());
            }
            "unlink" | "rmdir" => {
                if let Some(p) = path0() {
                    push(join(&here, &p), Op::Delete, ok, None);
                }
            }
            "unlinkat" => {
                if let Some(p) = args.get(1).and_then(|a| unquote(a)) {
                    push(at(&args[0], &p), Op::Delete, ok, None);
                }
            }
            "rename" => {
                if let (Some(a), Some(b)) = (path0(), args.get(1).and_then(|a| unquote(a))) {
                    push(join(&here, &a), Op::Move, ok, None);
                    push(join(&here, &b), Op::Trunc, ok, None);
                }
            }
            "renameat" | "renameat2" => {
                if let (Some(a), Some(b)) = (
                    args.get(1).and_then(|a| unquote(a)),
                    args.get(3).and_then(|a| unquote(a)),
                ) {
                    push(at(&args[0], &a), Op::Move, ok, None);
                    push(at(&args[2], &b), Op::Trunc, ok, None);
                }
            }
            "truncate" | "chmod" | "chown" | "lchown" | "utime" | "utimes" | "setxattr"
            | "lsetxattr" | "removexattr" | "lremovexattr" => {
                if let Some(p) = path0() {
                    push(join(&here, &p), Op::Modify, ok, None);
                }
            }
            "fchmodat" | "fchmodat2" | "fchownat" | "utimensat" | "futimesat" => {
                let Some(p) = args.get(1).and_then(|a| unquote(a)) else {
                    continue;
                };
                if p.is_empty() || args[1] == "NULL" {
                    continue; // on an open fd: that file's open is the write
                }
                push(at(&args[0], &p), Op::Modify, ok, None);
            }
            "connect" | "sendto" | "sendmsg" | "bind" => {
                if let Some(why) = network(name, args_s, ok) {
                    return Err(why);
                }
                if let Some(p) = unix_path(args_s) {
                    push(join(&here, &p), Op::Stat, ok, None);
                }
            }
            _ => {}
        }
    }
    Ok(t)
}

/// ` = <ret>` at the end of a complete line.
fn ret_of(l: &str) -> Option<(usize, &str)> {
    // strace pads the return value into a column: `)              = 18`.
    let mut upto = l.len();
    while let Some(eq) = l[..upto].rfind(" = ") {
        let before = l[..eq].trim_end_matches(' ');
        if before.ends_with(')') {
            return Some((before.len() - 1, &l[eq + 3..]));
        }
        upto = eq;
    }
    None
}

/// The type a successful stat showed, if it did.
fn stat_kind(args: &[String]) -> Option<&'static str> {
    for a in args {
        if a.contains("S_IFREG") {
            return Some("file");
        }
        if a.contains("S_IFDIR") {
            return Some("dir");
        }
        if a.contains("S_IFLNK") {
            return Some("link");
        }
        if a.contains("st_mode=") || a.contains("stx_mode=") {
            return Some("other");
        }
    }
    None
}

fn network(name: &str, args: &str, ok: bool) -> Option<String> {
    if name == "bind" {
        return None;
    }
    let inet = args.contains("sa_family=AF_INET");
    if !inet {
        if name == "connect" && ok && args.contains("sa_family=AF_UNIX") {
            let p = unix_path(args).unwrap_or_default();
            if !(p.starts_with("/tmp/") || p.is_empty()) {
                return Some(format!("it talks to a local service ({p})"));
            }
        }
        return None;
    }
    let addr = between(args, "inet_addr(\"", "\"")
        .or_else(|| between(args, "inet_pton(AF_INET6, \"", "\""))
        .unwrap_or_default();
    let port = between(args, "htons(", ")").unwrap_or_default();
    let loopback = addr.starts_with("127.") || addr == "::1" || addr.starts_with("::ffff:127.");
    if !loopback || port == "53" {
        return Some(format!("it uses the network ({addr}:{port})"));
    }
    None
}

fn unix_path(args: &str) -> Option<String> {
    let s = between(args, "sun_path=\"", "\"")?;
    (!s.starts_with('@')).then_some(s)
}

fn between(s: &str, a: &str, b: &str) -> Option<String> {
    let i = s.find(a)? + a.len();
    let j = s[i..].find(b)? + i;
    Some(s[i..j].to_string())
}

/// Splits strace's argument list at top-level commas.
fn split_args(s: &str) -> Vec<String> {
    let mut v = Vec::new();
    let mut cur = String::new();
    let mut depth = 0i32;
    let mut q = false;
    let mut esc = false;
    for ch in s.chars() {
        if q {
            cur.push(ch);
            if esc {
                esc = false;
            } else if ch == '\\' {
                esc = true;
            } else if ch == '"' {
                q = false;
            }
            continue;
        }
        match ch {
            '"' => {
                q = true;
                cur.push(ch);
            }
            '{' | '[' | '(' | '<' => {
                depth += 1;
                cur.push(ch);
            }
            '}' | ']' | ')' | '>' => {
                depth -= 1;
                cur.push(ch);
            }
            ',' if depth == 0 => {
                v.push(cur.trim().to_string());
                cur.clear();
            }
            _ => cur.push(ch),
        }
    }
    if !cur.trim().is_empty() {
        v.push(cur.trim().to_string());
    }
    v
}

/// A quoted C string, unescaped. `"..."...` (truncated) is refused.
fn unquote(a: &str) -> Option<String> {
    let s = a.strip_prefix('"')?;
    let mut out: Vec<u8> = Vec::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'"' => {
                if b.get(i + 1..i + 4) == Some(b"...") {
                    return None;
                }
                return Some(String::from_utf8_lossy(&out).into_owned());
            }
            b'\\' => {
                i += 1;
                let c = *b.get(i)?;
                match c {
                    b'n' => out.push(b'\n'),
                    b't' => out.push(b'\t'),
                    b'r' => out.push(b'\r'),
                    b'v' => out.push(0x0b),
                    b'f' => out.push(0x0c),
                    b'x' => {
                        let h = std::str::from_utf8(b.get(i + 1..i + 3)?).ok()?;
                        out.push(u8::from_str_radix(h, 16).ok()?);
                        i += 2;
                    }
                    b'0'..=b'7' => {
                        let mut n = 0u32;
                        let mut k = 0;
                        while k < 3 && i < b.len() && (b'0'..=b'7').contains(&b[i]) {
                            n = n * 8 + u32::from(b[i] - b'0');
                            i += 1;
                            k += 1;
                        }
                        out.push(n as u8);
                        continue;
                    }
                    other => out.push(other),
                }
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    None
}

/// `3</some/dir>` -> `/some/dir`.
fn fd_path(a: &str) -> Option<String> {
    let i = a.find('<')?;
    let p = a[i + 1..].strip_suffix('>')?;
    p.starts_with('/').then(|| p.to_string())
}

fn base(dirfd: &str, cwd: &str) -> String {
    if dirfd == "AT_FDCWD" {
        cwd.to_string()
    } else {
        fd_path(dirfd).unwrap_or_else(|| cwd.to_string())
    }
}

/// `base` joined with `p`, `.` and repeated slashes dropped; `..` kept (the
/// kernel resolves it when the key is hashed, through any symlink).
fn join(base: &str, p: &str) -> String {
    let full = if p.starts_with('/') {
        p.to_string()
    } else {
        format!("{base}/{p}")
    };
    let mut parts: Vec<&str> = Vec::new();
    for c in full.split('/') {
        if c.is_empty() || c == "." {
            continue;
        }
        parts.push(c);
    }
    format!("/{}", parts.join("/"))
}

// ---- the keyer ----

/// What is neither an input nor an output: the kernel's views, the private
/// /tmp, npm's own logs, Quod's own state.
struct Ignore {
    prefixes: Vec<String>,
}

impl Ignore {
    fn new(root: &Path, dir: &Path) -> Self {
        let mut prefixes: Vec<String> = ["/proc", "/sys", "/dev", "/tmp", "/run/user"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        if let Some(h) = std::env::var_os("HOME") {
            prefixes.push(Path::new(&h).join(".npm/_logs").display().to_string());
        }
        let d = fs::canonicalize(dir).unwrap_or(dir.to_path_buf());
        prefixes.push(d.display().to_string());
        let _ = root;
        Ignore { prefixes }
    }
    fn hit(&self, p: &str) -> bool {
        self.prefixes
            .iter()
            .any(|x| p == x || p.starts_with(&format!("{x}/")))
    }
}

/// The installed package a path is in: `.../node_modules/<name>` or
/// `.../node_modules/@scope/<name>` (not `.bin`, `.cache`, ...).
fn package_root(p: &str) -> Option<String> {
    if p.split('/').any(|c| c == "..") {
        return None;
    }
    let i = p.rfind("/node_modules/")?;
    let rest = &p[i + "/node_modules/".len()..];
    let mut it = rest.split('/');
    let first = it.next()?;
    if first.is_empty() || first.starts_with('.') {
        return None;
    }
    let name = if first.starts_with('@') {
        format!("{first}/{}", it.next()?)
    } else {
        first.to_string()
    };
    Some(format!("{}/node_modules/{name}", &p[..i]))
}

/// The trace's keys (with the state the trace itself proves, for paths the
/// unit later changed) and its outputs (relative to the root).
#[allow(clippy::type_complexity)]
fn keyer(
    root: &Path,
    t: &Trace,
    ignore: &Ignore,
) -> Result<(Vec<(String, Option<String>)>, Vec<String>), String> {
    let r = root.display().to_string();
    let under = |p: &str| p == r || p.starts_with(&format!("{r}/"));
    let rel = |p: &str| -> String {
        if p == r {
            ".".into()
        } else if under(p) {
            p[r.len() + 1..].to_string()
        } else {
            p.to_string()
        }
    };
    let is_write = |op: Op| {
        matches!(
            op,
            Op::Create | Op::Trunc | Op::Modify | Op::Delete | Op::Move
        )
    };
    // Per path, its events in order.
    let mut by: BTreeMap<&str, Vec<&Ev>> = BTreeMap::new();
    for e in &t.evs {
        if ignore.hit(&e.path) {
            continue;
        }
        by.entry(e.path.as_str()).or_default().push(e);
    }
    // Paths the unit changed, and what they were before.
    let mut written: BTreeMap<&str, Pre> = BTreeMap::new();
    for (p, evs) in &by {
        if !evs.iter().any(|e| is_write(e.op) && e.ok) {
            continue;
        }
        if !under(p) {
            return Err(format!("it writes outside the checkout ({p})"));
        }
        let rp = rel(p);
        if rp == ".git" || rp.starts_with(".git/") {
            return Err(format!("it writes into .git ({rp})"));
        }
        let first = evs[0];
        let pre = match (first.op, first.ok) {
            (Op::Create, true) => Pre::Absent,
            (Op::Create, false) => Pre::Unknown,
            (Op::Trunc, _) => Pre::Unknown,
            (Op::Delete, true) => Pre::Was(first.kind.unwrap_or("file")),
            // a link's or a special file's earlier form is not a plain kind
            (Op::Stat, true) if matches!(first.kind, Some("link" | "other")) => Pre::Unknown,
            (Op::Read | Op::Stat | Op::List | Op::Delete | Op::Modify | Op::Move, false) => {
                Pre::Absent
            }
            (Op::Stat, true) => match first.kind {
                Some(k) => Pre::Was(k),
                None => Pre::Unknown,
            },
            (Op::List, true) => Pre::Was("dir"),
            (Op::Read, true) => {
                return Err(format!("it reads {} and then rewrites it", rel(p)));
            }
            (Op::Modify, true) => {
                return Err(format!("it changes {} in place", rel(p)));
            }
            (Op::Move, true) => {
                return Err(format!("it moves {} away", rel(p)));
            }
        };
        written.insert(*p, pre);
    }
    // Directories the unit made: everything under them is its own.
    let made: Vec<&str> = written
        .iter()
        .filter(|(_, pre)| matches!(pre, Pre::Absent))
        .map(|(p, _)| *p)
        .collect();
    let own = |p: &str| {
        made.iter()
            .any(|m| p.len() > m.len() && p.starts_with(m) && p.as_bytes()[m.len()] == b'/')
    };
    let parent = |p: &str| -> String { p.rsplit_once('/').map_or("/", |x| x.0).to_string() };

    let mut keys: BTreeMap<String, Option<String>> = BTreeMap::new();
    let mut key = |k: String, pre: Option<String>| {
        keys.entry(k).or_insert(pre);
    };
    for (p, evs) in &by {
        if own(p) {
            continue;
        }
        if let Some(pre) = written.get(p) {
            let st = match pre {
                Pre::Absent => Some("absent"),
                Pre::Was(k) => Some(*k),
                Pre::Unknown => None,
            };
            if let Some(s) = st {
                key(format!("e:{}", rel(p)), Some(s.to_string()));
            }
            // A listing of a directory it made or changed, before the change.
            if evs.iter().any(|e| e.op == Op::List && e.ok) && !matches!(pre, Pre::Absent) {
                list_key(p, &written, &rel, &parent, &mut key)?;
            }
            continue;
        }
        if let Some(pkg) = package_root(p)
            && !written.keys().any(|w| w.starts_with(&format!("{pkg}/")))
        {
            key(format!("p:{}", rel(&pkg)), None);
            continue;
        }
        for e in evs {
            match (e.op, e.ok) {
                (Op::Read, true) => key(format!("f:{}", rel(p)), None),
                (Op::Stat, true) if e.kind == Some("file") => key(format!("f:{}", rel(p)), None),
                (Op::List, true) => list_key(p, &written, &rel, &parent, &mut key)?,
                _ => key(format!("e:{}", rel(p)), None),
            }
        }
    }
    let mut outs: Vec<String> = written.keys().map(|p| rel(p)).collect();
    outs.sort();
    Ok((keys.into_iter().collect(), outs))
}

/// `d:` for a listed directory, as it was before the unit: names it made
/// left out, names it removed put back; an entry whose earlier existence
/// the trace cannot tell fails closed.
fn list_key(
    dir: &str,
    written: &BTreeMap<&str, Pre>,
    rel: &dyn Fn(&str) -> String,
    parent: &dyn Fn(&str) -> String,
    key: &mut dyn FnMut(String, Option<String>),
) -> Result<(), String> {
    let mut add: Vec<(String, char)> = Vec::new();
    let mut drop: Vec<String> = Vec::new();
    for (w, pre) in written {
        if parent(w) != dir {
            continue;
        }
        let name = w.rsplit('/').next().unwrap_or("").to_string();
        match pre {
            Pre::Absent => drop.push(name),
            Pre::Was(k) => add.push((
                name,
                match *k {
                    "dir" => 'd',
                    "link" => 'l',
                    "file" => 'f',
                    _ => 'o',
                },
            )),
            Pre::Unknown => {
                return Err(format!(
                    "it lists {} and writes {} there, which may or may not have been there before",
                    rel(dir),
                    rel(w)
                ));
            }
        }
    }
    if add.is_empty() && drop.is_empty() {
        key(format!("d:{}", rel(dir)), None);
    } else {
        // the pre-run listing, fixed now: the unit's own entries undone
        let s = listing(Path::new(dir), &add, &drop);
        key(format!("d:{}", rel(dir)), Some(s));
    }
    Ok(())
}

/// What a path the unit changed was before it ran, as the trace shows.
enum Pre {
    Absent,
    Was(&'static str),
    Unknown,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::type_complexity)]
    fn keys_of(
        root: &Path,
        text: &str,
    ) -> Result<(Vec<(String, Option<String>)>, Vec<String>), String> {
        let t = parse(text, root)?;
        keyer(
            root,
            &t,
            &Ignore::new(root, Path::new("/nonexistent-quod-dir")),
        )
    }

    #[test]
    fn strings_unescape_and_args_split() {
        assert_eq!(
            unquote(r#""/a b/\"q\"\x41\101\n""#).as_deref(),
            Some("/a b/\"q\"AA\n")
        );
        assert_eq!(unquote(r#""/long"..."#), None);
        let a = split_args(r#"AT_FDCWD, "/x, y", {st_mode=S_IFREG|0644, st_size=3}, 3</d, e>"#);
        assert_eq!(a.len(), 4, "{a:?}");
        assert_eq!(fd_path("3</d, e>").as_deref(), Some("/d, e"));
        assert_eq!(join("/w/r", "./a//b/../c"), "/w/r/a/b/../c");
    }

    #[test]
    fn a_trace_keys_reads_by_kind_and_follows_working_directories() {
        let t = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(t.path()).unwrap();
        let r = root.display();
        let text = format!(
            r#"100 openat(AT_FDCWD, "src/a.ts", O_RDONLY|O_CLOEXEC) = 3<{r}/src/a.ts>
100 newfstatat(AT_FDCWD, "{r}/missing.ts", 0x7ffc, 0) = -1 ENOENT (No such file or directory)
100 statx(AT_FDCWD, "{r}/node_modules/typescript/lib/tsc.js", AT_STATX_SYNC_AS_STAT, STATX_ALL, {{stx_mask=STATX_BASIC_STATS, stx_mode=S_IFREG|0644}}) = 0
100 openat(AT_FDCWD, "{r}/node_modules/@scope/pkg/index.js", O_RDONLY) = 3<{r}/node_modules/@scope/pkg/index.js>
100 getdents64(4<{r}/src>, 0x5 /* 3 entries */, 32768) = 80
100 clone3({{flags=CLONE_VFORK|CLONE_VM, exit_signal=SIGCHLD}}, 88 <unfinished ...>
101 chdir("sub") = 0
101   openat(AT_FDCWD<{r}/sub>, "x.txt", O_RDONLY)  = 3<{r}/sub/x.txt>
100   <... clone3 resumed>)              = 101
101 execve("/usr/bin/git", ["git"], 0x1 /* 3 vars */) = 0
100 openat(AT_FDCWD, "/usr/lib/libz.so", O_RDONLY|O_CLOEXEC) = 3</usr/lib/libz.so>
100 openat(AT_FDCWD, "/proc/self/maps", O_RDONLY) = 3</proc/self/maps>
100 openat(AT_FDCWD, "/tmp/scratch", O_WRONLY|O_CREAT|O_TRUNC, 0644) = 3</tmp/scratch>
100 io_uring_setup(64, 0x7ffc) = -1 ENOSYS (Function not implemented) (INJECTED)
100 +++ exited with 0 +++
"#
        );
        let (keys, outs) = keys_of(&root, &text).unwrap();
        let ks: Vec<&str> = keys.iter().map(|(k, _)| k.as_str()).collect();
        for want in [
            "f:src/a.ts",
            "e:missing.ts",
            "p:node_modules/typescript",
            "p:node_modules/@scope/pkg",
            "d:src",
            "f:sub/x.txt",
            "f:/usr/bin/git",
            "f:/usr/lib/libz.so",
        ] {
            assert!(ks.contains(&want), "{want} not in {ks:?}");
        }
        assert!(
            !ks.iter().any(|k| k.contains("/proc") || k.contains("/tmp")),
            "{ks:?}"
        );
        assert!(outs.is_empty(), "{outs:?}");
    }

    #[test]
    fn outputs_their_earlier_state_and_what_fails_closed() {
        let t = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(t.path()).unwrap();
        let r = root.display();
        fs::create_dir_all(root.join("dist")).unwrap();
        fs::write(root.join("dist/a.js"), "x").unwrap();
        fs::write(root.join("keep.txt"), "k").unwrap();
        // made dist/ (absent before), wrote into it, listed the root (which
        // now has dist in it): the root's listing is keyed as before.
        let text = format!(
            r#"1 getdents64(3<{r}>, 0x1 /* 4 entries */, 32768) = 96
1 newfstatat(AT_FDCWD, "{r}/dist", 0x1, 0) = -1 ENOENT (No such file or directory)
1 mkdir("{r}/dist", 0777) = 0
1 openat(AT_FDCWD, "{r}/dist/a.js", O_WRONLY|O_CREAT|O_TRUNC|O_CLOEXEC, 0666) = 3<{r}/dist/a.js>
"#
        );
        let (keys, outs) = keys_of(&root, &text).unwrap();
        assert_eq!(outs, vec!["dist".to_string(), "dist/a.js".to_string()]);
        let get = |k: &str| keys.iter().find(|x| x.0 == k).map(|x| x.1.clone());
        assert_eq!(get("e:dist"), Some(Some("absent".into())));
        let before = get("d:.").unwrap().unwrap();
        fs::remove_dir_all(root.join("dist")).unwrap();
        assert_eq!(
            key_state(&root, "d:."),
            before,
            "the root as it was before the unit"
        );
        assert!(get("f:dist/a.js").is_none());

        for (text, why) in [
            (format!("1 openat(AT_FDCWD, \"{r}/keep.txt\", O_RDONLY) = 3\n1 openat(AT_FDCWD, \"{r}/keep.txt\", O_WRONLY|O_TRUNC) = 3\n"), "rewrites"),
            ("1 openat(AT_FDCWD, \"/home/x\", O_WRONLY|O_CREAT|O_TRUNC, 0666) = 3\n".to_string(), "outside the checkout"),
            (format!("1 openat(AT_FDCWD, \"{r}/.git/index\", O_WRONLY|O_CREAT|O_TRUNC, 0666) = 3\n"), ".git"),
            ("1 io_uring_setup(64, 0x1) = 7<anon_inode:[io_uring]>\n".to_string(), "io_uring"),
            ("1 connect(3<TCP:[1]>, {sa_family=AF_INET, sin_port=htons(443), sin_addr=inet_addr(\"140.82.112.3\")}, 16) = -1 ENETUNREACH (Network is unreachable)\n".to_string(), "network"),
            ("1 connect(3<UDP:[1]>, {sa_family=AF_INET, sin_port=htons(53), sin_addr=inet_addr(\"127.0.0.53\")}, 16) = 0\n".to_string(), "network"),
            (format!("1 getdents64(3<{r}>, 0x1, 32768) = 96\n1 openat(AT_FDCWD, \"{r}/new.txt\", O_WRONLY|O_CREAT|O_TRUNC, 0666) = 3\n"), "may or may not"),
            (format!("1 execve(\"{r}/node_modules/vitest/vitest.mjs\", [], 0x1) = 0\n"), "vitest"),
        ] {
            let e = keys_of(&root, &text).unwrap_err();
            assert!(e.contains(why), "{why}: {e}");
        }
        // loopback is fine
        let ok = "1 connect(3<TCP:[1]>, {sa_family=AF_INET, sin_port=htons(4000), sin_addr=inet_addr(\"127.0.0.1\")}, 16) = 0\n";
        assert!(keys_of(&root, ok).is_ok());
    }

    #[test]
    fn durations_are_masked_for_agreement() {
        let a = "✔ adds (1.234ms)\nℹ duration_ms 812.5\n⚡ Done in 45ms\nok 3 tests\n";
        let b = "✔ adds (0.9ms)\nℹ duration_ms 799\n⚡ Done in 51ms\nok 3 tests\n";
        assert_eq!(mask(a.as_bytes()), mask(b.as_bytes()));
        assert_ne!(mask(b"ok 3 tests"), mask(b"ok 4 tests"));
    }

    #[test]
    fn package_roots() {
        assert_eq!(
            package_root("/w/node_modules/a/b.js").as_deref(),
            Some("/w/node_modules/a")
        );
        assert_eq!(
            package_root("/w/node_modules/@s/a/x/y").as_deref(),
            Some("/w/node_modules/@s/a")
        );
        assert_eq!(
            package_root("/w/node_modules/a/node_modules/b/c").as_deref(),
            Some("/w/node_modules/a/node_modules/b")
        );
        assert_eq!(package_root("/w/node_modules/.bin/tsc"), None);
        assert_eq!(package_root("/w/node_modules/a/../b"), None);
    }
}
