//! The host fingerprint (scripts/host-fingerprint.sh), kept by the
//! identities of everything it reads.
//!
//! The script costs ~0.2 s on a 4-core box and ~0.6 s on GitHub's 2-vCPU
//! runner even with its binary hashes kept (about 40 processes: go, node,
//! python and uv started for their versions, four install trees walked and
//! sorted, every readable file under /etc hashed, dpkg's package list
//! sorted), on every `quod run`, replay or miss: most of a short step's
//! overhead (bench-lite, docs/bench/lite-baseline.md). Its output is a
//! function of files: the toolchains as PATH resolves them and their
//! install trees, /etc, dpkg's database, the C library, the kernel's
//! architecture. So it is kept under `<store>/host-fingerprint-cache` by a
//! validator: the BLAKE3 of the identity (device, inode, size, mtime and
//! ctime to the nanosecond, mode, type, symlink target) of every one of
//! those files, walked in-process (no fork) in parallel, ~20 ms. Writing a
//! file moves its ctime, which nothing can set back; adding, removing or
//! renaming an entry changes its directory's listing; so the kept output is
//! used only while every input is as it was. The output is kept only when
//! no input changed in the 2 s before the walk started (file times are
//! coarse: the racy index rule, as the script's own binary-hash cache
//! does), and never when a toolchain resolves outside its install tree (its
//! GOROOT or stdlib would not be walked) or an input lies on a filesystem
//! whose identities are a server's or a daemon's (NFS, 9p, FUSE, SMB, Ceph,
//! AFS). The validator also covers the script's text, the user it runs as
//! and the boot (the kernel). A kept entry is checked against its content
//! hash before use. What the script reads is listed here; the script says
//! so at its top, and a change to what it reads must change this list.

use std::fs::Metadata;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, bail};
use quod_cas::{Cas, Hash};

/// The script, as Quod runs it.
pub const FINGERPRINT_SH: &str = include_str!("../../../scripts/host-fingerprint.sh");

/// The PATH the script sets for itself (its third line of code).
pub const SCRIPT_PATH: &str =
    "/usr/local/go/bin:/usr/local/node/bin:/usr/local/python/bin:/usr/local/uv/bin:/usr/bin:/bin";

/// The toolchains the script asks for their versions, and the install tree
/// each must resolve into for its output to be kept.
const TOOLS: &[(&str, Option<&str>)] = &[
    ("go", Some("/usr/local/go")),
    ("node", Some("/usr/local/node")),
    ("npm", Some("/usr/local/node")),
    ("python3", Some("/usr/local/python")),
    ("uv", Some("/usr/local/uv")),
    // Its output is the package database's, walked below.
    ("dpkg-query", None),
];

/// The install trees the script lists (and whose pycs it reads).
const TREES: &[&str] = &[
    "/usr/local/go",
    "/usr/local/node",
    "/usr/local/python",
    "/usr/local/uv",
];

/// Under /etc, what the script leaves out (`etc_find`); of the first five
/// it records only the form (`sec_etc_forms`).
const ETC_FORMS: &[&str] = &[
    "/etc/hostname",
    "/etc/hosts",
    "/etc/resolv.conf",
    "/etc/machine-id",
    "/etc/mtab",
];
const ETC_SKIP: &[&str] = &["/etc/passwd", "/etc/group"];
const ETC_SKIP_PREFIX: &[&str] = &["/etc/shadow", "/etc/gshadow"];

/// More files the script's output depends on: dpkg's database (`dpkg-query
/// -W` reads the status file and the journal of a change in progress),
/// the C library `getconf` reports and getconf itself, os-release's target.
const EXTRA_WALKS: &[&str] = &["/var/lib/dpkg/updates"];
const EXTRA_FILES: &[&str] = &[
    "/var/lib/dpkg/status",
    "/usr/bin/getconf",
    "/etc/os-release",
    "/lib/x86_64-linux-gnu/libc.so.6",
    "/lib/aarch64-linux-gnu/libc.so.6",
    "/usr/lib/x86_64-linux-gnu/libc.so.6",
    "/usr/lib/aarch64-linux-gnu/libc.so.6",
    "/lib64/libc.so.6",
    "/usr/lib64/libc.so.6",
    "/lib/libc.so.6",
    "/usr/lib/libc.so.6",
];

/// Identities, as bytes to hash, and the newest time among them.
#[derive(Default)]
struct Ids {
    buf: Vec<u8>,
    /// The newest mtime or ctime seen, in nanoseconds since the epoch.
    newest: i128,
    /// An input lies where identities cannot be trusted.
    untrusted: bool,
}

impl Ids {
    fn line(&mut self, parts: &[&[u8]]) {
        for p in parts {
            self.buf.extend_from_slice(p);
            self.buf.push(0);
        }
        self.buf.push(b'\n');
    }

    fn entry(&mut self, path: &Path, m: &Metadata) {
        let mtime = i128::from(m.mtime()) * 1_000_000_000 + i128::from(m.mtime_nsec());
        let ctime = i128::from(m.ctime()) * 1_000_000_000 + i128::from(m.ctime_nsec());
        self.newest = self.newest.max(mtime).max(ctime);
        let target = if m.file_type().is_symlink() {
            std::fs::read_link(path).unwrap_or_default()
        } else {
            PathBuf::new()
        };
        let id = format!(
            "{} {} {} {} {} {:o}",
            m.dev(),
            m.ino(),
            m.size(),
            mtime,
            ctime,
            m.mode()
        );
        self.line(&[
            path.as_os_str().as_bytes(),
            id.as_bytes(),
            target.as_os_str().as_bytes(),
        ]);
    }

    /// One path, not followed: its identity, or that it is absent.
    fn path(&mut self, path: &Path) {
        match std::fs::symlink_metadata(path) {
            Ok(m) => self.entry(path, &m),
            Err(e) => self.line(&[
                path.as_os_str().as_bytes(),
                format!("{:?}", e.kind()).as_bytes(),
            ]),
        }
    }

    /// One path and, if it is a symlink, where it finally leads.
    fn resolved(&mut self, path: &Path) {
        self.path(path);
        if let Ok(real) = std::fs::canonicalize(path)
            && real != path
        {
            self.path(&real);
        }
    }

    /// Every entry under `root` (itself included), not following symlinks,
    /// in name order; `skip` says which to leave out (and not descend).
    fn walk(&mut self, root: &Path, skip: &dyn Fn(&Path) -> bool) {
        if !root.exists() && std::fs::symlink_metadata(root).is_err() {
            self.path(root);
            return;
        }
        if !quod_sandbox::trusted_identities(root) {
            self.untrusted = true;
        }
        self.walk_in(root, skip);
    }

    fn walk_in(&mut self, dir: &Path, skip: &dyn Fn(&Path) -> bool) {
        let Ok(m) = std::fs::symlink_metadata(dir) else {
            self.path(dir);
            return;
        };
        self.entry(dir, &m);
        if !m.is_dir() {
            return;
        }
        let mut names: Vec<_> = match std::fs::read_dir(dir) {
            Ok(rd) => rd.flatten().map(|e| e.file_name()).collect(),
            Err(e) => {
                self.line(&[
                    dir.as_os_str().as_bytes(),
                    format!("unreadable {:?}", e.kind()).as_bytes(),
                ]);
                return;
            }
        };
        names.sort();
        for n in names {
            let p = dir.join(n);
            if !skip(&p) {
                self.walk_in(&p, skip);
            }
        }
    }
}

fn etc_skip(p: &Path) -> bool {
    let s = p.to_string_lossy();
    ETC_FORMS.iter().chain(ETC_SKIP).any(|x| s == *x)
        || ETC_SKIP_PREFIX.iter().any(|x| s.starts_with(x))
}

/// The first `name` on `path` that is an executable regular file, as
/// `command -v` finds it.
fn which(name: &str, path: &str) -> Option<PathBuf> {
    path.split(':')
        .map(|d| Path::new(d).join(name))
        .find(|p| std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.mode() & 0o111 != 0))
}

/// The validator of the script's output on this host, with the newest
/// input time (ns since the epoch); `None` when the output must not be
/// kept (a toolchain outside its tree, an untrusted filesystem).
fn validator() -> Option<(Hash, i128)> {
    let mut parts: Vec<Ids> = std::thread::scope(|s| {
        let mut handles = Vec::new();
        for t in TREES {
            handles.push(s.spawn(move || {
                let mut ids = Ids::default();
                ids.walk(Path::new(t), &|_| false);
                ids
            }));
        }
        handles.push(s.spawn(|| {
            let mut ids = Ids::default();
            ids.walk(Path::new("/etc"), &etc_skip);
            ids
        }));
        handles
            .into_iter()
            .map(|h| {
                h.join().unwrap_or_else(|_| Ids {
                    untrusted: true,
                    ..Ids::default()
                })
            })
            .collect()
    });
    let mut ids = Ids::default();
    ids.line(&[b"quod host fingerprint validator v1"]);
    ids.line(&[Hash::of(FINGERPRINT_SH.as_bytes()).hex().as_bytes()]);
    ids.line(&[quod_key::profile::HOST_PATH.as_bytes()]);
    // Who runs it (what /etc it can read) and on which boot (the kernel).
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    for l in status.lines() {
        if l.starts_with("Uid:") || l.starts_with("Gid:") || l.starts_with("Groups:") {
            ids.line(&[l.as_bytes()]);
        }
    }
    let boot = std::fs::read("/proc/sys/kernel/random/boot_id").ok()?;
    ids.line(&[b"boot", &boot]);
    // PATH: each directory (an entry added or removed there changes its
    // identity), then each tool as the script finds it.
    for d in SCRIPT_PATH.split(':') {
        ids.path(Path::new(d));
    }
    for (tool, tree) in TOOLS {
        match which(tool, SCRIPT_PATH) {
            Some(p) => {
                let real = std::fs::canonicalize(&p).ok()?;
                if let Some(t) = tree
                    && !real.starts_with(t)
                {
                    return None;
                }
                ids.line(&[
                    tool.as_bytes(),
                    p.as_os_str().as_bytes(),
                    real.as_os_str().as_bytes(),
                ]);
                ids.resolved(&p);
            }
            None => ids.line(&[tool.as_bytes(), b"absent"]),
        }
    }
    for f in EXTRA_FILES {
        ids.resolved(Path::new(f));
    }
    for w in EXTRA_WALKS {
        ids.walk(Path::new(w), &|_| false);
    }
    for f in ETC_FORMS {
        // Only the form: a regular file, or a symlink and its target.
        let p = Path::new(f);
        match std::fs::symlink_metadata(p) {
            Ok(m) if m.file_type().is_symlink() => {
                let t = std::fs::read_link(p).unwrap_or_default();
                ids.line(&[f.as_bytes(), b"->", t.as_os_str().as_bytes()]);
            }
            Ok(m) => ids.line(&[f.as_bytes(), format!("{:?}", m.file_type()).as_bytes()]),
            Err(_) => ids.line(&[f.as_bytes(), b"absent"]),
        }
    }
    for p in parts.drain(..) {
        ids.buf.extend_from_slice(&p.buf);
        ids.newest = ids.newest.max(p.newest);
        ids.untrusted |= p.untrusted;
    }
    if ids.untrusted
        || ["/etc", "/var/lib/dpkg"]
            .iter()
            .any(|p| Path::new(p).exists() && !quod_sandbox::trusted_identities(Path::new(p)))
    {
        return None;
    }
    Some((Hash::of(&ids.buf), ids.newest))
}

/// Runs the script (the toolchain binaries' hashes kept by identity in
/// `cache`, its QUOD_FP_CACHE).
fn run_script(cache: &Path) -> anyhow::Result<Vec<u8>> {
    let out = Command::new("/bin/bash")
        .arg("-c")
        .arg(FINGERPRINT_SH)
        .env_clear()
        .env("PATH", quod_key::profile::HOST_PATH)
        .env("QUOD_FP_CACHE", cache)
        .output()
        .context("run host fingerprint")?;
    if !out.status.success() {
        bail!(
            "host fingerprint failed ({}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(out.stdout)
}

/// A kept output: the BLAKE3 hex of the output, a newline, the output.
fn read_kept(path: &Path) -> Option<Vec<u8>> {
    let b = std::fs::read(path).ok()?;
    let nl = b.iter().position(|c| *c == b'\n')?;
    let (sum, rest) = (&b[..nl], &b[nl + 1..]);
    (sum == Hash::of(rest).hex().as_bytes()).then(|| rest.to_vec())
}

fn keep(path: &Path, out: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    let mut b = Hash::of(out).hex().into_bytes();
    b.push(b'\n');
    b.extend_from_slice(out);
    std::fs::write(&tmp, &b)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

fn now_ns() -> i128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as i128)
}

/// The host fingerprint script's output on this host, kept in the store
/// while every file it reads is as it was (module doc). `QUOD_FP_KEEP=0`
/// runs the script every time.
pub fn fingerprint(cas: &Cas) -> anyhow::Result<Vec<u8>> {
    let cache = cas.dir().join("host-fingerprint-cache");
    if std::env::var("QUOD_FP_KEEP").is_ok_and(|v| v == "0") {
        return run_script(&cache);
    }
    let start = now_ns();
    let Some((v, newest)) = validator() else {
        return run_script(&cache);
    };
    let kept = cache.join(format!("out-{}", v.hex()));
    if let Some(out) = read_kept(&kept) {
        tracing::debug!(target: "quod", validator = %v, "host fingerprint kept");
        return Ok(out);
    }
    let out = run_script(&cache)?;
    let racy = Duration::from_secs(2).as_nanos() as i128;
    if newest + racy < start {
        std::fs::create_dir_all(&cache).ok();
        if let Err(e) = keep(&kept, &out) {
            tracing::warn!(target: "quod", "could not keep the host fingerprint: {e}");
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_script_sets_the_path_this_module_resolves_with() {
        assert!(FINGERPRINT_SH.contains(&format!("export PATH={SCRIPT_PATH}\n")));
        // Every tree the script walks is walked here.
        for t in TREES {
            assert!(FINGERPRINT_SH.contains(&format!("sec_tree {t}")), "{t}");
        }
        assert!(FINGERPRINT_SH.contains("dpkg-query -W"));
    }

    #[test]
    fn a_kept_output_is_checked_against_its_hash() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("out-x");
        keep(&p, b"go absent\nnode absent\n").unwrap();
        assert_eq!(read_kept(&p).unwrap(), b"go absent\nnode absent\n");
        let mut b = std::fs::read(&p).unwrap();
        let last = b.len() - 2;
        b[last] = b'X';
        std::fs::write(&p, &b).unwrap();
        assert_eq!(read_kept(&p), None);
        std::fs::write(&p, b"not a hash").unwrap();
        assert_eq!(read_kept(&p), None);
    }

    /// The validator moves with any write, addition, removal or rename in a
    /// walked tree, and not otherwise.
    #[cfg(target_os = "linux")]
    #[test]
    fn identities_move_with_every_change() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("tree");
        std::fs::create_dir_all(root.join("lib")).unwrap();
        std::fs::write(root.join("lib/a.py"), b"a").unwrap();
        std::os::unix::fs::symlink("a.py", root.join("lib/b.py")).unwrap();
        let ids = || {
            let mut i = Ids::default();
            i.walk(&root, &|_| false);
            Hash::of(&i.buf)
        };
        let first = ids();
        assert_eq!(ids(), first);
        // Same size, same mtime set back: the ctime still moves.
        let m = std::fs::metadata(root.join("lib/a.py")).unwrap();
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(root.join("lib/a.py"), b"b").unwrap();
        let mtime = std::fs::FileTimes::new().set_modified(m.modified().unwrap());
        std::fs::File::options()
            .write(true)
            .open(root.join("lib/a.py"))
            .unwrap()
            .set_times(mtime)
            .unwrap();
        let second = ids();
        assert_ne!(second, first);
        std::fs::write(root.join("lib/c.py"), b"").unwrap();
        let third = ids();
        assert_ne!(third, second);
        std::fs::rename(root.join("lib/c.py"), root.join("lib/d.py")).unwrap();
        assert_ne!(ids(), third);
        let fourth = ids();
        std::fs::remove_file(root.join("lib/b.py")).unwrap();
        std::os::unix::fs::symlink("d.py", root.join("lib/b.py")).unwrap();
        assert_ne!(ids(), fourth);
    }

    /// Kept or not, the output is the script's, byte for byte.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_kept_output_is_the_scripts() {
        let d = tempfile::tempdir().unwrap();
        let cas = Cas::open(d.path().join("cas")).unwrap();
        let plain = run_script(&d.path().join("plain-cache")).unwrap();
        let first = fingerprint(&cas).unwrap();
        let second = fingerprint(&cas).unwrap();
        assert_eq!(first, plain);
        assert_eq!(second, plain);
    }

    #[test]
    fn etc_skips_what_the_script_leaves_out() {
        assert!(etc_skip(Path::new("/etc/hostname")));
        assert!(etc_skip(Path::new("/etc/shadow-")));
        assert!(etc_skip(Path::new("/etc/passwd")));
        assert!(!etc_skip(Path::new("/etc/passwd-")));
        assert!(!etc_skip(Path::new("/etc/gitconfig")));
    }
}
