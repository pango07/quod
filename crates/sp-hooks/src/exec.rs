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
    /// A host directory bound over GOCACHE.
    pub gocache: Option<PathBuf>,
    /// `gocache` is the scope's warm build cache, shared with later steps
    /// and jobs: what the step leaves in it is checked (`go_cache_delta`).
    pub gocache_shared: bool,
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
    /// The step left something in the shared warm build cache that is not
    /// Go's own cache layout, or an entry holding a secret value: the run is
    /// not recordable and the caller wipes the cache.
    pub warm_violation: Option<String>,
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
    if let Some(dir) = &x.gocache {
        let gocache = x
            .env
            .iter()
            .find(|(k, _)| k == "GOCACHE")
            .map(|(_, v)| v.clone())
            .context("a GOCACHE bind needs GOCACHE in the step env")?;
        std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
        mounts.push(Mount {
            src: dir.clone(),
            dst: gocache.into(),
            writable: true,
        });
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
    let warm_before = match (&x.gocache, x.gocache_shared) {
        (Some(dir), true) => Some(go_cache_snapshot(dir)),
        _ => None,
    };
    let r = sp_sandbox::run(&rs);
    let phases_exec = t.elapsed();
    // Checked before anything is unmounted; a check that fails is a
    // violation (not recorded, the cache wiped), never an early return
    // that would skip the unmounting below.
    let warm_violation = match (&x.gocache, &warm_before) {
        (Some(dir), Some(before)) => go_cache_delta(dir, before, x.masks).unwrap_or_else(|e| {
            Some(format!(
                "what it wrote into the warm build cache could not be checked ({e:#})"
            ))
        }),
        _ => None,
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
    if x.gocache.is_some() && x.keep == Keep::Store {
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
        warm_violation,
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

/// Go's own build-cache layout (cmd/go/internal/cache): `README`,
/// `trim.txt`, two-hex-digit directories holding `<64 hex>-a` (action) and
/// `<64 hex>-d` (output) entries, and `testexpand/`.
fn go_cache_path_ok(rel: &str, is_dir: bool) -> bool {
    let hex = |s: &str, n: usize| {
        s.len() == n
            && s.bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    };
    let mut parts = rel.split('/');
    match (parts.next(), parts.next(), parts.next()) {
        (Some("README" | "trim.txt"), None, None) => !is_dir,
        (Some("testexpand"), ..) => true,
        (Some(hh), None, None) => is_dir && hex(hh, 2),
        (Some(hh), Some(entry), None) => {
            !is_dir
                && hex(hh, 2)
                && entry
                    .strip_suffix("-a")
                    .or_else(|| entry.strip_suffix("-d"))
                    .is_some_and(|id| hex(id, 64))
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_warm_build_cache_takes_only_go_s_layout() {
        let id = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        assert!(go_cache_path_ok("README", false) && go_cache_path_ok("trim.txt", false));
        assert!(go_cache_path_ok("ab", true));
        assert!(go_cache_path_ok(&format!("ab/{id}-a"), false));
        assert!(go_cache_path_ok(&format!("ab/{id}-d"), false));
        assert!(go_cache_path_ok("testexpand/x/y", false));
        for bad in [
            "notes.txt".to_string(),
            "ab/payload".to_string(),
            format!("ab/{id}-x"),
            format!("ab/{}-a", &id[1..]),
            format!("AB/{id}-a"),
            format!("ab/{id}-a/inner"),
            "fuzz/x".to_string(),
            "zz".to_string(),
        ] {
            assert!(!go_cache_path_ok(&bad, false), "{bad}");
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
