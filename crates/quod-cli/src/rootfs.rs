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
//! The digest pins the tree, which is less than the image: what a tree does
//! not hold is not kept. Owners (every file belongs to the step user),
//! setuid, setgid and sticky bits (modes are masked to 0777), xattrs and
//! file capabilities, directory modes (every directory is 0755), hard links
//! (each name is its own file), device nodes, sockets and fifos (refused at
//! import), and timestamps (a served file's time derives from its content,
//! `quod_fs::content_mtime`; directories' and symlinks' are the epoch). Two
//! images that differ only there have one digest, and a step sees the same
//! rootfs in both.
//!
//! At run time the rootfs is served from its blobs by the observer's FUSE
//! server (`quod_fs::serve`; its log is discarded, since the digest already
//! pins everything it would record), not copied onto a host filesystem: a
//! file's times derive from its content (`quod_fs::content_mtime`, as in the
//! workspace and the sealed snapshots) and other times are fixed, listings
//! come in tree order, nothing records an access time, inode numbers are the
//! tree's, and every file belongs to whoever runs Quod, which the
//! sandbox maps to the step user: what a copy on a host filesystem would let
//! through (copy times, ext4's per-filesystem hash order, atime updates,
//! ownership) does not. `RootOverlay::mount` puts a per-step overlay over it
//! whose upper lives on a tmpfs of fixed limits (`ROOT_TMPFS`), so the step
//! may write anywhere and the upper is diffed into the result's
//! `root_outputs` like the workspace overlay is diffed into `outputs`,
//! except /tmp, which is scratch. The workspace stays the observed FUSE
//! tree, and its overlay's upper is on the state filesystem: the listing
//! order of a directory a step creates in the workspace follows that
//! filesystem (its type is in the key, its hash seed is not; a known gap).
//!
//! The same document format pins any read-only tree (`put_tree`): a runner
//! adapter's mounts are served the same way (`PinnedTree`) and their digests
//! are part of the key. The runner API is `crate::step`.
//!
//! What a rootfs mode step can still learn about the host is listed on
//! `quod_sandbox`; the key pins all of it that is constant for a host
//! (`platform`, with the host class).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use anyhow::{Context, bail};
use clap::{Args, Subcommand};
use quod_cas::{Cas, Entry, Hash, Kind, Tree, canonical_json};
use quod_key::{Image, ImageKind, Platform};
use serde::{Deserialize, Serialize};

use crate::snapshot;
use crate::state::State;

/// Format of the rootfs document (`{"entries":[...],"v":1}`), whose BLAKE3
/// is the digest. Changing what `import` keeps or fixes up bumps this, since
/// every digest changes with it.
pub const VERSION: u32 = 1;

/// The per-step tmpfs holding the root overlay's upper: everything a step
/// writes outside the workspace, Go's `$WORK` and every other TMPDIR use
/// included, lives in RAM up to these limits, which are part of the host
/// class (a step writing beyond them gets ENOSPC everywhere alike).
pub const ROOT_TMPFS: &str = "size=4096m,nr_inodes=1048576,mode=0755";

/// The hostname every step sees (also set by the sandbox).
const HOSTNAME: &str = "quod\n";
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

/// An opened rootfs: its digest and tree, every blob present in the store.
pub struct Rootfs {
    pub digest: Hash,
    tree: Tree,
}

impl Rootfs {
    #[cfg(test)]
    pub fn tree(&self) -> &Tree {
        &self.tree
    }

    /// The step key's `image` for a step run in this rootfs on this host,
    /// with run directories under `runs`.
    pub fn image(&self, cas: &Cas, runs: &Path) -> anyhow::Result<Image> {
        Ok(Image::new(
            ImageKind::Rootfs,
            self.digest,
            platform(cas, runs)?,
        ))
    }
}

/// What this host's kernel and CPU show a step, as the key pins it: the part
/// of the environment no root filesystem can. Read the way the step sees it,
/// by this binary run in a sandbox like a step's (`__host-probe`, through
/// `quod_sandbox::probe_in_sandbox`: a fresh procfs, fresh namespaces, the
/// fixed process state), not from this process's own /proc, which a
/// container runtime may virtualize (lxcfs) where the step's fresh procfs is
/// not. Then what only the host sees (`quod_sandbox::host_class_outside`):
/// cgroup limits, binfmt_misc, the filesystems of the store and of `runs`
/// (where run directories, the workspace overlay's upper, live), the
/// sandbox's version; and the root overlay's tmpfs limits.
pub fn platform(cas: &Cas, runs: &Path) -> anyhow::Result<Platform> {
    fs::create_dir_all(runs).with_context(|| format!("create {}", runs.display()))?;
    let exe = std::env::current_exe().context("locate this binary for the host probe")?;
    let out = quod_sandbox::probe_in_sandbox(&exe, &[HOST_PROBE])
        .context("probe the host class in the sandbox")?;
    let probe: Probe = serde_json::from_str(&out).context("parse the host probe")?;
    let mut host = probe.host;
    host.extend(
        quod_sandbox::host_class_outside(&[("store", cas.dir()), ("runs", runs)])
            .context("probe the host class")?,
    );
    host.push(("root tmpfs".into(), ROOT_TMPFS.into()));
    Ok(Platform::new(
        &probe.machine,
        &probe.release,
        &probe.cpu_features,
        probe.cpus,
        probe.page_size,
        host,
    ))
}

/// The hidden subcommand `platform` runs in the sandbox.
pub const HOST_PROBE: &str = "__host-probe";

/// What `__host-probe` prints.
#[derive(Serialize, Deserialize)]
struct Probe {
    machine: String,
    release: String,
    cpu_features: String,
    cpus: u32,
    page_size: u64,
    host: Vec<(String, String)>,
}

/// `quod __host-probe`: run inside the sandbox by `platform`; prints
/// the platform and the inside half of the host class as JSON.
pub fn host_probe() -> anyhow::Result<u8> {
    let p = quod_sandbox::host_platform()?;
    let mut host = quod_sandbox::host_class_inside()?;
    // The kernel's configuration, when it publishes it.
    let config = fs::read("/proc/config.gz").map_or("-".to_string(), |b| Hash::of(&b).to_string());
    host.push(("kernel config".into(), config));
    let probe = Probe {
        machine: p.machine,
        release: p.release,
        cpu_features: p.cpu_features,
        cpus: p.cpus,
        page_size: p.page_size,
        host,
    };
    println!("{}", serde_json::to_string(&probe)?);
    Ok(0)
}

/// The host-root pin: BLAKE3 of the host fingerprint script's output, which
/// covers every toolchain installed on the host (go, node, python, uv), run
/// with every profile's PATH so it resolves the binaries steps do. The
/// fallback for hosts without a rootfs; such a key never travels.
pub fn host_image(cas: &Cas, runs: &Path) -> anyhow::Result<Image> {
    // The fingerprint and the platform probe are independent processes
    // (~0.3s and ~0.2s): run them at once.
    let (out, platform) = std::thread::scope(|s| {
        let probe = s.spawn(|| platform(cas, runs));
        // The toolchain binaries' hashes are kept by file identity under
        // the store (the script's QUOD_FP_CACHE): the output is the same
        // bytes, without reading ~200 MB of binaries every run.
        let out = Command::new("/bin/bash")
            .arg("-c")
            .arg(FINGERPRINT_SH)
            .env_clear()
            .env("PATH", quod_key::profile::HOST_PATH)
            .env("QUOD_FP_CACHE", cas.dir().join("host-fingerprint-cache"))
            .output()
            .context("run host fingerprint");
        let platform = probe
            .join()
            .unwrap_or_else(|_| Err(anyhow::anyhow!("the host platform probe panicked")));
        (out, platform)
    });
    let out = out?;
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
        platform?,
    ))
}

/// The environment pin a run uses: `--rootfs` (or `$QUOD_ROOTFS`) opens
/// that rootfs; `--host-root` pins the host (the flag wins over the
/// variable; clap refuses it next to `--rootfs`); neither pins the host too,
/// with a note that the proof will not travel.
pub struct Pin {
    pub image: Image,
    pub rootfs: Option<Rootfs>,
}

pub fn pin(cas: &Cas, rootfs: Option<&str>, host_root: bool, runs: &Path) -> anyhow::Result<Pin> {
    let from_env = std::env::var("QUOD_ROOTFS").ok().filter(|s| !s.is_empty());
    let id = rootfs.map(String::from).or(from_env);
    match id {
        Some(_) if host_root => Ok(Pin {
            image: host_image(cas, runs)?,
            rootfs: None,
        }),
        Some(id) => {
            let digest = resolve(cas, &id)?;
            let r = open(cas, &digest)?;
            Ok(Pin {
                image: r.image(cas, runs)?,
                rootfs: Some(r),
            })
        }
        None => {
            if !host_root {
                eprintln!(
                    "QUOD  note: no rootfs pinned; the key pins this host's fingerprint and the proof will not travel (pass --rootfs DIGEST or set QUOD_ROOTFS; --host-root silences this)"
                );
            }
            Ok(Pin {
                image: host_image(cas, runs)?,
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

/// A rootfs digest from `b3:<hex>`, or from the OCI digest `sha256:<hex>`
/// of an image imported into this store.
pub fn resolve(cas: &Cas, id: &str) -> anyhow::Result<Hash> {
    if let Ok(h) = id.parse::<Hash>() {
        if cas.get(&h)?.is_none() {
            bail!(
                "rootfs {h} is not in the store {} (import it with `quod rootfs import <export.tar>`)",
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
                "no rootfs imported from OCI image {id} in {} (`quod rootfs list`)",
                cas.dir().display()
            ),
            Err(e) => Err(e).with_context(|| format!("read {}", p.display())),
        };
    }
    bail!("{id:?} is neither a rootfs digest (b3:...) nor an OCI digest (sha256:...)")
}

/// The tree a digest names, from the store: a rootfs, or any tree stored
/// with `put_tree`.
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

/// Every file blob `tree` names is in the store (else serving it would be
/// an I/O error in the middle of a step).
fn check_blobs(cas: &Cas, tree: &Tree, what: &str) -> anyhow::Result<()> {
    for e in tree.iter() {
        if let Some(h) = e.h.filter(|_| e.k == Kind::File)
            && !cas.has(&h)
        {
            bail!("{what}: {} names blob {h}, which is not in the store", e.p);
        }
    }
    Ok(())
}

/// Stores a tree as a document (the rootfs format, without the rootfs
/// rules) and returns its digest: how a runner pins a workspace or a
/// read-only mount by content. Every blob it names must be in the store.
pub fn put_tree(cas: &Cas, tree: &Tree) -> anyhow::Result<Hash> {
    check_blobs(cas, tree, "tree")?;
    let doc = Doc {
        v: VERSION,
        entries: tree.iter().cloned().collect(),
    };
    Ok(cas.put(canonical_json(&doc)?.as_bytes())?)
}

/// Opens a rootfs: its tree, checked to be servable from the store.
pub fn open(cas: &Cas, digest: &Hash) -> anyhow::Result<Rootfs> {
    let t = Instant::now();
    let tree = tree(cas, digest)?;
    check_blobs(cas, &tree, &format!("rootfs {digest}"))?;
    tracing::info!(target: "quod", rootfs = %digest, entries = tree.len(), ms = t.elapsed().as_millis() as u64, "rootfs open");
    Ok(Rootfs {
        digest: *digest,
        tree,
    })
}

/// Serves `tree` read-only at `dir` (created): the observer without its log.
/// Opens are answered (no zero-message opens): a root filesystem is mostly
/// binaries and libraries read once per process, which passthrough serves
/// without a READ reaching userspace.
fn serve_tree(cas: &Cas, tree: &Tree, dir: &Path) -> anyhow::Result<quod_fs::Served> {
    fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    let opts = quod_fs::Options {
        zero_open: false,
        ..quod_fs::Options::from_env()
    };
    quod_fs::serve_with(tree, cas, dir, &opts)
        .with_context(|| format!("serve the rootfs at {}", dir.display()))
}

/// Unmounts a served tree. The log is discarded, but an I/O error served to
/// the step is a Quod error, as on the workspace.
fn unserve(served: quod_fs::Served) -> anyhow::Result<()> {
    served.unmount().context("unmount the served rootfs")?;
    Ok(())
}

/// A per-step overlay over a served rootfs: the step's `/`, writable, with
/// every write landing in an upper dir on a tmpfs.
pub struct RootOverlay<'a> {
    overlay: quod_fs::Overlay,
    served: quod_fs::Served,
    tmpfs: quod_fs::Tmpfs,
    lower: &'a Tree,
}

impl<'a> RootOverlay<'a> {
    /// Mounts under `dir/root`: the served rootfs at `lower`, a tmpfs at
    /// `rw` (size half the bucketed RAM, so its limit is a function of the
    /// key) holding `upper` and `work`, the overlay at `merged`.
    pub fn mount(rootfs: &'a Rootfs, cas: &Cas, dir: &Path) -> anyhow::Result<RootOverlay<'a>> {
        let root = dir.join("root");
        let served = serve_tree(cas, &rootfs.tree, &root.join("lower"))?;
        let tmpfs = quod_fs::tmpfs(&root.join("rw"), ROOT_TMPFS)
            .context("mount the rootfs overlay's tmpfs")?;
        let overlay = quod_fs::overlay(
            &served,
            &tmpfs.dir().join("upper"),
            &tmpfs.dir().join("work"),
            &root.join("merged"),
        )
        .context("mount the rootfs overlay")?;
        Ok(RootOverlay {
            overlay,
            served,
            tmpfs,
            lower: &rootfs.tree,
        })
    }

    /// The directory to bind at `/`.
    pub fn merged(&self) -> &Path {
        self.overlay.merged()
    }

    /// Unmounts and diffs the upper dir against the rootfs tree: the step's
    /// writes outside the workspace, paths relative to `/`, files stored in
    /// the CAS. The upper is edited only after the overlay is unmounted
    /// (nothing reaches it then): /tmp is dropped whole, since it is scratch
    /// (as on a runner between jobs; every step starts from the pinned image
    /// anyway, and root outputs are not replayed), so its leftovers are
    /// neither outputs nor copied into the store; sockets and fifos are
    /// dropped, since they hold no content, and where one replaced a file of
    /// the image the deletion is recorded instead. Same precondition as
    /// `quod_fs::Overlay::finish`: the step is over.
    pub fn finish(mut self, cas: &Cas) -> anyhow::Result<Vec<Entry>> {
        self.overlay.unmount()?;
        let upper = self.tmpfs.dir().join("upper");
        let tmp = upper.join("tmp");
        let scratch = match fs::symlink_metadata(&tmp) {
            Ok(_) => {
                let n = count_entries(&tmp);
                quod_cas::remove_any(&tmp)?;
                n
            }
            Err(_) => 0,
        };
        let mut deleted = Vec::new();
        drop_special_files(&upper, quod_cas::ROOT, &mut deleted)?;
        let mut out = self.overlay.finish(self.lower, cas)?;
        if scratch > 0 || !deleted.is_empty() {
            tracing::info!(target: "quod", scratch, special = deleted.len(), "outside the workspace: /tmp left out, sockets and fifos dropped");
        }
        out.extend(
            deleted
                .into_iter()
                .filter(|p| self.lower.get(p).is_some())
                .map(Entry::deleted),
        );
        // The diff walks in path order; a deletion of a leaf has no
        // children, and a stable sort keeps `x` before `d` for one path.
        out.sort_by(|a, b| a.p.cmp(&b.p));
        unserve(self.served)?;
        self.tmpfs.unmount()?;
        Ok(out)
    }
}

/// Entries under `dir` (itself included), for the event.
fn count_entries(dir: &Path) -> usize {
    1 + fs::read_dir(dir).map_or(0, |d| {
        d.flatten()
            .map(|e| {
                if e.file_type().is_ok_and(|t| t.is_dir()) {
                    count_entries(&e.path())
                } else {
                    1
                }
            })
            .sum()
    })
}

/// Removes sockets and fifos under `dir` (tree path `rel`), collecting
/// their tree paths.
fn drop_special_files(dir: &Path, rel: &str, out: &mut Vec<String>) -> anyhow::Result<()> {
    use std::os::unix::fs::FileTypeExt;
    for item in fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))? {
        let item = item?;
        let ft = item.file_type()?;
        let name = item.file_name().to_string_lossy().into_owned();
        let p = quod_cas::child_path(rel, &name);
        if ft.is_socket() || ft.is_fifo() {
            fs::remove_file(item.path())?;
            out.push(p);
        } else if ft.is_dir() {
            drop_special_files(&item.path(), &p, out)?;
        }
    }
    Ok(())
}

/// A rootfs served read-only for a step that is never memoized (step A,
/// with network): its writes outside the workspace fail, /tmp is a tmpfs.
pub struct ReadOnlyRoot {
    served: quod_fs::Served,
}

impl ReadOnlyRoot {
    pub fn mount(rootfs: &Rootfs, cas: &Cas, dir: &Path) -> anyhow::Result<ReadOnlyRoot> {
        Ok(ReadOnlyRoot {
            served: serve_tree(cas, &rootfs.tree, &dir.join("root"))?,
        })
    }

    pub fn dir(&self) -> &Path {
        self.served.mountpoint()
    }

    pub fn unmount(self) -> anyhow::Result<()> {
        unserve(self.served)
    }
}

/// A pinned read-only tree (`put_tree`), served for binding into a step.
pub struct PinnedTree {
    served: quod_fs::Served,
}

impl PinnedTree {
    /// Serves the tree `digest` names at `dir`.
    pub fn mount(cas: &Cas, digest: &Hash, dir: &Path) -> anyhow::Result<PinnedTree> {
        let tree = tree(cas, digest)?;
        check_blobs(cas, &tree, &format!("tree {digest}"))?;
        Ok(PinnedTree {
            served: serve_tree(cas, &tree, dir)?,
        })
    }

    pub fn dir(&self) -> &Path {
        self.served.mountpoint()
    }

    pub fn unmount(self) -> anyhow::Result<()> {
        unserve(self.served)
    }
}

/// Imports a container filesystem export (a tar from `docker export`, or a
/// directory it was unpacked into) into the store and returns its record.
/// The digest is a function of the image alone: files Docker writes per
/// container (`.dockerenv`, `/etc/hostname`, `/etc/hosts`,
/// `/etc/resolv.conf`) are dropped or given fixed content, the kernel's and
/// sandbox's directories (`/dev`, `/proc`, `/sys`, `/tmp`) are emptied, and
/// the sandbox's mount points are created. What a tree does not hold is not
/// kept (see the module docs). An OCI digest given here is indexed and
/// cannot later be re-pointed to another rootfs.
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
            let _ = quod_cas::remove_any(&extracted);
            bail!("tar -xf {} failed ({st})", src.display());
        }
        &extracted
    };
    let result = import_dir(cas, dir, source, oci);
    if !meta.is_dir() {
        let _ = quod_cas::remove_any(dir);
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
    // An OCI digest names one image, so it can only ever name one rootfs:
    // the first import of it is trusted, a different one is refused, never
    // silently re-pointed (a key found by OCI digest would change meaning).
    if let Some(o) = oci
        && let Ok(earlier) = fs::read_to_string(oci_dir(cas).join(&o["sha256:".len()..]))
        && earlier.trim() != digest.to_string()
    {
        bail!(
            "OCI image {o} was imported earlier as rootfs {}, and this export is {digest}: refusing to re-point it (same image, different filesystem: check the export)",
            earlier.trim()
        );
    }
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
            .map_or_else(quod_ledger::now_rfc3339, |r| r.imported_at.clone()),
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
        assert_eq!(content("etc/hostname"), b"quod\n");
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
    fn open_checks_the_store_and_pins_the_platform() {
        let tmp = tempfile::tempdir().unwrap();
        let cas = Cas::open(tmp.path().join("cas")).unwrap();
        let a = tmp.path().join("a");
        export(&a, "ID=test\n");
        let r = import(&cas, &a, None, None).unwrap();
        let opened = open(&cas, &r.digest).unwrap();
        assert_eq!(opened.tree().len(), r.entries);
        // The platform half of the image is probed by the binary itself in
        // the sandbox (`__host-probe`): covered by the integration tests.
        assert!(open(&cas, &Hash::of(b"missing")).is_err());
        // A blob that is not a rootfs document is refused.
        let junk = cas.put(b"{\"v\":1}").unwrap();
        assert!(open(&cas, &junk).is_err());
        // A rootfs whose blobs are not all in the store is refused up front.
        let file = r.digest;
        let partial = Tree::from_leaves([
            Entry::file("bin/sh", Hash::of(b"never stored"), 0o755),
            Entry::file("etc/passwd", file, 0o644),
        ]);
        let doc = Doc {
            v: VERSION,
            entries: partial.iter().cloned().collect(),
        };
        let d = cas.put(canonical_json(&doc).unwrap().as_bytes()).unwrap();
        let e = open(&cas, &d).err().expect("refused").to_string();
        assert!(e.contains("not in the store"), "{e}");
    }

    #[test]
    fn an_oci_digest_cannot_be_re_pointed() {
        let tmp = tempfile::tempdir().unwrap();
        let cas = Cas::open(tmp.path().join("cas")).unwrap();
        let oci = format!("sha256:{}", "c".repeat(64));
        let a = tmp.path().join("a");
        export(&a, "ID=one\n");
        let r = import(&cas, &a, None, Some(&oci)).unwrap();
        // The same content again: fine.
        assert_eq!(import(&cas, &a, None, Some(&oci)).unwrap().digest, r.digest);
        let b = tmp.path().join("b");
        export(&b, "ID=two\n");
        let e = import(&cas, &b, None, Some(&oci)).unwrap_err().to_string();
        assert!(e.contains("refusing to re-point"), "{e}");
        assert_eq!(resolve(&cas, &oci).unwrap(), r.digest, "still the first");
    }

    #[test]
    fn put_tree_pins_any_tree() {
        let tmp = tempfile::tempdir().unwrap();
        let cas = Cas::open(tmp.path().join("cas")).unwrap();
        let src = tmp.path().join("tools");
        fs::create_dir_all(src.join("bin")).unwrap();
        fs::write(src.join("bin/t"), "#!/bin/sh\n").unwrap();
        fs::set_permissions(src.join("bin/t"), fs::Permissions::from_mode(0o755)).unwrap();
        let t = cas.import_dir(&src).unwrap();
        let d = put_tree(&cas, &t).unwrap();
        assert_eq!(tree(&cas, &d).unwrap().len(), t.len());
        assert_eq!(put_tree(&cas, &t).unwrap(), d, "content-addressed");
        // A tree naming a blob the store lacks is refused.
        let bad = Tree::from_leaves([Entry::file("x", Hash::of(b"absent"), 0o644)]);
        assert!(put_tree(&cas, &bad).is_err());
    }
}
