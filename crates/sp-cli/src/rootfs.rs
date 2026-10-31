//! Pinned root filesystems: the environment a step runs in, as content.
//!
//! A rootfs is imported once (`import`) from a container filesystem export
//! (`docker export` of an image pulled by OCI digest, as a tar or an
//! unpacked directory): every file becomes a blob in the store and the tree
//! becomes a document whose BLAKE3 is the rootfs digest, `b3:...`. That
//! digest is what the step key's `image` field pins; two hosts that imported
//! the same image compute the same key. The OCI digest the image was pulled
//! by is recorded beside it and resolves to it (`resolve`), so a runner that
//! knows only the image reference can find the proof before unpacking.
//!
//! At run time `open` materializes the tree once per store as a sealed
//! snapshot (`crate::snapshot`: built under a temporary name, made durable,
//! marked complete, shared-locked while in use, garbage collected), the
//! rootfs under its `root/` directory so the snapshot's own marker is not
//! part of what the step sees, and `RootOverlay::mount` puts a per-step
//! overlay over it, so the step may write anywhere (/tmp included) and the
//! overlay's upper dir is diffed into the result's `root_outputs` exactly
//! like the workspace overlay is diffed into `outputs`. The rootfs lower is
//! pinned by digest, so nothing read from it needs recording; the workspace
//! stays the observed FUSE tree.
//!
//! Every directory of a rootfs snapshot is 0755 and every file has its tree
//! mode, all owned by the user who built it, which the sandbox maps to the
//! step user: the step may create and change files anywhere in its overlay,
//! as root may in a container, while the snapshot itself is only ever an
//! overlay lower or a read-only bind and never changes. A process of
//! another uid would see every file as nobody's, a different environment,
//! so `open` refuses a snapshot it does not own.
//!
//! What a rootfs mode step can still learn about the host is listed on
//! `sp_sandbox`; the key pins what changes behavior (`platform`).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use anyhow::{Context, bail};
use clap::{Args, Subcommand};
use serde::{Deserialize, Serialize};
use sp_cas::{Cas, Entry, Hash, Kind, Tree, canonical_json};
use sp_key::{Image, ImageKind, Platform};

use crate::snapshot::{self, Snapshot};
use crate::state::State;

/// Format of the rootfs document (`{"entries":[...],"v":1}`), whose BLAKE3
/// is the digest. Changing what `import` keeps or fixes up bumps this, since
/// every digest changes with it.
pub const VERSION: u32 = 1;

/// Sealed snapshots of rootfs trees kept per store beyond the one in use.
const KEEP: usize = 3;

/// Where a tree lies inside its snapshot directory (beside the marker).
const SUBDIR: &str = "root";

/// Directory mode of a rootfs snapshot: writable by the step user, so that
/// the overlay over it accepts writes anywhere.
const ROOTFS_DIR_MODE: u32 = 0o755;

/// The hostname every step sees (also set by the sandbox).
const HOSTNAME: &str = "stillpass\n";
/// A fixed /etc/hosts: Docker writes one per container, so the exported one
/// is empty and `localhost` would not resolve.
const HOSTS: &str = "127.0.0.1\tlocalhost\n::1\tlocalhost ip6-localhost ip6-loopback\n";

/// Top-level directories whose contents are the kernel's or the sandbox's,
/// never the image's: dropped at import (the directory itself stays as the
/// mount point).
const RUNTIME_DIRS: &[&str] = &["dev", "proc", "sys", "tmp"];

/// Directories the sandbox mounts on; created at import if the image lacks
/// them.
const MOUNT_POINTS: &[&str] = &["dev", "proc", "tmp", "workspace"];

const FINGERPRINT_SH: &str = include_str!("../../../scripts/host-fingerprint.sh");

#[derive(Serialize, Deserialize)]
struct Doc {
    v: u32,
    entries: Vec<Entry>,
}

/// What `import` recorded about a rootfs (`<cas>/rootfs/index/<digest>.json`).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Record {
    pub digest: Hash,
    /// Free text: the image tag or file it came from.
    pub source: Option<String>,
    /// The OCI digest (`sha256:...`) of the image, when known; indexed so
    /// `resolve` accepts it.
    pub oci: Option<String>,
    pub imported_at: String,
    pub entries: usize,
    pub files: usize,
    pub bytes: u64,
}

/// An opened rootfs: its tree and its sealed snapshot, shared-locked against
/// gc for as long as this is held.
pub struct Rootfs {
    pub digest: Hash,
    tree: Tree,
    snapshot: Snapshot,
}

impl Rootfs {
    /// The sealed snapshot: the rootfs at its own paths. Never written to.
    pub fn dir(&self) -> PathBuf {
        self.snapshot.dir.join(SUBDIR)
    }

    pub fn tree(&self) -> &Tree {
        &self.tree
    }

    /// The step key's `image` for a step run in this rootfs on this host.
    pub fn image(&self) -> anyhow::Result<Image> {
        Ok(Image::new(ImageKind::Rootfs, self.digest, platform()?))
    }
}

/// What this host's kernel and CPU show a step (`sp_sandbox::host_platform`)
/// as the key pins it: the part of the environment no root filesystem can.
pub fn platform() -> anyhow::Result<Platform> {
    let p = sp_sandbox::host_platform().context("probe the host platform")?;
    Ok(Platform::new(
        &p.machine,
        &p.release,
        &p.cpu_features,
        p.cpus,
        p.page_size,
    ))
}

/// The host-root pin: BLAKE3 of the host fingerprint script's output. The
/// fallback for hosts without a rootfs; such a key never travels.
pub fn host_image() -> anyhow::Result<Image> {
    let out = Command::new("/bin/bash")
        .arg("-c")
        .arg(FINGERPRINT_SH)
        .env_clear()
        .env("PATH", "/usr/local/go/bin:/usr/bin:/bin")
        .output()
        .context("run host fingerprint")?;
    if !out.status.success() {
        bail!(
            "host fingerprint failed ({}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(Image::new(
        ImageKind::Host,
        Hash::of(&out.stdout),
        platform()?,
    ))
}

/// The environment pin a run uses: `--rootfs` (or `$STILLPASS_ROOTFS`) opens
/// that rootfs; `--host-root` pins the host (the flag wins over the
/// variable; clap refuses it next to `--rootfs`); neither pins the host too,
/// with a note that the proof will not travel.
pub struct Pin {
    pub image: Image,
    pub rootfs: Option<Rootfs>,
}

pub fn pin(cas: &Cas, rootfs: Option<&str>, host_root: bool) -> anyhow::Result<Pin> {
    let from_env = std::env::var("STILLPASS_ROOTFS")
        .ok()
        .filter(|s| !s.is_empty());
    let id = rootfs.map(String::from).or(from_env);
    match id {
        Some(_) if host_root => Ok(Pin {
            image: host_image()?,
            rootfs: None,
        }),
        Some(id) => {
            let digest = resolve(cas, &id)?;
            let r = open(cas, &digest)?;
            Ok(Pin {
                image: r.image()?,
                rootfs: Some(r),
            })
        }
        None => {
            if !host_root {
                eprintln!(
                    "STILLPASS  note: no rootfs pinned; the key pins this host's fingerprint and the proof will not travel (pass --rootfs DIGEST or set STILLPASS_ROOTFS; --host-root silences this)"
                );
            }
            Ok(Pin {
                image: host_image()?,
                rootfs: None,
            })
        }
    }
}

fn index_dir(cas: &Cas) -> PathBuf {
    cas.dir().join("rootfs/index")
}

fn oci_dir(cas: &Cas) -> PathBuf {
    cas.dir().join("rootfs/oci")
}

fn snapshots_dir(cas: &Cas) -> PathBuf {
    cas.dir().join("rootfs/snapshots")
}

/// A rootfs digest from `b3:<hex>`, or from the OCI digest `sha256:<hex>`
/// of an image imported into this store.
pub fn resolve(cas: &Cas, id: &str) -> anyhow::Result<Hash> {
    if let Ok(h) = id.parse::<Hash>() {
        if cas.get(&h)?.is_none() {
            bail!(
                "rootfs {h} is not in the store {} (import it with `stillpass rootfs import <export.tar>`)",
                cas.dir().display()
            );
        }
        return Ok(h);
    }
    if let Some(hex) = id.strip_prefix("sha256:")
        && hex.len() == 64
        && hex.bytes().all(|b| b.is_ascii_hexdigit())
    {
        let p = oci_dir(cas).join(hex);
        return match fs::read_to_string(&p) {
            Ok(s) => s
                .trim()
                .parse::<Hash>()
                .with_context(|| format!("corrupt OCI index entry {}", p.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => bail!(
                "no rootfs imported from OCI image {id} in {} (`stillpass rootfs list`)",
                cas.dir().display()
            ),
            Err(e) => Err(e).with_context(|| format!("read {}", p.display())),
        };
    }
    bail!("{id:?} is neither a rootfs digest (b3:...) nor an OCI digest (sha256:...)")
}

/// The tree a rootfs digest names, from the store.
pub fn tree(cas: &Cas, digest: &Hash) -> anyhow::Result<Tree> {
    let bytes = cas
        .get(digest)?
        .with_context(|| format!("tree {digest} is not in the store"))?;
    let doc: Doc = serde_json::from_slice(&bytes)
        .with_context(|| format!("{digest} is not a tree document"))?;
    if doc.v != VERSION {
        bail!(
            "tree {digest} has document version {}, this binary knows {VERSION}",
            doc.v
        );
    }
    Ok(Tree::from_leaves(doc.entries))
}

/// `entries` under `SUBDIR/`, the way a snapshot holds them.
fn under_subdir(tree: &Tree) -> Vec<Entry> {
    tree.iter()
        .map(|e| Entry {
            p: if e.p == sp_cas::ROOT {
                SUBDIR.to_string()
            } else {
                format!("{SUBDIR}/{}", e.p)
            },
            ..e.clone()
        })
        .collect()
}

/// The snapshot's tree must belong to this process's uid: that is the uid
/// the sandbox maps to the step user. Any other would make every file
/// nobody's inside, and the step a different environment than the digest
/// says.
fn check_owner(dir: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::MetadataExt;
    let owner = fs::metadata(dir)
        .with_context(|| format!("stat {}", dir.display()))?
        .uid();
    let me = sp_sandbox::caller_uid();
    if owner != me {
        bail!(
            "{} belongs to uid {owner}, this process is uid {me}: inside the sandbox its files would not belong to the step user (a store shared between users needs one snapshot per user)",
            dir.display()
        );
    }
    Ok(())
}

/// Opens a rootfs: its tree, and its sealed snapshot built now if no run
/// built it before (shared with every state dir and process using the store).
pub fn open(cas: &Cas, digest: &Hash) -> anyhow::Result<Rootfs> {
    let tree = tree(cas, digest)?;
    let entries = under_subdir(&tree);
    let refs: Vec<&Entry> = entries.iter().collect();
    let t = Instant::now();
    let snapshot = snapshot::ensure_with(cas, &refs, &snapshots_dir(cas), ROOTFS_DIR_MODE)
        .with_context(|| format!("materialize rootfs {digest}"))?;
    tracing::info!(target: "stillpass", rootfs = %digest, key = %snapshot.key, reused = snapshot.reused, entries = entries.len(), ms = t.elapsed().as_millis() as u64, "rootfs snapshot");
    let r = Rootfs {
        digest: *digest,
        tree,
        snapshot,
    };
    check_owner(&r.dir())?;
    Ok(r)
}

/// Removes sealed rootfs snapshots beyond the `KEEP` most recently used
/// (never one in use). Housekeeping: never fails a run.
pub fn gc(cas: &Cas, current: &Rootfs) -> usize {
    snapshot::gc(&snapshots_dir(cas), KEEP, &current.snapshot.dir)
}

/// A per-step overlay over a sealed rootfs: the step's `/`, writable, with
/// every write landing in the upper dir.
pub struct RootOverlay<'a> {
    overlay: sp_fs::Overlay,
    lower: &'a Tree,
}

impl<'a> RootOverlay<'a> {
    /// Mounts the overlay under `dir/root` (`upper`, `work`, `merged`).
    pub fn mount(rootfs: &'a Rootfs, dir: &Path) -> anyhow::Result<RootOverlay<'a>> {
        let root = dir.join("root");
        let overlay = sp_fs::overlay_dir(
            &rootfs.dir(),
            &root.join("upper"),
            &root.join("work"),
            &root.join("merged"),
        )
        .context("mount the rootfs overlay")?;
        Ok(RootOverlay {
            overlay,
            lower: rootfs.tree(),
        })
    }

    /// The directory to bind at `/`.
    pub fn merged(&self) -> &Path {
        self.overlay.merged()
    }

    /// Unmounts and diffs the upper dir against the rootfs tree: the step's
    /// writes outside the workspace, paths relative to `/`, files stored in
    /// the CAS. Same precondition as `sp_fs::Overlay::finish`: the step is
    /// over.
    pub fn finish(self, cas: &Cas) -> anyhow::Result<Vec<Entry>> {
        Ok(self.overlay.finish(self.lower, cas)?)
    }
}

/// Imports a container filesystem export (a tar from `docker export`, or a
/// directory it was unpacked into) into the store and returns its record.
/// The digest is a function of the image alone: files Docker writes per
/// container (`.dockerenv`, `/etc/hostname`, `/etc/hosts`,
/// `/etc/resolv.conf`) are dropped or given fixed content, the kernel's and
/// sandbox's directories (`/dev`, `/proc`, `/sys`, `/tmp`) are emptied, and
/// the sandbox's mount points are created. Owners, setuid bits and xattrs
/// are not part of a tree and are not kept: inside the sandbox every file
/// belongs to the step user and nothing has capabilities.
pub fn import(
    cas: &Cas,
    src: &Path,
    source: Option<&str>,
    oci: Option<&str>,
) -> anyhow::Result<Record> {
    if let Some(o) = oci
        && !(o.starts_with("sha256:") && o.len() == 71)
    {
        bail!("--oci must be an OCI digest, sha256:<64 hex>");
    }
    let meta = fs::metadata(src).with_context(|| format!("rootfs source {}", src.display()))?;
    let extracted;
    let dir: &Path = if meta.is_dir() {
        src
    } else {
        extracted = cas
            .dir()
            .join("tmp")
            .join(format!("rootfs.{}", snapshot::random()));
        fs::create_dir_all(&extracted)?;
        let st = Command::new("tar")
            .args(["--no-same-owner", "-xpf"])
            .arg(src)
            .arg("-C")
            .arg(&extracted)
            .status()
            .context("run tar")?;
        if !st.success() {
            let _ = sp_cas::remove_any(&extracted);
            bail!("tar -xf {} failed ({st})", src.display());
        }
        &extracted
    };
    let result = import_dir(cas, dir, source, oci);
    if !meta.is_dir() {
        let _ = sp_cas::remove_any(dir);
    }
    result
}

fn import_dir(
    cas: &Cas,
    dir: &Path,
    source: Option<&str>,
    oci: Option<&str>,
) -> anyhow::Result<Record> {
    let (raw, stats) = cas
        .import_dir_stats(dir)
        .with_context(|| format!("import {}", dir.display()))?;
    let tree = normalize(cas, &raw)?;
    check_usable(&tree)?;
    let doc = Doc {
        v: VERSION,
        entries: tree.iter().cloned().collect(),
    };
    let digest = cas.put(canonical_json(&doc)?.as_bytes())?;
    let index = index_dir(cas);
    fs::create_dir_all(&index)?;
    let path = index.join(format!("{}.json", digest.hex()));
    // A re-import of the same content keeps what the first one recorded,
    // unless this one says more.
    let earlier: Option<Record> = fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok());
    let record = Record {
        digest,
        source: source
            .map(String::from)
            .or_else(|| earlier.as_ref().and_then(|r| r.source.clone())),
        oci: oci
            .map(String::from)
            .or_else(|| earlier.as_ref().and_then(|r| r.oci.clone())),
        imported_at: earlier
            .as_ref()
            .map_or_else(sp_ledger::now_rfc3339, |r| r.imported_at.clone()),
        entries: tree.len(),
        files: tree.iter().filter(|e| e.k == Kind::File).count(),
        bytes: stats.bytes,
    };
    fs::write(&path, serde_json::to_string_pretty(&record)?)?;
    if let Some(o) = oci {
        let d = oci_dir(cas);
        fs::create_dir_all(&d)?;
        fs::write(d.join(&o["sha256:".len()..]), digest.to_string())?;
    }
    Ok(record)
}

/// The import rules (see `import`), applied to a raw tree.
fn normalize(cas: &Cas, raw: &Tree) -> anyhow::Result<Tree> {
    let runtime: Vec<String> = RUNTIME_DIRS.iter().map(|d| format!("{d}/")).collect();
    let fixed = [
        ("etc/hostname", HOSTNAME.as_bytes()),
        ("etc/hosts", HOSTS.as_bytes()),
        ("etc/resolv.conf", b"".as_slice()),
    ];
    let mut leaves: Vec<Entry> = raw
        .iter()
        .filter(|e| e.p != ".dockerenv")
        .filter(|e| !runtime.iter().any(|r| e.p.starts_with(r)))
        .filter(|e| !fixed.iter().any(|(p, _)| e.p == *p))
        .cloned()
        .collect();
    for (p, content) in fixed {
        leaves.push(Entry::file(p, cas.put(content)?, 0o644));
    }
    for d in MOUNT_POINTS {
        if !raw.get(d).is_some_and(|e| e.k == Kind::Dir) {
            if raw.get(d).is_some() {
                bail!("the image has a file at /{d}, where the sandbox needs a directory");
            }
            leaves.push(Entry::dir(*d, Hash::of(b"")));
        }
    }
    Ok(Tree::from_leaves(leaves))
}

/// A rootfs the sandbox can run a step in: `/bin/sh` (the step's shell and
/// the start wrapper's), `/etc/passwd` and `/etc/group` (bind targets for
/// the step identity).
fn check_usable(tree: &Tree) -> anyhow::Result<()> {
    let resolve_top = |p: &str| -> String {
        let (top, rest) = p.split_once('/').unwrap_or((p, ""));
        match tree.get(top) {
            Some(e) if e.k == Kind::Symlink => {
                let t = e.t.as_deref().unwrap_or("").trim_matches('/');
                if rest.is_empty() {
                    t.to_string()
                } else {
                    format!("{t}/{rest}")
                }
            }
            _ => p.to_string(),
        }
    };
    let present = |p: &str| {
        tree.get(&resolve_top(p))
            .is_some_and(|e| matches!(e.k, Kind::File | Kind::Symlink))
    };
    for p in ["bin/sh", "etc/passwd", "etc/group"] {
        if !present(p) {
            bail!("not a usable rootfs: /{p} is missing");
        }
    }
    Ok(())
}

/// Every imported rootfs, newest first.
pub fn list(cas: &Cas) -> anyhow::Result<Vec<Record>> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(index_dir(cas)) else {
        return Ok(out);
    };
    for e in entries.flatten() {
        if e.path().extension().is_some_and(|x| x == "json") {
            let text = fs::read_to_string(e.path())?;
            out.push(serde_json::from_str::<Record>(&text)?);
        }
    }
    out.sort_by(|a, b| b.imported_at.cmp(&a.imported_at));
    Ok(out)
}

#[derive(Args)]
pub struct RootfsArgs {
    #[command(subcommand)]
    cmd: RootfsCmd,
}

#[derive(Subcommand)]
enum RootfsCmd {
    /// Import a container filesystem export (a `docker export` tar, or the
    /// directory it was unpacked into) and print its digest.
    Import {
        path: PathBuf,
        /// Where it came from, for the record (an image tag).
        #[arg(long)]
        source: Option<String>,
        /// The OCI digest (sha256:...) of the image it was exported from;
        /// `--rootfs sha256:...` then resolves to this import.
        #[arg(long)]
        oci: Option<String>,
        #[arg(long)]
        cas: Option<PathBuf>,
    },
    /// List the imported root filesystems.
    List {
        #[arg(long)]
        cas: Option<PathBuf>,
    },
}

pub fn cli(state: &State, args: RootfsArgs) -> anyhow::Result<()> {
    match args.cmd {
        RootfsCmd::Import {
            path,
            source,
            oci,
            cas,
        } => {
            let cas = Cas::open(cas.unwrap_or_else(|| state.cas_dir()))?;
            let t = Instant::now();
            let r = import(&cas, &path, source.as_deref(), oci.as_deref())?;
            eprintln!(
                "imported {} entries ({} files, {} bytes) in {:.1}s into {}",
                r.entries,
                r.files,
                r.bytes,
                t.elapsed().as_secs_f64(),
                cas.dir().display()
            );
            println!("{}", r.digest);
        }
        RootfsCmd::List { cas } => {
            let cas = Cas::open(cas.unwrap_or_else(|| state.cas_dir()))?;
            for r in list(&cas)? {
                println!(
                    "{}  {}  {} entries  {} bytes  {}{}",
                    r.digest,
                    r.imported_at,
                    r.entries,
                    r.bytes,
                    r.source.as_deref().unwrap_or("-"),
                    r.oci
                        .as_deref()
                        .map(|o| format!("  {o}"))
                        .unwrap_or_default()
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    /// A tiny container export: merged usr, a shell, the files Docker
    /// writes per container, device nodes as empty files, a host file.
    fn export(dir: &Path, marker: &str) {
        let w = |p: &str, s: &str, mode: u32| {
            let p = dir.join(p);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, s).unwrap();
            fs::set_permissions(&p, fs::Permissions::from_mode(mode)).unwrap();
        };
        w("usr/bin/dash", "#!/bin/dash\n", 0o755);
        std::os::unix::fs::symlink("dash", dir.join("usr/bin/sh")).unwrap();
        std::os::unix::fs::symlink("usr/bin", dir.join("bin")).unwrap();
        w("etc/passwd", "root:x:0:0::/root:/bin/sh\n", 0o644);
        w("etc/group", "root:x:0:\n", 0o644);
        w("etc/os-release", marker, 0o644);
        w("etc/hostname", "a1b2c3d4\n", 0o755);
        w("etc/hosts", "", 0o755);
        w("etc/resolv.conf", "nameserver 127.0.0.11\n", 0o755);
        w(".dockerenv", "", 0o755);
        w("dev/console", "", 0o755);
        fs::create_dir_all(dir.join("dev/pts")).unwrap();
        fs::create_dir_all(dir.join("proc")).unwrap();
        fs::create_dir_all(dir.join("sys/kernel")).unwrap();
        w("tmp/leftover", "x", 0o644);
        fs::create_dir_all(dir.join("opt")).unwrap();
    }

    #[test]
    fn import_fixes_up_runtime_files_and_is_deterministic() {
        let tmp = tempfile::tempdir().unwrap();
        let cas = Cas::open(tmp.path().join("cas")).unwrap();
        let a = tmp.path().join("a");
        export(&a, "ID=test\n");
        let r1 = import(
            &cas,
            &a,
            Some("test:1"),
            Some(&format!("sha256:{}", "a".repeat(64))),
        )
        .unwrap();
        let t = tree(&cas, &r1.digest).unwrap();
        assert!(t.get(".dockerenv").is_none());
        assert!(t.get("dev").is_some_and(|e| e.k == Kind::Dir) && t.get("dev/console").is_none());
        assert!(t.get("dev/pts").is_none() && t.get("sys/kernel").is_none());
        assert!(t.get("tmp").is_some() && t.get("tmp/leftover").is_none());
        assert!(
            t.get("workspace").is_some_and(|e| e.k == Kind::Dir),
            "mount point created"
        );
        let content = |p: &str| cas.get(&t.get(p).unwrap().h.unwrap()).unwrap().unwrap();
        assert_eq!(content("etc/hostname"), b"stillpass\n");
        assert_eq!(content("etc/hosts"), HOSTS.as_bytes());
        assert_eq!(content("etc/resolv.conf"), b"");
        assert_eq!(t.get("etc/hostname").unwrap().m, Some(0o644));
        assert_eq!(content("etc/os-release"), b"ID=test\n");
        assert_eq!(t.get("usr/bin/dash").unwrap().m, Some(0o755));
        assert_eq!(t.get("bin").unwrap().t.as_deref(), Some("usr/bin"));

        // The same export elsewhere, with other per-container junk: same digest.
        let b = tmp.path().join("b");
        export(&b, "ID=test\n");
        fs::write(b.join("etc/hostname"), "zzzz\n").unwrap();
        fs::write(b.join("etc/resolv.conf"), "nameserver 1.1.1.1\n").unwrap();
        fs::write(b.join("dev/console"), "tty").unwrap();
        fs::write(b.join("tmp/other"), "y").unwrap();
        let r2 = import(&cas, &b, None, None).unwrap();
        assert_eq!(r2.digest, r1.digest);
        // A real difference is another digest.
        let c = tmp.path().join("c");
        export(&c, "ID=other\n");
        assert_ne!(import(&cas, &c, None, None).unwrap().digest, r1.digest);

        // Resolution by digest and by OCI digest; the index lists both.
        assert_eq!(resolve(&cas, &r1.digest.to_string()).unwrap(), r1.digest);
        assert_eq!(
            resolve(&cas, &format!("sha256:{}", "a".repeat(64))).unwrap(),
            r1.digest
        );
        assert!(resolve(&cas, &format!("sha256:{}", "b".repeat(64))).is_err());
        assert!(resolve(&cas, &Hash::of(b"nope").to_string()).is_err());
        assert!(resolve(&cas, "latest").is_err());
        let l = list(&cas).unwrap();
        assert_eq!(l.len(), 2);
        let kept = l.iter().find(|r| r.digest == r1.digest).unwrap();
        assert_eq!(
            (kept.source.as_deref(), kept.oci.is_some()),
            (Some("test:1"), true),
            "a re-import without provenance keeps the first import's"
        );
        assert_eq!(kept.imported_at, r1.imported_at);
    }

    #[test]
    fn unusable_exports_are_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let cas = Cas::open(tmp.path().join("cas")).unwrap();
        let a = tmp.path().join("a");
        export(&a, "x");
        fs::remove_file(a.join("usr/bin/sh")).unwrap();
        let e = import(&cas, &a, None, None).unwrap_err().to_string();
        assert!(e.contains("/bin/sh is missing"), "{e}");
        let b = tmp.path().join("b");
        export(&b, "x");
        fs::remove_file(b.join("etc/group")).unwrap();
        assert!(import(&cas, &b, None, None).is_err());
        let c = tmp.path().join("c");
        export(&c, "x");
        fs::write(c.join("workspace"), "a file").unwrap();
        let e = import(&cas, &c, None, None).unwrap_err().to_string();
        assert!(e.contains("/workspace"), "{e}");
        assert!(import(&cas, &a, None, Some("sha256:short")).is_err());
    }

    #[test]
    fn open_materializes_once_and_reuses() {
        let tmp = tempfile::tempdir().unwrap();
        let cas = Cas::open(tmp.path().join("cas")).unwrap();
        let a = tmp.path().join("a");
        export(&a, "ID=test\n");
        let r = import(&cas, &a, None, None).unwrap();
        let first = open(&cas, &r.digest).unwrap();
        assert!(!first.snapshot.reused);
        let dir = first.dir();
        assert!(
            !dir.join(".complete").exists() && first.snapshot.dir.join(".complete").exists(),
            "the marker is beside the rootfs, not in it"
        );
        assert_eq!(
            fs::read_to_string(dir.join("etc/hostname")).unwrap(),
            "stillpass\n"
        );
        assert_eq!(
            fs::read_link(dir.join("bin")).unwrap().to_str(),
            Some("usr/bin")
        );
        assert_eq!(
            fs::metadata(dir.join("usr/bin/dash"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o755
        );
        assert!(dir.join("workspace").is_dir() && dir.join("dev").is_dir());
        assert!(!dir.join("tmp/leftover").exists());
        for d in [&dir, &dir.join("usr/bin"), &dir.join("tmp")] {
            assert_eq!(
                fs::metadata(d).unwrap().permissions().mode() & 0o777,
                0o755,
                "{}: the step user may write anywhere in its overlay",
                d.display()
            );
        }
        let again = open(&cas, &r.digest).unwrap();
        assert!(again.snapshot.reused && again.dir() == dir);
        let image = again.image().unwrap();
        assert_eq!((image.kind, image.digest), (ImageKind::Rootfs, r.digest));
        assert!(image.mounts.is_empty() && image.platform.cpus >= 1);
        assert!(open(&cas, &Hash::of(b"missing")).is_err());
        // A blob that is not a rootfs document is refused.
        let junk = cas.put(b"{\"v\":1}").unwrap();
        assert!(open(&cas, &junk).is_err());
        drop((first, again));

        // A snapshot another uid built would show the step nobody's files:
        // refused (checkable only where this test may chown, i.e. as root).
        let me = sp_sandbox::caller_uid();
        if std::os::unix::fs::chown(&dir, Some(me + 1), None).is_ok() {
            let e = open(&cas, &r.digest).err().expect("refused").to_string();
            assert!(e.contains("belongs to uid"), "{e}");
            std::os::unix::fs::chown(&dir, Some(me), None).unwrap();
            assert!(open(&cas, &r.digest).is_ok());
        }
    }
}
