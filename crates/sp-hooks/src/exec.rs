//! One execution of a step: the workspace tree served over FUSE under an
//! overlay, the image root under a per-step overlay, both bound into
//! bubblewrap with the surface's mounts; afterwards the access log, what the
//! step wrote, and what may have decided the outcome besides its inputs.
//!
//! Layers. A step that wrote outside the workspace leaves its root upper
//! dir as a layer every later step of the job sees (as later `docker exec`s
//! would), and taints them. A step that ran with the job's secrets (the
//! runner's surface) stores nothing: its workspace writes stay in its upper
//! dir, a layer over the served tree for the rest of the job, deleted at
//! cleanup, and every later step is tainted by it. Mount points bubblewrap
//! creates land in the step's own root upper, which is dropped unless the
//! step wrote something else there.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::Context;
use sp_cas::{Cas, Entry, Hash, Tree};
use sp_manifest::Access;
use sp_sandbox::{Mount, RunSpec};

use crate::rootfs::Rootfs;
use crate::seal::{seal_modcache, sealed_accesses};
use crate::secrets;
use crate::snapshot;

/// What a step gets as GOCACHE.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GoCache {
    /// Nothing bound: GOCACHE, if set, is in the observed workspace.
    None,
    /// The job's own scratch cache (the runner's and the network's
    /// surfaces, never recorded): not checked.
    Scratch(PathBuf),
    /// The scope's warm build cache, written in place: a writer job's
    /// (`Job::writer`). What the step leaves in it is checked
    /// (`go_cache_delta`).
    Shared(PathBuf),
    /// The scope's warm build cache under the job's own copy-on-write layer
    /// (`layer`/upper, `layer`/work): every job that is not a writer. The
    /// step reads the cache and writes the layer, which goes with the job.
    /// Checked like `Shared`.
    Layered { lower: PathBuf, layer: PathBuf },
}

/// What becomes of a step's writes and logs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Keep {
    /// Outputs into the store, logs masked and stored: a step that ran
    /// without the job's secrets.
    Store,
    /// Nothing stored: the workspace writes stay a job-local layer.
    Layer,
}

pub struct Exec<'a> {
    pub cas: &'a Cas,
    pub tree: &'a Tree,
    /// The image root; `None` runs on the host's system paths (tests).
    pub rootfs: Option<&'a Rootfs>,
    /// Earlier steps' writes outside the workspace, newest first.
    pub root_layers: &'a [PathBuf],
    /// Earlier secret-bearing steps' workspace writes, newest first.
    pub ws_layers: &'a [PathBuf],
    pub step_dir: &'a Path,
    /// GITHUB_WORKSPACE inside the sandbox: where the tree is bound.
    pub workspace: &'a str,
    /// The surface's binds (host paths).
    pub binds: Vec<Mount>,
    pub echo: bool,
    pub cmd: Vec<String>,
    pub cwd: String,
    /// The complete environment the step sees.
    pub env: Vec<(String, String)>,
    pub egress: bool,
    /// Count network attempts (offline steps).
    pub net_probe: bool,
    /// Serve GOMODCACHE from a sealed snapshot.
    pub seal: bool,
    /// What is bound over GOCACHE.
    pub gocache: GoCache,
    pub uid: Option<u32>,
    pub gid: Option<u32>,
    pub keep: Keep,
    /// Secret values: masked in stored logs; outputs containing one are not
    /// stored.
    pub masks: &'a [Vec<u8>],
}

pub struct Executed {
    pub exit_code: i32,
    pub signal: Option<i32>,
    pub wall: Duration,
    /// What the step read, after sealing and warm-state over-recording.
    pub log: Vec<Access>,
    /// Keep::Store: the workspace outputs, in the store. `None` when the
    /// writes were kept as a layer instead.
    pub outputs: Option<Vec<Entry>>,
    /// The step's workspace upper dir, when kept as a layer and not empty.
    pub ws_layer: Option<PathBuf>,
    /// An output file that contained a secret value (then kept as a layer).
    pub secret_output: Option<String>,
    /// The step's root upper dir and the first path in it, if the step
    /// wrote outside the workspace.
    pub root_layer: Option<(PathBuf, String)>,
    /// Keep::Store: the logs, masked.
    pub stdout: Option<Hash>,
    pub stderr: Option<Hash>,
    pub observer: String,
    pub net_attempts: Option<u64>,
    pub resource_failure: Option<String>,
    /// A stored run (`Keep::Store`) whose stdout, stderr or outputs hold a
    /// placeholder (`surface::PLACEHOLDER_MARKS`): where.
    pub placeholder_in: Option<String>,
    /// The step left something in the shared warm build cache that is not
    /// Go's own cache layout, or an entry holding a secret value: the run is
    /// not recordable and the caller wipes the cache (or the job's layer).
    pub warm_violation: Option<String>,
    /// `GoCache::Layered`: the first path this step put into the job's layer
    /// that is not a copy of the cache's own entry (a new entry, or one
    /// rewritten with other bytes). A later step that reads the layer reads
    /// something no key covers, written by this job.
    pub warm_layer_written: Option<String>,
    pub phases: Phases,
}

#[derive(Default, Debug, Clone)]
pub struct Phases {
    pub mount: Duration,
    pub exec: Duration,
    pub diff: Duration,
}

pub fn execute(x: &Exec) -> anyhow::Result<Executed> {
    let t = Instant::now();
    std::fs::create_dir_all(x.step_dir)?;
    let lower = x.step_dir.join("lower");
    std::fs::create_dir_all(&lower)?;
    let served = sp_fs::serve(x.tree, x.cas, &lower).context("serve workspace")?;
    let mut ws_lowers: Vec<&Path> = x.ws_layers.iter().map(PathBuf::as_path).collect();
    ws_lowers.push(served.mountpoint());
    let upper = x.step_dir.join("upper");
    let ov = sp_fs::overlay_layers(
        &ws_lowers,
        &upper,
        &x.step_dir.join("work"),
        &x.step_dir.join("merged"),
    )
    .context("overlay workspace")?;

    let root_upper = x.step_dir.join("root-upper");
    let root_ov = match x.rootfs {
        Some(r) => {
            let mut lowers: Vec<&Path> = x.root_layers.iter().map(PathBuf::as_path).collect();
            lowers.push(&r.dir);
            Some(
                sp_fs::overlay_layers(
                    &lowers,
                    &root_upper,
                    &x.step_dir.join("root-work"),
                    &x.step_dir.join("root"),
                )
                .context("overlay image root")?,
            )
        }
        None => None,
    };

    let mut mounts: Vec<Mount> = x.binds.clone();
    // Deeper targets after their parents, so a bind over /__w/o/r lands on
    // top of the bind of /__w.
    mounts.sort_by_key(|m| m.dst.components().count());
    mounts.push(Mount {
        src: ov.merged().to_path_buf(),
        dst: PathBuf::from(x.workspace),
        writable: true,
    });
    let ws_rel = |p: &str| Path::new(x.workspace).join(p);
    // The warm build cache under the job's layer: an overlay of its own,
    // mounted for the step.
    let (gocache_dir, gocache_ov) = match &x.gocache {
        GoCache::None => (None, None),
        GoCache::Scratch(d) | GoCache::Shared(d) => (Some(d.clone()), None),
        GoCache::Layered { lower, layer } => {
            std::fs::create_dir_all(lower)
                .with_context(|| format!("create {}", lower.display()))?;
            seed_trim(&layer.join("upper"))?;
            let merged = x.step_dir.join("gocache");
            let ov = sp_fs::overlay_layers(
                &[lower.as_path()],
                &layer.join("upper"),
                &layer.join("work"),
                &merged,
            )
            .context("overlay the warm build cache")?;
            (Some(merged), Some(ov))
        }
    };
    let gocache_checked = matches!(x.gocache, GoCache::Shared(_) | GoCache::Layered { .. });
    if let Some(dir) = &gocache_dir {
        let gocache = x
            .env
            .iter()
            .find(|(k, _)| k == "GOCACHE")
            .map(|(_, v)| v.clone())
            .context("a GOCACHE bind needs GOCACHE in the step env")?;
        std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
        mounts.push(Mount {
            src: dir.clone(),
            dst: gocache.clone().into(),
            writable: true,
        });
        if gocache_checked {
            // The fuzz cache is not shared. Go reads it only under `go test
            // -fuzz` (cmd/go passes -test.fuzzcachedir only then), where it
            // is an input no key covers; each step gets an empty one that
            // goes with the step, as on a runner with no cache restored.
            let mount_point = dir.join("fuzz");
            std::fs::create_dir_all(&mount_point)
                .with_context(|| format!("create {}", mount_point.display()))?;
            let fuzz = x.step_dir.join("gofuzz");
            std::fs::create_dir_all(&fuzz).with_context(|| format!("create {}", fuzz.display()))?;
            mounts.push(Mount {
                src: fuzz,
                dst: Path::new(&gocache).join("fuzz"),
                writable: true,
            });
        }
    }
    let snapshots = x.cas.dir().join("snapshots");
    let sealed = if x.seal && x.ws_layers.is_empty() {
        x.env
            .iter()
            .find(|(k, _)| k == "GOMODCACHE")
            .and_then(|(_, m)| seal_modcache(m, x.workspace, x.tree, x.cas, &snapshots, true))
    } else {
        None
    };
    if let Some(s) = &sealed {
        mounts.push(Mount {
            src: s.snapshot.dir.join(&s.prefix),
            dst: ws_rel(&s.prefix),
            writable: false,
        });
        mounts.push(Mount {
            src: ov.merged().join(&s.carve_out),
            dst: ws_rel(&s.carve_out),
            writable: true,
        });
    }
    if root_ov.is_some() {
        // A writable rootfs keeps /tmp in its overlay (sp_sandbox), where a
        // step's temporary files would read as writes outside the workspace
        // and taint the job. /tmp is scratch here, as on a runner.
        let tmp = x.step_dir.join("tmp");
        std::fs::create_dir_all(&tmp).with_context(|| format!("create {}", tmp.display()))?;
        mounts.push(Mount {
            src: tmp,
            dst: "/tmp".into(),
            writable: true,
        });
    }
    let phases_mount = t.elapsed();

    let t = Instant::now();
    let rs = RunSpec {
        cmd: x.cmd.clone(),
        cwd: x.cwd.clone(),
        env: x.env.clone(),
        mounts,
        egress: x.egress,
        stdout: x.step_dir.join("stdout"),
        stderr: x.step_dir.join("stderr"),
        echo: x.echo,
        identity: None,
        uid: x.uid,
        gid: x.gid,
        net_probe: x.net_probe,
        rootfs: root_ov.as_ref().map(|o| sp_sandbox::RootFs {
            dir: o.merged().to_path_buf(),
            writable: true,
        }),
    };
    let warm_before = match (&gocache_dir, gocache_checked) {
        (Some(dir), true) => Some(go_cache_snapshot(dir)),
        _ => None,
    };
    let layer_before = match &x.gocache {
        GoCache::Layered { layer, .. } => Some(go_cache_snapshot(&layer.join("upper"))),
        _ => None,
    };
    let r = sp_sandbox::run(&rs);
    let phases_exec = t.elapsed();
    // Checked before anything is unmounted; a check that fails is a
    // violation (not recorded, the cache wiped), never an early return
    // that would skip the unmounting below.
    let warm_violation = match (&gocache_dir, &warm_before) {
        (Some(dir), Some(before)) => go_cache_delta(dir, before, x.masks).unwrap_or_else(|e| {
            Some(format!(
                "what it wrote into the warm build cache could not be checked ({e:#})"
            ))
        }),
        _ => None,
    };
    let warm_layer_written = match (&x.gocache, &layer_before) {
        (GoCache::Layered { lower, layer }, Some(before)) => {
            layer_written(lower, &layer.join("upper"), before)
        }
        _ => None,
    };
    // Unmounted here, before the workspace: a failure is a violation (the
    // caller discards the layer), never an early return.
    let warm_violation = match gocache_ov.map(unmount_patiently) {
        Some(Err(e)) => warm_violation.or(Some(format!(
            "the job's layer over the warm build cache could not be unmounted ({e})"
        ))),
        _ => warm_violation,
    };

    // Unmount everything whatever happened in the sandbox.
    let t = Instant::now();
    let mut secret_output = None;
    let (outputs, ws_layer) = match (x.keep, &r) {
        (Keep::Store, Ok(_)) => match secrets::file_containing(&upper, x.masks)? {
            Some(f) => {
                secret_output = Some(
                    f.strip_prefix(&upper)
                        .unwrap_or(&f)
                        .to_string_lossy()
                        .into_owned(),
                );
                ov.release().context("unmount workspace overlay")?;
                (None, Some(upper.clone()))
            }
            None => (
                Some(ov.finish(x.tree, x.cas).context("diff workspace outputs")?),
                None,
            ),
        },
        _ => {
            ov.release().context("unmount workspace overlay")?;
            let wrote = std::fs::read_dir(&upper).is_ok_and(|mut d| d.next().is_some());
            (None, wrote.then(|| upper.clone()))
        }
    };
    let stats = served.stats();
    let mut log = served.unmount().context("unmount observer")?;
    if let Some(s) = &sealed {
        log.extend(sealed_accesses(x.tree, &s.prefix, &s.carve_out));
        let removed = snapshot::gc(&snapshots, snapshot::KEEP, &s.snapshot.dir);
        if removed > 0 {
            tracing::info!(target: "stillpass", removed, "snapshot gc");
        }
    }
    if x.gocache != GoCache::None && x.keep == Keep::Store {
        // What a cached vet verdict may stand in for (see sp_key::go_source_file).
        sp_manifest::record_children(x.tree, &mut log, sp_key::go_source_file);
    }
    let mut root_layer = None;
    if let Some(o) = root_ov {
        o.release().context("unmount image root overlay")?;
        let skeleton: BTreeSet<PathBuf> = rs
            .mounts
            .iter()
            .flat_map(|m| m.dst.ancestors().map(Path::to_path_buf).collect::<Vec<_>>())
            .chain(["/etc/passwd", "/etc/group"].map(PathBuf::from))
            .collect();
        let mut writes = BTreeSet::new();
        walk_upper(&root_upper, &root_upper, &skeleton, &mut writes)?;
        if let Some(first) = writes.iter().next() {
            root_layer = Some((root_upper.clone(), format!("/{first}")));
        }
    }
    let placeholder_in = match (x.keep, &r) {
        (Keep::Store, Ok(_)) => placeholder_in(&rs.stdout, &rs.stderr, &upper),
        _ => None,
    };
    let r = r?;
    let (stdout, stderr) = match x.keep {
        Keep::Store => {
            let put = |p: &Path| -> anyhow::Result<Hash> {
                let raw = std::fs::read(p).with_context(|| format!("read {}", p.display()))?;
                Ok(x.cas.put(&secrets::mask(&raw, x.masks))?)
            };
            (Some(put(&rs.stdout)?), Some(put(&rs.stderr)?))
        }
        Keep::Layer => (None, None),
    };
    let phases_diff = t.elapsed();
    let observer = format!(
        "fuse{}{}",
        if stats.passthrough {
            "+passthrough"
        } else {
            ""
        },
        if sealed.is_some() { "+sealed" } else { "" }
    );
    Ok(Executed {
        exit_code: r.exit_code,
        signal: r.signal,
        wall: r.wall,
        log,
        outputs,
        ws_layer,
        secret_output,
        root_layer,
        stdout,
        stderr,
        observer,
        net_attempts: r.net_attempts,
        resource_failure: r.resource_failure,
        placeholder_in,
        warm_violation,
        warm_layer_written,
        phases: Phases {
            mount: phases_mount,
            exec: phases_exec,
            diff: phases_diff,
        },
    })
}

/// Every path in a root upper dir that is not a mount point bubblewrap
/// created (or the sandbox's own files), as `/`-relative strings.
fn walk_upper(
    base: &Path,
    dir: &Path,
    skeleton: &BTreeSet<PathBuf>,
    out: &mut BTreeSet<String>,
) -> anyhow::Result<()> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Ok(());
    };
    for item in rd {
        let item = item?;
        let path = item.path();
        let rel = path.strip_prefix(base).unwrap().to_path_buf();
        let as_root = Path::new("/").join(&rel);
        let is_dir = item.file_type()?.is_dir();
        let in_skeleton = skeleton.contains(&as_root);
        let name = rel.as_os_str();
        if name == ".stillpass-started" || name == ".stillpass-net" || name == "tmp" {
            continue;
        }
        if is_dir {
            if !in_skeleton {
                out.insert(rel.to_string_lossy().into_owned());
            }
            walk_upper(base, &path, skeleton, out)?;
        } else if !in_skeleton {
            out.insert(rel.to_string_lossy().into_owned());
        }
    }
    Ok(())
}

/// What a build-cache entry is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CacheKind {
    File,
    Dir,
    Symlink,
    Other,
}

/// A build-cache entry's identity: its kind, and everything a change to it
/// moves. A rewrite moves mtime, which a step can set back, and ctime,
/// which it cannot (the kernel sets it on every data or metadata change);
/// a replacement moves the inode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Ident {
    kind: CacheKind,
    dev: u64,
    ino: u64,
    size: u64,
    mtime: (i64, i64),
    ctime: (i64, i64),
}

/// Every entry under a build cache, by relative path.
fn go_cache_snapshot(dir: &Path) -> BTreeMap<String, Ident> {
    use std::os::unix::fs::MetadataExt;
    let mut out = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            let Ok(m) = std::fs::symlink_metadata(&p) else {
                continue;
            };
            let ft = m.file_type();
            let kind = if ft.is_dir() {
                CacheKind::Dir
            } else if ft.is_symlink() {
                CacheKind::Symlink
            } else if ft.is_file() {
                CacheKind::File
            } else {
                CacheKind::Other
            };
            if kind == CacheKind::Dir {
                stack.push(p.clone());
            }
            let rel = p.strip_prefix(dir).unwrap().to_string_lossy().into_owned();
            out.insert(
                rel,
                Ident {
                    kind,
                    dev: m.dev(),
                    ino: m.ino(),
                    size: m.size(),
                    mtime: (m.mtime(), m.mtime_nsec()),
                    ctime: (m.ctime(), m.ctime_nsec()),
                },
            );
        }
    }
    out
}

/// Go's own build-cache layout, read from cmd/go/internal/cache in the
/// image's Go (1.27.3): `README`, `trim.txt` and `testexpire.txt` (`go
/// clean -testcache`) at the top; two-hex-digit directories holding `<64
/// hex>-a` (action) and `<64 hex>-d` (output) entries, an output being a
/// file, or for an executable `go run` or `go tool` caches (PutExecutable,
/// Go 1.24 and later) a directory holding that one executable; and `fuzz`,
/// the fuzz cache, which stays empty here (each step gets its own, bound
/// over it: see `execute`). Nothing else: Go has no `testexpand`.
fn go_cache_path_ok(rel: &str, is_dir: bool) -> bool {
    let hex = |s: &str, n: usize| {
        s.len() == n
            && s.bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    };
    let entry = |e: &str, suffix: &str| e.strip_suffix(suffix).is_some_and(|id| hex(id, 64));
    let parts: Vec<&str> = rel.split('/').collect();
    match parts.as_slice() {
        ["README" | "trim.txt" | "testexpire.txt"] => !is_dir,
        ["fuzz"] => is_dir,
        [hh] => is_dir && hex(hh, 2),
        [hh, e] => hex(hh, 2) && (entry(e, "-d") || (!is_dir && entry(e, "-a"))),
        [hh, e, exe] => !is_dir && hex(hh, 2) && entry(e, "-d") && !matches!(*exe, "" | "." | ".."),
        _ => false,
    }
}

/// What a step changed in the shared warm build cache, checked: every new or
/// rewritten path in Go's layout (anything else is a step using the cache
/// as a side channel to later steps and jobs), no symlink, and no entry
/// holding one of the step's secret values (`go run -ldflags -X` caches the
/// value in an object file). `Some(why)` when the run must not be recorded.
fn go_cache_delta(
    dir: &Path,
    before: &BTreeMap<String, Ident>,
    masks: &[Vec<u8>],
) -> anyhow::Result<Option<String>> {
    let after = go_cache_snapshot(dir);
    let changed: Vec<(&String, &Ident)> = after
        .iter()
        .filter(|(p, v)| before.get(*p) != Some(*v))
        .collect();
    for (p, v) in &changed {
        let ok = match v.kind {
            CacheKind::File => go_cache_path_ok(p, false),
            CacheKind::Dir => go_cache_path_ok(p, true),
            // Go never makes a symlink, a fifo or a device node.
            CacheKind::Symlink | CacheKind::Other => false,
        };
        if !ok {
            return Ok(Some(format!(
                "it wrote {p} into the warm build cache, which is not Go's cache layout"
            )));
        }
    }
    // An executable's output directory holds that executable and nothing
    // else. At most one: Go makes the directory, then the file, and a
    // concurrent job's step can look in between.
    let exe_dirs: BTreeSet<&str> = changed
        .iter()
        .filter_map(|(p, v)| match p.matches('/').count() {
            1 if v.kind == CacheKind::Dir => Some(p.as_str()),
            2 => p.rsplit_once('/').map(|(d, _)| d),
            _ => None,
        })
        .collect();
    for d in exe_dirs {
        let prefix = format!("{d}/");
        let inside: Vec<&Ident> = after
            .range(prefix.clone()..)
            .take_while(|(p, _)| p.starts_with(&prefix))
            .map(|(_, v)| v)
            .collect();
        if inside.len() > 1 || inside.iter().any(|v| v.kind != CacheKind::File) {
            return Ok(Some(format!(
                "it left {d} in the warm build cache holding more than one executable file, which is not Go's cache layout"
            )));
        }
    }
    if !masks.is_empty() {
        let ac = aho_corasick::AhoCorasick::new(masks).context("mask patterns")?;
        for (p, v) in &changed {
            if v.kind != CacheKind::File {
                continue;
            }
            let Ok(f) = std::fs::File::open(dir.join(p)) else {
                continue;
            };
            if ac.stream_find_iter(f).next().is_some() {
                return Ok(Some(format!(
                    "the warm build cache entry {p} holds one of the step's secret values"
                )));
            }
        }
    }
    Ok(None)
}

/// Where a run's stdout, stderr or workspace writes (`upper`) hold a
/// placeholder. Unreadable is as good as holding one.
fn placeholder_in(stdout: &Path, stderr: &Path, upper: &Path) -> Option<String> {
    let marks = crate::surface::placeholder_marks();
    let Ok(ac) = aho_corasick::AhoCorasick::new(&marks) else {
        return Some("its result (the placeholder check failed)".into());
    };
    let raw = |p: &Path| match std::fs::read(p) {
        Ok(b) => ac.find(&b).is_some(),
        Err(e) => e.kind() != std::io::ErrorKind::NotFound,
    };
    if raw(stdout) {
        return Some("its stdout".into());
    }
    if raw(stderr) {
        return Some("its stderr".into());
    }
    match secrets::file_containing(upper, &marks) {
        Ok(None) => None,
        Ok(Some(f)) => Some(format!(
            "its output {}",
            f.strip_prefix(upper).unwrap_or(&f).display()
        )),
        Err(e) => Some(format!("its outputs (unreadable: {e:#})")),
    }
}

/// Unmounts an overlay that may be busy for a moment: a process another
/// thread of this one forks holds a copy of any descriptor open at that
/// instant (one into the mount included) until it execs, and the kernel
/// refuses the unmount meanwhile (EBUSY). Tries for up to a second.
fn unmount_patiently(mut ov: sp_fs::Overlay) -> Result<(), sp_fs::FsError> {
    let mut tries = 0;
    loop {
        match ov.unmount() {
            Ok(()) => return Ok(()),
            Err(_) if tries < 50 => {
                tries += 1;
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => return Err(e),
        }
    }
}

/// Go trims its cache at most once a day, by the time in `trim.txt`, and a
/// trim's deletions in a job's layer would be the job's own writes there
/// (every later step unrecorded). The layer says the cache was trimmed
/// just now: a job that is not a writer never trims a cache it does not
/// own; a writer, which works on the cache itself, does.
fn seed_trim(upper: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(upper).with_context(|| format!("create {}", upper.display()))?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let p = upper.join("trim.txt");
    std::fs::write(&p, now.to_string()).with_context(|| format!("write {}", p.display()))
}

/// The first entry the step changed in the job's layer (`upper`, over the
/// cache `lower`) that is not a plain copy of the cache's own: overlayfs
/// copies an entry up whole when Go merely bumps its mtime (markUsed), and
/// that copy holds the cache's bytes, mode, owner and attributes. Anything
/// else is a write: a new file, other bytes, another mode, owner or
/// extended attribute (overlayfs's own aside), a symlink, a whiteout (an
/// entry deleted: what the cache held is gone), an opaque directory (one
/// removed and made again), a directory whose mode or owner changed. A new
/// directory is not one by itself; its entries count. `trim.txt` is the
/// hooks' own (`seed_trim`).
fn layer_written(lower: &Path, upper: &Path, before: &BTreeMap<String, Ident>) -> Option<String> {
    go_cache_snapshot(upper)
        .into_iter()
        .filter(|(p, v)| before.get(p) != Some(v))
        .filter(|(p, _)| p != "trim.txt")
        .find(|(p, v)| {
            let (l, u) = (lower.join(p), upper.join(p));
            match v.kind {
                CacheKind::Dir => {
                    sp_fs::is_opaque_dir(&u).unwrap_or(true)
                        || std::fs::symlink_metadata(&l).is_ok() && !same_meta(&l, &u)
                }
                CacheKind::File => !same_meta(&l, &u) || !same_bytes(&l, &u),
                CacheKind::Symlink | CacheKind::Other => true,
            }
        })
        .map(|(p, _)| p)
}

/// The same kind, permission bits, owner and extended attributes (but
/// overlayfs's own, `trusted.overlay.*` and `user.overlay.*`).
fn same_meta(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let (Ok(ma), Ok(mb)) = (std::fs::symlink_metadata(a), std::fs::symlink_metadata(b)) else {
        return false;
    };
    let attrs = |p: &Path| {
        sp_fs::xattrs(p).ok().map(|x| {
            x.into_iter()
                .filter(|(k, _)| {
                    !k.starts_with("trusted.overlay.") && !k.starts_with("user.overlay.")
                })
                .collect::<BTreeMap<_, _>>()
        })
    };
    ma.file_type() == mb.file_type()
        && ma.mode() & 0o7777 == mb.mode() & 0o7777
        && (ma.uid(), ma.gid()) == (mb.uid(), mb.gid())
        && matches!((attrs(a), attrs(b)), (Some(x), Some(y)) if x == y)
}

/// Two regular files (never through a symlink) with the same bytes.
fn same_bytes(a: &Path, b: &Path) -> bool {
    use std::io::Read;
    let open = |p: &Path| {
        std::fs::symlink_metadata(p)
            .ok()
            .filter(|m| m.is_file())
            .and_then(|m| std::fs::File::open(p).ok().map(|f| (f, m.len())))
    };
    let (Some((mut fa, la)), Some((mut fb, lb))) = (open(a), open(b)) else {
        return false;
    };
    if la != lb {
        return false;
    }
    let fill = |f: &mut std::fs::File, buf: &mut [u8]| -> std::io::Result<usize> {
        let mut n = 0;
        while n < buf.len() {
            match f.read(&mut buf[n..])? {
                0 => break,
                k => n += k,
            }
        }
        Ok(n)
    };
    let (mut ba, mut bb) = (vec![0u8; 1 << 16], vec![0u8; 1 << 16]);
    loop {
        match (fill(&mut fa, &mut ba), fill(&mut fb, &mut bb)) {
            (Ok(na), Ok(nb)) if na == nb && ba[..na] == bb[..nb] => {
                if na == 0 {
                    return true;
                }
            }
            _ => return false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_warm_build_cache_takes_only_go_s_layout() {
        let id = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        for (ok, is_dir) in [
            ("README".to_string(), false),
            ("trim.txt".to_string(), false),
            ("testexpire.txt".to_string(), false),
            ("fuzz".to_string(), true),
            ("ab".to_string(), true),
            (format!("ab/{id}-a"), false),
            (format!("ab/{id}-d"), false),
            // `go run` and `go tool` executables (Go 1.24+).
            (format!("ab/{id}-d"), true),
            (format!("ab/{id}-d/gh"), false),
        ] {
            assert!(go_cache_path_ok(&ok, is_dir), "{ok}");
        }
        for (bad, is_dir) in [
            ("notes.txt".to_string(), false),
            ("README".to_string(), true),
            ("fuzz".to_string(), false),
            ("ab/payload".to_string(), false),
            (format!("ab/{id}-x"), false),
            (format!("ab/{}-a", &id[1..]), false),
            (format!("AB/{id}-a"), false),
            (format!("ab/{id}-a"), true),
            (format!("ab/{id}-a/inner"), false),
            (format!("ab/{id}-d/inner"), true),
            (format!("ab/{id}-d/inner/deeper"), false),
            (format!("ab/{id}-d/.."), false),
            // Not Go's: there is no testexpand; the fuzz cache stays empty.
            ("testexpand/x/y".to_string(), false),
            (
                "fuzz/example.com/m/FuzzX/0123456789abcdef".to_string(),
                false,
            ),
            ("zz".to_string(), true),
        ] {
            assert!(!go_cache_path_ok(&bad, is_dir), "{bad}");
        }

        let tmp = tempfile::tempdir().unwrap();
        let c = tmp.path();
        std::fs::create_dir_all(c.join("ab")).unwrap();
        std::fs::write(c.join("README"), "go cache").unwrap();
        let before = go_cache_snapshot(c);
        // Go's own writes pass, a secret value in one does not.
        std::fs::write(c.join(format!("ab/{id}-d")), "object code").unwrap();
        assert_eq!(go_cache_delta(c, &before, &[]).unwrap(), None);
        let mask = vec![b"s3cr3t-value-0123".to_vec()];
        std::fs::write(
            c.join(format!("ab/{id}-a")),
            "ldflags -X main.k=s3cr3t-value-0123",
        )
        .unwrap();
        assert!(
            go_cache_delta(c, &before, &mask)
                .unwrap()
                .unwrap()
                .contains("secret"),
            "a cached secret is caught"
        );
        // A same-size rewrite that sets mtime back is still a change (ctime
        // moves), so its content is checked.
        let entry = c.join(format!("ab/{id}-d"));
        std::fs::write(&entry, "object code 0123x").unwrap();
        let mtime = std::fs::metadata(&entry).unwrap().modified().unwrap();
        let before = go_cache_snapshot(c);
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&entry, "s3cr3t-value-0123").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&entry)
            .unwrap()
            .set_modified(mtime)
            .unwrap();
        let m = std::fs::metadata(&entry).unwrap();
        assert_eq!(
            (m.modified().unwrap(), m.len()),
            (mtime, 17),
            "same size, same mtime"
        );
        assert!(
            go_cache_delta(c, &before, &mask)
                .unwrap()
                .is_some_and(|w| w.contains("secret")),
            "a same-size rewrite with its mtime set back is caught"
        );
        std::fs::write(&entry, "object code").unwrap();
        // Anything outside the layout, a symlink included, is caught.
        let before = go_cache_snapshot(c);
        std::fs::write(c.join("stash"), "for the next job").unwrap();
        assert!(
            go_cache_delta(c, &before, &[])
                .unwrap()
                .unwrap()
                .contains("stash")
        );
        std::fs::remove_file(c.join("stash")).unwrap();
        let before = go_cache_snapshot(c);
        std::os::unix::fs::symlink("/etc/passwd", c.join(format!("ab/{}-d", "f".repeat(64))))
            .unwrap();
        assert!(
            go_cache_delta(c, &before, &[]).unwrap().is_some(),
            "a symlink is caught"
        );
        std::fs::remove_file(c.join(format!("ab/{}-d", "f".repeat(64)))).unwrap();

        // An executable `go run` cached: a -d directory holding it passes;
        // a second file, a directory or a symlink beside it does not.
        let exe = c.join(format!("ab/{}-d", "e".repeat(64)));
        let before = go_cache_snapshot(c);
        std::fs::create_dir(&exe).unwrap();
        std::fs::write(exe.join("hello"), "ELF").unwrap();
        assert_eq!(go_cache_delta(c, &before, &[]).unwrap(), None);
        let before = go_cache_snapshot(c);
        std::fs::write(exe.join("stash"), "for the next job").unwrap();
        assert!(
            go_cache_delta(c, &before, &[])
                .unwrap()
                .is_some_and(|w| w.contains("more than one")),
            "a second file beside the executable"
        );
        std::fs::remove_file(exe.join("stash")).unwrap();
        let before = go_cache_snapshot(c);
        std::os::unix::fs::symlink("/etc/passwd", exe.join("link")).unwrap();
        assert!(
            go_cache_delta(c, &before, &[]).unwrap().is_some(),
            "a symlink beside it"
        );
        std::fs::remove_file(exe.join("link")).unwrap();
        let before = go_cache_snapshot(c);
        std::fs::create_dir(exe.join("sub")).unwrap();
        assert!(
            go_cache_delta(c, &before, &[]).unwrap().is_some(),
            "a directory beside it"
        );
        std::fs::remove_dir(exe.join("sub")).unwrap();
        // A secret compiled into a cached executable is caught too.
        let before = go_cache_snapshot(c);
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(exe.join("hello"), "ELF s3cr3t-value-0123").unwrap();
        assert!(
            go_cache_delta(c, &before, &mask)
                .unwrap()
                .is_some_and(|w| w.contains("secret"))
        );
        // The fuzz cache's mount point stays empty: anything in it is not
        // Go writing through the per-step bind.
        std::fs::create_dir(c.join("fuzz")).unwrap();
        let before = go_cache_snapshot(c);
        std::fs::create_dir_all(c.join("fuzz/example.com/m/FuzzX")).unwrap();
        assert!(
            go_cache_delta(c, &before, &[]).unwrap().is_some(),
            "fuzz/ written"
        );
    }

    /// N2, through a real overlay (root; STILLPASS_INTEGRATION=1, as
    /// scripts/dev.sh sets): only a plain copy-up of the cache's own entry
    /// is not a write to the job's layer.
    #[test]
    fn a_layer_entry_counts_as_written_unless_it_is_a_plain_copy_up() {
        if std::env::var("STILLPASS_INTEGRATION").as_deref() != Ok("1") {
            eprintln!("needs STILLPASS_INTEGRATION=1 (overlayfs); skipping");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let (lower, layer) = (tmp.path().join("lower"), tmp.path().join("layer"));
        let id = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        for hh in ["ab", "cd", "ef"] {
            std::fs::create_dir_all(lower.join(hh)).unwrap();
            std::fs::write(lower.join(format!("{hh}/{id}-d")), "object code").unwrap();
            std::fs::write(lower.join(format!("{hh}/{id}-a")), "v1 action").unwrap();
        }
        std::fs::write(lower.join("trim.txt"), "0").unwrap();
        let upper = layer.join("upper");
        let step = |n: usize, act: &dyn Fn(&Path)| -> Option<String> {
            seed_trim(&upper).unwrap();
            let merged = tmp.path().join(format!("merged-{n}"));
            let ov =
                sp_fs::overlay_layers(&[lower.as_path()], &upper, &layer.join("work"), &merged)
                    .unwrap();
            let before = go_cache_snapshot(&upper);
            act(&merged);
            let w = layer_written(&lower, &upper, &before);
            // Other tests of this binary fork processes concurrently.
            unmount_patiently(ov).unwrap();
            w
        };
        let set_mtime = |p: &Path| {
            std::fs::File::options()
                .write(true)
                .open(p)
                .unwrap()
                .set_modified(std::time::SystemTime::now())
                .unwrap()
        };
        // Go's mark-used bump: copied up whole, the cache's bytes. Not a
        // write; nor is the hooks' trim.txt, nor Go reading it.
        let d = format!("ab/{id}-d");
        assert_eq!(step(0, &|m| set_mtime(&m.join(&d))), None);
        assert!(upper.join(&d).is_file(), "copied up");
        assert_eq!(
            step(1, &|m| {
                std::fs::read(m.join("trim.txt")).unwrap();
            }),
            None
        );
        // Other bytes, another mode, an extended attribute: writes.
        let a = format!("ab/{id}-a");
        assert_eq!(
            step(2, &|m| std::fs::write(m.join(&a), "v1 poison").unwrap()).as_deref(),
            Some(a.as_str())
        );
        std::fs::remove_dir_all(&layer).unwrap();
        let cd = format!("cd/{id}-d");
        assert_eq!(
            step(3, &|m| {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(m.join(&cd), std::fs::Permissions::from_mode(0o600))
                    .unwrap()
            })
            .as_deref(),
            Some(cd.as_str())
        );
        std::fs::remove_dir_all(&layer).unwrap();
        let setxattr = |p: &Path| {
            let ok = std::process::Command::new("python3")
                .args([
                    "-c",
                    "import os, sys; os.setxattr(sys.argv[1], 'user.stillpass', b'x')",
                ])
                .arg(p)
                .status()
                .is_ok_and(|s| s.success());
            assert!(ok, "set a user xattr on {}", p.display());
        };
        assert_eq!(
            step(4, &|m| setxattr(&m.join(&cd))).as_deref(),
            Some(cd.as_str())
        );
        std::fs::remove_dir_all(&layer).unwrap();
        // A deletion (a whiteout), a directory removed and made again (opaque).
        let ef = format!("ef/{id}-a");
        assert_eq!(
            step(5, &|m| std::fs::remove_file(m.join(&ef)).unwrap()).as_deref(),
            Some(ef.as_str())
        );
        std::fs::remove_dir_all(&layer).unwrap();
        assert_eq!(
            step(6, &|m| {
                std::fs::remove_dir_all(m.join("ef")).unwrap();
                std::fs::create_dir(m.join("ef")).unwrap();
            })
            .as_deref(),
            Some("ef")
        );
        std::fs::remove_dir_all(&layer).unwrap();
        // A new entry, a symlink: writes. A new directory alone is not.
        assert_eq!(
            step(7, &|m| std::fs::create_dir(m.join("12")).unwrap()),
            None
        );
        assert!(
            step(8, &|m| std::fs::write(m.join(format!("12/{id}-a")), "v1")
                .unwrap())
            .is_some()
        );
        std::fs::remove_dir_all(&layer).unwrap();
        assert!(
            step(9, &|m| std::os::unix::fs::symlink(
                "/etc/passwd",
                m.join("ab/link")
            )
            .unwrap())
            .is_some()
        );
    }

    #[test]
    fn upper_walk_skips_the_mount_skeleton() {
        let tmp = tempfile::tempdir().unwrap();
        let up = tmp.path();
        std::fs::create_dir_all(up.join("__w/o/r")).unwrap();
        std::fs::create_dir_all(up.join("github/home")).unwrap();
        std::fs::create_dir_all(up.join("go/pkg/mod")).unwrap();
        std::fs::write(up.join("go/pkg/mod/x"), "x").unwrap();
        std::fs::write(up.join(".stillpass-started"), "1").unwrap();
        std::fs::write(up.join(".stillpass-net"), "").unwrap();
        std::fs::create_dir_all(up.join("tmp/scratch")).unwrap();
        let skeleton: BTreeSet<PathBuf> = ["/__w/o/r", "/github/home"]
            .iter()
            .flat_map(|p| {
                Path::new(p)
                    .ancestors()
                    .map(Path::to_path_buf)
                    .collect::<Vec<_>>()
            })
            .collect();
        let mut out = BTreeSet::new();
        walk_upper(up, up, &skeleton, &mut out).unwrap();
        assert_eq!(
            out.into_iter().collect::<Vec<_>>(),
            ["go", "go/pkg", "go/pkg/mod", "go/pkg/mod/x"]
        );
    }
}
