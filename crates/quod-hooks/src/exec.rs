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
use quod_cas::{Cas, Entry, Hash, Tree};
use quod_manifest::Access;
use quod_sandbox::{Mount, RunSpec};

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
    /// The store the workspace tree is served from.
    pub cas: &'a Cas,
    /// Where `Keep::Store` puts what the step wrote and printed: the step's
    /// own staging store. Its blobs reach `cas` only once the caller has
    /// judged the run recordable (or applies its outputs to the job's
    /// tree), so a run set aside leaves nothing in the shared store.
    pub stage: &'a Cas,
    /// The step was given a classified secret (or the job is opaque): its
    /// workspace writes are never stored, they stay a job-local layer.
    pub secret_given: bool,
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

/// Is a step's workspace upper dir empty (it wrote nothing there)?
fn upper_empty(upper: &Path) -> bool {
    std::fs::read_dir(upper).map_or(true, |mut d| d.next().is_none())
}

pub struct Executed {
    pub exit_code: i32,
    pub signal: Option<i32>,
    pub wall: Duration,
    /// What the step read, after sealing and warm-state over-recording.
    pub log: Vec<Access>,
    /// Files in `log` whose content the step was never served: it looked
    /// them up or stat()ed them through the observer, or they are
    /// over-recorded go sources it never opened. The withheld-name scan
    /// skips them; the manifest records them like any other.
    pub unopened: BTreeSet<String>,
    /// Keep::Store: the workspace outputs, in the store. `None` when the
    /// writes were kept as a layer instead.
    pub outputs: Option<Vec<Entry>>,
    /// The step's workspace upper dir, when kept as a layer and not empty.
    pub ws_layer: Option<PathBuf>,
    /// Why the workspace writes were kept as a layer: an output that held a
    /// secret value (its path, masked), or a step given secrets that wrote.
    pub secret_output: Option<String>,
    /// Values the step registered with `::add-mask::` (stdout or stderr).
    pub add_masks: Vec<String>,
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
    // Every open must reach the observer here: the withheld-name scan
    // exempts files the step was never served (`Observed::opened`), and
    // zero-message opens would leave only the first OPEN and the READs.
    let opts = quod_fs::Options {
        zero_open: false,
        ..quod_fs::Options::from_env()
    };
    let served = quod_fs::serve_with(x.tree, x.cas, &lower, &opts).context("serve workspace")?;
    let mut ws_lowers: Vec<&Path> = x.ws_layers.iter().map(PathBuf::as_path).collect();
    ws_lowers.push(served.mountpoint());
    let upper = x.step_dir.join("upper");
    let mut ov = quod_fs::overlay_layers(
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
                quod_fs::overlay_layers(
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
            let ov = quod_fs::overlay_layers(
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
        // A writable rootfs keeps /tmp in its overlay (quod_sandbox), where a
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
        rootfs: root_ov.as_ref().map(|o| quod_sandbox::RootFs {
            dir: o.merged().to_path_buf(),
            writable: true,
        }),
    };
    // What the step may change, before it runs. In a job's layer that is
    // the layer alone (every write, a copy-up included, lands in its upper
    // dir), so the check costs what the job wrote, not the cache's size;
    // a writer works on the cache itself, which is walked whole.
    let layer_before = match &x.gocache {
        GoCache::Layered { layer, .. } => Some(go_cache_snapshot(&layer.join("upper"))),
        _ => None,
    };
    let warm_before = match (&gocache_dir, gocache_checked, &x.gocache) {
        (_, _, GoCache::Layered { .. }) => layer_before.clone(),
        (Some(dir), true, _) => Some(go_cache_snapshot(dir)),
        _ => None,
    };
    let r = quod_sandbox::run(&rs);
    let phases_exec = t.elapsed();
    // Install-time bytecode the step wrote with its source, made the same
    // bytes on every run (`hash_install_pycs`). Only for what may be stored
    // (the hermetic and network surfaces): the runner's surface keeps
    // exactly what the step wrote. The upper dir is edited unmounted.
    if x.keep == Keep::Store && r.is_ok() {
        ov.unmount().context("unmount workspace overlay")?;
        let n = hash_install_pycs(&upper)?;
        if n > 0 {
            tracing::info!(target: "quod", pycs = n, "install-time bytecode made hash-based");
        }
    }
    // A value the step registered with `::add-mask::` (the runner reads both
    // streams) is a secret from now on: masked in what is stored of this
    // run, and the run's workspace writes are kept as a layer.
    let raw = |p: &Path| std::fs::read(p).unwrap_or_default();
    let mut add_masks = secrets::add_masks(&raw(&rs.stdout));
    add_masks.extend(secrets::add_masks(&raw(&rs.stderr)));
    let mut masks = x.masks.to_vec();
    for v in &add_masks {
        masks.extend(secrets::masks_for(v));
    }
    masks.sort_by_key(|b| std::cmp::Reverse(b.len()));
    masks.dedup();
    let masks = masks.as_slice();
    let secret_given = x.secret_given || !add_masks.is_empty();
    // Before the outputs are diffed: that moves the step's files out of the
    // upper dir into the staging store.
    let placeholder_in = match (x.keep, &r) {
        (Keep::Store, Ok(_)) => placeholder_in(&rs.stdout, &rs.stderr, &upper),
        _ => None,
    };
    let upper_dir = match &x.gocache {
        GoCache::Layered { lower, layer } => Some((lower.clone(), layer.join("upper"))),
        _ => None,
    };
    // The check scans the layer's upper dir in a job's layer (reading the
    // merged view only for what it names), the cache itself for a writer.
    let scan = match &upper_dir {
        Some((_, u)) => Some(u.clone()),
        None => gocache_dir.clone(),
    };
    let view = gocache_dir.clone();
    let (warm_violation, warm_layer_written) = finish_warm_cache(
        warm_before.as_ref().and(scan.as_deref()),
        warm_before.as_ref(),
        masks,
        upper_dir
            .as_ref()
            .zip(layer_before.as_ref())
            .map(|((l, u), b)| (l.as_path(), u.as_path(), b)),
        gocache_ov,
        |scan: &Path, before: &BTreeMap<String, Ident>, masks: &[Vec<u8>]| {
            go_cache_delta_in(view.as_deref().unwrap_or(scan), scan, before, masks)
        },
    );

    // Unmount everything whatever happened in the sandbox.
    let t = Instant::now();
    let mut secret_output = None;
    let (outputs, ws_layer) = match (x.keep, &r) {
        (Keep::Store, Ok(_)) if secret_given && !upper_empty(&upper) => {
            secret_output = Some(match secrets::file_containing(&upper, masks)? {
                Some(f) => format!("an output held a secret value ({})", f.display()),
                None => {
                    "a step given the job's secrets (or in an opaque job) wrote to the workspace"
                        .to_string()
                }
            });
            ov.release().context("unmount workspace overlay")?;
            (None, Some(upper.clone()))
        }
        (Keep::Store, Ok(_)) => match secrets::file_containing(&upper, masks)? {
            Some(f) => {
                secret_output = Some(format!("an output held a secret value ({})", f.display()));
                ov.release().context("unmount workspace overlay")?;
                (None, Some(upper.clone()))
            }
            None => (
                Some(
                    ov.finish(x.tree, x.stage)
                        .context("diff workspace outputs")?,
                ),
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
    let observed = served.unmount_observed().context("unmount observer")?;
    let mut log = observed.accesses;
    // Files the step saw only as attributes (looked up, stat()ed: git
    // status over a served index), never as bytes. Still recorded as their
    // content; the withheld-name scan skips them, since no program can
    // learn a variable's name from a stat.
    let file_paths = |log: &[Access]| -> BTreeSet<String> {
        log.iter()
            .filter(|a| a.kind == quod_cas::Kind::File)
            .map(|a| a.path.clone())
            .collect()
    };
    // Unknown (a mount with zero-message opens, which this one is not):
    // every file counts as opened, so none is exempt from the scan.
    let opened = |p: &String| observed.opened.as_ref().is_none_or(|o| o.contains(p));
    let mut unopened: BTreeSet<String> = file_paths(&log)
        .into_iter()
        .filter(|p| !opened(p))
        .collect();
    if let Some(s) = &sealed {
        log.extend(sealed_accesses(x.tree, &s.prefix, &s.carve_out));
        let removed = snapshot::gc(&snapshots, snapshot::KEEP, &s.snapshot.dir);
        if removed > 0 {
            tracing::info!(target: "quod", removed, "snapshot gc");
        }
    }
    if x.gocache != GoCache::None && x.keep == Keep::Store {
        // What a cached vet verdict may stand in for (see quod_key::go_source_file).
        let before = file_paths(&log);
        quod_manifest::record_children(x.tree, &mut log, quod_key::go_source_file);
        // Recorded so that a change to them misses; no code of theirs runs
        // in this step: whatever the go command compiles it opens (package
        // loading reads every file's header, the action id hashes every
        // compiled file, on a full cache hit too), and a vet verdict is
        // diagnostics, not a program. Only the unopened ones join.
        unopened.extend(
            file_paths(&log)
                .into_iter()
                .filter(|p| !before.contains(p) && !opened(p)),
        );
    }
    // A sealed prefix is bound over the observed tree: the observer never
    // sees it opened, so none of it is exempt from the scan.
    if let Some(s) = &sealed {
        let under = format!("{}/", s.prefix);
        unopened.retain(|p| *p != s.prefix && !p.starts_with(&under));
    }
    let mut root_layer = None;
    if let Some(o) = root_ov {
        o.release().context("unmount image root overlay")?;
        if let Some(r) = x.rootfs {
            drop_image_bytecode(&r.dir, x.root_layers, &root_upper)?;
        }
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
                Ok(x.stage.put(&secrets::mask(&raw, masks))?)
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
        unopened,
        outputs,
        ws_layer,
        secret_output,
        add_masks,
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

/// Takes out of a step's root upper dir the bytecode Python wrote for the
/// image's own modules, and the directories it made or copied up for
/// nothing else: they are not a write outside the workspace.
///
/// The official python image ships its stdlib without `.pyc` files, so the
/// first import of a module writes `__pycache__/<name>.<tag>.pyc` next to
/// it under /usr/local, and the step looked like it wrote outside the
/// workspace: its root upper became a layer and every later step of the
/// job was tainted. Such a file is a pure cache of an image file: a
/// timestamp pyc whose header names the source's mtime and size, which
/// Python checks on every import and recompiles from the source when they
/// differ, so a later step that does not find it behaves exactly as one
/// that does (it compiles the same source with the same interpreter, and
/// writes the same file again). Only that is dropped: a pyc whose source is
/// not the image's own (the step wrote or replaced it, or an earlier step's
/// layer did), whose header does not validate against the image's source,
/// a hash-based pyc, anything not named as Python names its cache, and
/// every other write stay, and still make a layer.
///
/// The header proves the pyc was compiled from the image's source only as
/// far as the source's mtime tells that file apart. An image whose sources
/// carry no mtime information (a reproducible build clamps them all to 0, 1
/// or 1980, or to one SOURCE_DATE_EPOCH) leaves the size alone to check, so
/// for such a source nothing is dropped (`MtimePools::informative`): the
/// step makes a layer, as before this function existed. Residual: a step
/// that writes a pyc whose header validates against an informative mtime
/// but whose code is not its source's (deliberately; Python never does) has
/// it dropped, so a later step runs the source where a container would have
/// run that code.
fn drop_image_bytecode(image: &Path, layers: &[PathBuf], upper: &Path) -> anyhow::Result<()> {
    let mut caches = Vec::new();
    find_pycache_dirs(upper, upper, &mut caches);
    let mut touched: BTreeSet<PathBuf> = BTreeSet::new();
    let mut pools = MtimePools::default();
    for rel_dir in caches {
        let dir = upper.join(&rel_dir);
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for item in rd.flatten() {
            let name = item.file_name().to_string_lossy().into_owned();
            let Some(stem) = pyc_source_stem(&name) else {
                continue;
            };
            let parent = rel_dir.parent().unwrap_or(Path::new(""));
            let src_rel = parent.join(format!("{stem}.py"));
            // The source must be the image's own, untouched by this step or
            // an earlier one.
            if upper_has(upper, &src_rel)
                || layers.iter().any(|l| upper_has(l, &src_rel))
                || layers.iter().any(|l| hides(l, &src_rel))
                || hides(upper, &src_rel)
            {
                continue;
            }
            let Ok(src) = std::fs::symlink_metadata(image.join(&src_rel)) else {
                continue;
            };
            if !src.is_file() || !pools.informative(image, &src_rel, &src) {
                continue;
            }
            let pyc = item.path();
            if valid_timestamp_pyc(&pyc, &src) {
                std::fs::remove_file(&pyc).with_context(|| format!("remove {}", pyc.display()))?;
                touched.insert(rel_dir.clone());
            }
        }
    }
    // The directories that held them, then their parents, while each is
    // empty and adds nothing to the image: a `__pycache__` the step made,
    // or a directory copied up from the image unchanged.
    for rel_dir in touched {
        let mut rel = rel_dir;
        loop {
            let d = upper.join(&rel);
            let empty = std::fs::read_dir(&d).is_ok_and(|mut r| r.next().is_none());
            let opaque = quod_fs::is_opaque_dir(&d).unwrap_or(true);
            let in_image = std::fs::symlink_metadata(image.join(&rel)).ok();
            let made_cache = in_image.is_none()
                && rel.file_name().is_some_and(|n| n == "__pycache__")
                && !layers.iter().any(|l| l.join(&rel).exists());
            let copied_up = in_image
                .as_ref()
                .is_some_and(|i| i.is_dir() && same_owner_and_mode(i, &d));
            if !empty || opaque || !(made_cache || copied_up) {
                break;
            }
            std::fs::remove_dir(&d).with_context(|| format!("remove {}", d.display()))?;
            match rel.parent() {
                Some(p) if !p.as_os_str().is_empty() => rel = p.to_path_buf(),
                _ => break,
            }
        }
    }
    Ok(())
}

/// The latest mtime a reproducible build clamps files to (1980-01-01, the
/// ZIP epoch; Nix uses 1, others 0): at or before it, an mtime says nothing.
const CLAMPED_MTIME: i64 = 315_532_800;

/// Whether an image source's mtime tells it apart from other sources
/// (`drop_image_bytecode`), judged over the `.py` files of the image tree it
/// belongs to (its innermost ancestor named `python*`, such as
/// `usr/local/lib/python3.12`, else its directory), counted once per call.
/// Not informative: an mtime at or before `CLAMPED_MTIME`, or one that 90%
/// or more of that tree's sources share (a build that set them all to its
/// SOURCE_DATE_EPOCH). The official python:3.12-slim image's 1,097 stdlib
/// sources have 5 distinct mtimes, the most common held by 40% (a `make
/// install` over several seconds), so its bytecode is still dropped.
#[derive(Default)]
struct MtimePools {
    /// Tree -> (its `.py` files, how many have each mtime).
    pools: BTreeMap<PathBuf, (usize, BTreeMap<i64, usize>)>,
}

impl MtimePools {
    fn informative(&mut self, image: &Path, src_rel: &Path, src: &std::fs::Metadata) -> bool {
        use std::os::unix::fs::MetadataExt;
        let mtime = src.mtime();
        if mtime <= CLAMPED_MTIME {
            return false;
        }
        let parent = src_rel.parent().unwrap_or(Path::new(""));
        let root = parent
            .ancestors()
            .find(|a| {
                a.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("python"))
            })
            .unwrap_or(parent)
            .to_path_buf();
        let (total, by_mtime) = self.pools.entry(root.clone()).or_insert_with(|| {
            let mut pool = (0, BTreeMap::new());
            count_py_mtimes(&image.join(&root), &mut pool);
            pool
        });
        let same = by_mtime.get(&mtime).copied().unwrap_or(0);
        !(*total >= 2 && same * 10 >= *total * 9)
    }
}

/// Counts the regular `.py` files under `dir` by mtime (symlinks are not
/// followed).
fn count_py_mtimes(dir: &Path, pool: &mut (usize, BTreeMap<i64, usize>)) {
    use std::os::unix::fs::MetadataExt;
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for item in rd.flatten() {
        let Ok(t) = item.file_type() else {
            continue;
        };
        if t.is_dir() {
            count_py_mtimes(&item.path(), pool);
        } else if t.is_file()
            && item.file_name().to_string_lossy().ends_with(".py")
            && let Ok(m) = item.metadata()
        {
            pool.0 += 1;
            *pool.1.entry(m.mtime()).or_insert(0) += 1;
        }
    }
}

/// `__pycache__` directories under `dir`, relative to `base`.
fn find_pycache_dirs(base: &Path, dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for item in rd.flatten() {
        if !item.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let path = item.path();
        if item.file_name() == "__pycache__" {
            out.push(path.strip_prefix(base).unwrap().to_path_buf());
        } else {
            find_pycache_dirs(base, &path, out);
        }
    }
}

/// The module a cache file is named after: `<stem>.<tag>.pyc` or
/// `<stem>.<tag>.opt-<n>.pyc` (PEP 3147, PEP 488), `<tag>` an interpreter
/// tag such as `cpython-312`.
fn pyc_source_stem(name: &str) -> Option<&str> {
    let base = name.strip_suffix(".pyc")?;
    let base = match base.rsplit_once(".opt-") {
        Some((b, n)) if !n.is_empty() && n.bytes().all(|c| c.is_ascii_digit()) => b,
        _ => base,
    };
    let (stem, tag) = base.rsplit_once('.')?;
    let tagged = tag.split_once('-').is_some_and(|(imp, ver)| {
        !imp.is_empty()
            && imp.bytes().all(|c| c.is_ascii_alphanumeric())
            && !ver.is_empty()
            && ver.bytes().all(|c| c.is_ascii_digit())
    });
    (tagged && !stem.is_empty()).then_some(stem)
}

/// Is `pyc` a regular file whose header (PEP 552: magic ending `\r\n`,
/// flags 0, the source's mtime and size, each as 32 bits) validates
/// against `src`, as Python checks it on import?
fn valid_timestamp_pyc(pyc: &Path, src: &std::fs::Metadata) -> bool {
    use std::io::Read;
    use std::os::unix::fs::MetadataExt;
    let Ok(m) = std::fs::symlink_metadata(pyc) else {
        return false;
    };
    if !m.is_file() || m.nlink() != 1 {
        return false;
    }
    let mut h = [0u8; 16];
    let Ok(mut f) = std::fs::File::open(pyc) else {
        return false;
    };
    if f.read_exact(&mut h).is_err() {
        return false;
    }
    let u32_at = |i: usize| u32::from_le_bytes([h[i], h[i + 1], h[i + 2], h[i + 3]]);
    h[2..4] == *b"\r\n"
        && u32_at(4) == 0
        && u32_at(8) == src.mtime() as u32
        && u32_at(12) == src.size() as u32
}

/// Does `upper` hold `rel` itself (a write, or a whiteout for it)?
fn upper_has(upper: &Path, rel: &Path) -> bool {
    std::fs::symlink_metadata(upper.join(rel)).is_ok()
}

/// Does `upper` hide `rel` from below: one of its ancestors is replaced by
/// something else, or is an opaque directory?
fn hides(upper: &Path, rel: &Path) -> bool {
    rel.ancestors()
        .skip(1)
        .filter(|a| !a.as_os_str().is_empty())
        .any(|a| match std::fs::symlink_metadata(upper.join(a)) {
            Ok(m) if m.is_dir() => quod_fs::is_opaque_dir(&upper.join(a)).unwrap_or(true),
            Ok(_) => true,
            Err(_) => false,
        })
}

fn same_owner_and_mode(a: &std::fs::Metadata, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    std::fs::symlink_metadata(b)
        .is_ok_and(|b| b.mode() == a.mode() && b.uid() == a.uid() && b.gid() == a.gid())
}

/// Rewrites, in a step's workspace upper dir, each timestamp pyc the step
/// wrote together with its source as a checked-hash pyc (PEP 552) of the
/// same code; returns how many.
///
/// Why: pip (and uv with UV_COMPILE_BYTECODE) compile what they install with
/// py_compile, and without SOURCE_DATE_EPOCH (GitHub sets none) a timestamp
/// pyc embeds its source's mtime, the install step's wall clock. An install
/// re-runs on every job (it fetches, so it is never replayed), wrote new pyc
/// bytes each time, and every step that imported them read new inputs and
/// missed. A checked-hash pyc holds the source's hash instead: the same
/// bytes for the same source and interpreter.
///
/// Python's own semantics are kept. Only a pyc whose header validates
/// against its source as the step left it (magic ending `\r\n`, flags 0,
/// the source's mtime and size) is rewritten, so the code in it is what
/// Python would have used for that source after the step; a checked-hash
/// pyc is validated on every import against the source's content (Python
/// recompiles it when they differ), as the timestamp one was against the
/// mtime. Later steps see the source with the observed tree's content
/// mtime, not the install's, so without this they recompiled it anyway.
/// The hash is CPython's `importlib.util.source_hash` (SipHash keyed with
/// the magic number: 2-4 up to 3.10, 1-3 from 3.11); a magic number this
/// does not know is left alone, and a wrong hash only makes Python
/// recompile. What differs from GitHub: the pyc's bytes (its header), never
/// what runs.
fn hash_install_pycs(upper: &Path) -> anyhow::Result<usize> {
    let mut caches = Vec::new();
    find_pycache_dirs(upper, upper, &mut caches);
    let mut n = 0;
    for rel_dir in caches {
        let Ok(rd) = std::fs::read_dir(upper.join(&rel_dir)) else {
            continue;
        };
        let parent = rel_dir.parent().unwrap_or(Path::new("")).to_path_buf();
        for item in rd.flatten() {
            let name = item.file_name().to_string_lossy().into_owned();
            let Some(stem) = pyc_source_stem(&name) else {
                continue;
            };
            // The source, written (or copied up) by this step.
            let src = upper.join(&parent).join(format!("{stem}.py"));
            let Ok(sm) = std::fs::symlink_metadata(&src) else {
                continue;
            };
            if sm.is_file() && rehash_pyc(&item.path(), &src, &sm)? {
                n += 1;
            }
        }
    }
    Ok(n)
}

/// `hash_install_pycs` for one pyc and its source.
fn rehash_pyc(pyc: &Path, src: &Path, sm: &std::fs::Metadata) -> anyhow::Result<bool> {
    use std::os::unix::fs::MetadataExt;
    let Ok(m) = std::fs::symlink_metadata(pyc) else {
        return Ok(false);
    };
    if !m.is_file() || m.nlink() != 1 {
        return Ok(false);
    }
    let mut b = std::fs::read(pyc).with_context(|| format!("read {}", pyc.display()))?;
    let u32_at = |b: &[u8], i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
    if b.len() < 16
        || b[2..4] != *b"\r\n"
        || u32_at(&b, 4) != 0
        || u32_at(&b, 8) != sm.mtime() as u32
        || u32_at(&b, 12) != sm.size() as u32
    {
        return Ok(false);
    }
    let source = std::fs::read(src).with_context(|| format!("read {}", src.display()))?;
    if source.len() as u64 != sm.size() {
        return Ok(false);
    }
    let Some(hash) = pyc_source_hash(b[..4].try_into().unwrap(), &source) else {
        return Ok(false);
    };
    // Flags: hash-based (bit 0), checked against the source (bit 1).
    b[4..8].copy_from_slice(&3u32.to_le_bytes());
    b[8..16].copy_from_slice(&hash);
    std::fs::write(pyc, &b).with_context(|| format!("write {}", pyc.display()))?;
    Ok(true)
}

/// CPython's `importlib.util.source_hash(source)` for the interpreter whose
/// pyc magic number is `magic`: SipHash of the source keyed with the magic
/// number (k0, little-endian) and 0 (k1), as 8 little-endian bytes;
/// SipHash-2-4 for 3.7-3.10 (magic 3394-3439), SipHash-1-3 from 3.11
/// (3495 on). `None` for another magic number (an older CPython, 3.11's
/// pre-releases, another implementation).
fn pyc_source_hash(magic: [u8; 4], source: &[u8]) -> Option<[u8; 8]> {
    if magic[2..4] != *b"\r\n" {
        return None;
    }
    let version = u16::from_le_bytes([magic[0], magic[1]]);
    let (c, d) = match version {
        3394..=3439 => (2, 4),
        3495..=3999 => (1, 3),
        _ => return None,
    };
    Some(siphash(c, d, u64::from(u32::from_le_bytes(magic)), 0, source).to_le_bytes())
}

/// SipHash-c-d (Aumasson and Bernstein) of `data` under the key (k0, k1).
fn siphash(c: usize, d: usize, k0: u64, k1: u64, data: &[u8]) -> u64 {
    let mut v = [
        k0 ^ 0x736f_6d65_7073_6575,
        k1 ^ 0x646f_7261_6e64_6f6d,
        k0 ^ 0x6c79_6765_6e65_7261,
        k1 ^ 0x7465_6462_7974_6573,
    ];
    let round = |v: &mut [u64; 4]| {
        v[0] = v[0].wrapping_add(v[1]);
        v[1] = v[1].rotate_left(13) ^ v[0];
        v[0] = v[0].rotate_left(32);
        v[2] = v[2].wrapping_add(v[3]);
        v[3] = v[3].rotate_left(16) ^ v[2];
        v[0] = v[0].wrapping_add(v[3]);
        v[3] = v[3].rotate_left(21) ^ v[0];
        v[2] = v[2].wrapping_add(v[1]);
        v[1] = v[1].rotate_left(17) ^ v[2];
        v[2] = v[2].rotate_left(32);
    };
    let compress = |v: &mut [u64; 4], m: u64| {
        v[3] ^= m;
        for _ in 0..c {
            round(v);
        }
        v[0] ^= m;
    };
    let mut chunks = data.chunks_exact(8);
    for w in &mut chunks {
        compress(&mut v, u64::from_le_bytes(w.try_into().unwrap()));
    }
    let mut last = [0u8; 8];
    last[..chunks.remainder().len()].copy_from_slice(chunks.remainder());
    last[7] = data.len() as u8;
    compress(&mut v, u64::from_le_bytes(last));
    v[2] ^= 0xff;
    for _ in 0..d {
        round(&mut v);
    }
    v[0] ^ v[1] ^ v[2] ^ v[3]
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
        if name == ".quod-started" || name == ".quod-net" || name == "tmp" {
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
#[cfg(test)]
fn go_cache_delta(
    dir: &Path,
    before: &BTreeMap<String, Ident>,
    masks: &[Vec<u8>],
) -> anyhow::Result<Option<String>> {
    go_cache_delta_in(dir, dir, before, masks)
}

/// `go_cache_delta` with what changed found under `scan` against `before`
/// (a snapshot of `scan`), and read through `view`, the cache as the step
/// saw it: `scan` is the cache itself for a writer, the job's layer's upper
/// dir otherwise, where every write lands (new entries, and copy-ups of
/// rewritten ones). A whiteout there is a deletion, which the cache
/// itself shows as an absent entry: not a change checked here (the layer's
/// writes report it, `layer_written`).
fn go_cache_delta_in(
    view: &Path,
    scan: &Path,
    before: &BTreeMap<String, Ident>,
    masks: &[Vec<u8>],
) -> anyhow::Result<Option<String>> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    let whiteout = |p: &str| {
        scan != view
            && std::fs::symlink_metadata(scan.join(p))
                .is_ok_and(|m| m.file_type().is_char_device() && m.rdev() == 0)
    };
    let after = go_cache_snapshot(scan);
    let changed: Vec<(&String, &Ident)> = after
        .iter()
        .filter(|(p, v)| before.get(*p) != Some(*v))
        .filter(|(p, v)| !(v.kind == CacheKind::Other && whiteout(p)))
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
        // What the step sees in it (a layer's upper holds only what this
        // job added, the cache may hold more).
        let inside: Vec<CacheKind> = go_cache_snapshot(&view.join(d))
            .into_values()
            .map(|v| v.kind)
            .collect();
        if inside.len() > 1 || inside.iter().any(|k| *k != CacheKind::File) {
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
            let Ok(f) = std::fs::File::open(view.join(p)) else {
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

/// After a step, the warm build cache: checked (`check`, `go_cache_delta`)
/// while still mounted, the job's layer's writes found, then the layer
/// unmounted, whatever the check did. A check that fails, like an unmount
/// that fails, is a violation (not recorded, the cache or layer wiped),
/// never an early return that would leave the layer mounted. Returns the
/// violation and the layer's first write.
#[allow(clippy::type_complexity)]
fn finish_warm_cache(
    view: Option<&Path>,
    before: Option<&BTreeMap<String, Ident>>,
    masks: &[Vec<u8>],
    layer: Option<(&Path, &Path, &BTreeMap<String, Ident>)>,
    ov: Option<quod_fs::Overlay>,
    check: impl Fn(&Path, &BTreeMap<String, Ident>, &[Vec<u8>]) -> anyhow::Result<Option<String>>,
) -> (Option<String>, Option<String>) {
    let violation = match (view, before) {
        (Some(dir), Some(before)) => check(dir, before, masks).unwrap_or_else(|e| {
            Some(format!(
                "what it wrote into the warm build cache could not be checked ({e:#})"
            ))
        }),
        _ => None,
    };
    let written = layer.and_then(|(lower, upper, before)| layer_written(lower, upper, before));
    let violation = match ov.map(unmount_patiently) {
        Some(Err(e)) => violation.or(Some(format!(
            "the job's layer over the warm build cache could not be unmounted ({e})"
        ))),
        _ => violation,
    };
    (violation, written)
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
fn unmount_patiently(mut ov: quod_fs::Overlay) -> Result<(), quod_fs::FsError> {
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
                    quod_fs::is_opaque_dir(&u).unwrap_or(true)
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
        quod_fs::xattrs(p).ok().map(|x| {
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

    /// A job's layer is checked by what the layer holds, read through the
    /// cache as the step saw it: a second file put into an executable's
    /// output directory the cache already had is caught though the layer
    /// holds only that one file, and a secret in a new entry is found.
    #[test]
    fn a_layer_is_checked_by_what_it_wrote_through_the_merged_view() {
        let id = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let tmp = tempfile::tempdir().unwrap();
        let (view, upper) = (tmp.path().join("merged"), tmp.path().join("upper"));
        let exe = format!("ab/{id}-d");
        std::fs::create_dir_all(view.join(&exe)).unwrap();
        std::fs::write(view.join(&exe).join("gh"), "exe").unwrap();
        std::fs::create_dir_all(&upper).unwrap();
        let before = go_cache_snapshot(&upper);
        assert_eq!(
            go_cache_delta_in(&view, &upper, &before, &[]).unwrap(),
            None
        );
        // The step adds a second file there: it lands in the layer, and the
        // step sees both.
        for d in [&view, &upper] {
            std::fs::create_dir_all(d.join(&exe)).unwrap();
            std::fs::write(d.join(&exe).join("payload"), "x").unwrap();
        }
        let v = go_cache_delta_in(&view, &upper, &before, &[]).unwrap();
        assert!(v.is_some_and(|v| v.contains("more than one executable")));
        // A new action entry with a secret value in it, in a fresh layer.
        let upper2 = tmp.path().join("upper2");
        std::fs::create_dir_all(upper2.join("cd")).unwrap();
        let before2 = go_cache_snapshot(&upper2);
        let a = format!("cd/{id}-a");
        for d in [&view, &upper2] {
            std::fs::create_dir_all(d.join("cd")).unwrap();
            std::fs::write(d.join(&a), "k=s3cr3t-value-0123").unwrap();
        }
        let mask = vec![b"s3cr3t-value-0123".to_vec()];
        let v = go_cache_delta_in(&view, &upper2, &before2, &mask).unwrap();
        assert!(v.is_some_and(|v| v.contains("secret")));
        assert_eq!(
            go_cache_delta_in(&view, &upper2, &before2, &[]).unwrap(),
            None
        );
    }

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

    /// A secret base64-encoded into a `-d` output (an embedded config, a
    /// `-ldflags -X` of an encoded value) is caught like a plain one.
    #[test]
    fn an_encoded_secret_in_a_cache_entry_is_caught() {
        let id = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let tmp = tempfile::tempdir().unwrap();
        let c = tmp.path();
        std::fs::create_dir_all(c.join("ab")).unwrap();
        let before = go_cache_snapshot(c);
        // base64("hunter2/hunter 2")
        std::fs::write(c.join(format!("ab/{id}-d")), "cfg=aHVudGVyMi9odW50ZXIgMg==").unwrap();
        let masks = secrets::masks_for("hunter2/hunter 2");
        assert!(
            go_cache_delta(c, &before, &masks)
                .unwrap()
                .is_some_and(|w| w.contains("secret")),
            "a base64-encoded secret in a -d entry is caught"
        );
    }

    /// N2, through a real overlay (root; QUOD_INTEGRATION=1, as
    /// scripts/dev.sh sets): only a plain copy-up of the cache's own entry
    /// is not a write to the job's layer.
    #[test]
    fn a_layer_entry_counts_as_written_unless_it_is_a_plain_copy_up() {
        if std::env::var("QUOD_INTEGRATION").as_deref() != Ok("1") {
            eprintln!("needs QUOD_INTEGRATION=1 (overlayfs); skipping");
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
                quod_fs::overlay_layers(&[lower.as_path()], &upper, &layer.join("work"), &merged)
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
                    "import os, sys; os.setxattr(sys.argv[1], 'user.quod', b'x')",
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

    /// MINOR 7: a warm-cache check that fails is a violation, and the job's
    /// layer is unmounted all the same (root; QUOD_INTEGRATION=1).
    #[test]
    fn a_failed_check_still_unmounts_the_layer() {
        if std::env::var("QUOD_INTEGRATION").as_deref() != Ok("1") {
            eprintln!("needs QUOD_INTEGRATION=1 (overlayfs); skipping");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let (lower, upper, work, merged) = (
            tmp.path().join("lower"),
            tmp.path().join("upper"),
            tmp.path().join("work"),
            tmp.path().join("merged"),
        );
        std::fs::create_dir_all(&lower).unwrap();
        let ov = quod_fs::overlay_layers(&[lower.as_path()], &upper, &work, &merged).unwrap();
        let mounted = |p: &Path| {
            std::fs::read_to_string("/proc/self/mountinfo")
                .unwrap()
                .lines()
                .any(|l| l.split(' ').nth(4) == Some(p.to_str().unwrap()))
        };
        assert!(mounted(&merged));
        let before = go_cache_snapshot(&merged);
        let layer_before = go_cache_snapshot(&upper);
        let (violation, _) = finish_warm_cache(
            Some(&merged),
            Some(&before),
            &[],
            Some((&lower, &upper, &layer_before)),
            Some(ov),
            |_, _, _| anyhow::bail!("injected"),
        );
        assert!(
            violation.is_some_and(|v| v.contains("could not be checked") && v.contains("injected")),
        );
        assert!(
            !mounted(&merged),
            "the layer is unmounted after a failed check"
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
        std::fs::write(up.join(".quod-started"), "1").unwrap();
        std::fs::write(up.join(".quod-net"), "").unwrap();
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

    /// The python image's stdlib bytecode, written at a step's first import
    /// under /usr/local, is no write outside the workspace; any other write
    /// there still is (the finding on real Python jobs).
    #[test]
    fn image_bytecode_is_no_write_outside_the_workspace() {
        use std::time::{Duration, UNIX_EPOCH};
        let tmp = tempfile::tempdir().unwrap();
        let image = tmp.path().join("image");
        let upper = tmp.path().join("upper");
        let lib = "usr/local/lib/python3.12";
        let src = |rel: &str, body: &str, mtime: u64| {
            let p = image.join(lib).join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, body).unwrap();
            std::fs::File::options()
                .write(true)
                .open(&p)
                .unwrap()
                .set_modified(UNIX_EPOCH + Duration::from_secs(mtime))
                .unwrap();
        };
        src("os.py", "import abc\n", 1_700_000_000);
        src("json/__init__.py", "x = 1\n", 1_700_000_100);
        src("site.py", "y = 2\n", 1_700_000_200);
        src("mod3.py", "z = 3\n", 1_700_000_300);
        std::fs::create_dir_all(image.join(lib).join("__pycache__")).unwrap();
        let pyc = |rel: &str, mtime: u32, size: u32| {
            let p = upper.join(lib).join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            let mut b = vec![0xcb, 0x0d, b'\r', b'\n', 0, 0, 0, 0];
            b.extend_from_slice(&mtime.to_le_bytes());
            b.extend_from_slice(&size.to_le_bytes());
            b.extend_from_slice(b"code");
            std::fs::write(p, b).unwrap();
        };
        // Copied-up directories carry the image's modes, as overlayfs makes
        // them.
        let copy_up = |rel: &str| {
            use std::os::unix::fs::PermissionsExt;
            let mut acc = PathBuf::new();
            for c in Path::new(rel).components() {
                acc.push(c);
                let (i, u) = (image.join(&acc), upper.join(&acc));
                std::fs::create_dir_all(&u).unwrap();
                let mode = std::fs::metadata(&i).unwrap().permissions().mode();
                std::fs::set_permissions(&u, std::fs::Permissions::from_mode(mode)).unwrap();
            }
        };
        copy_up(lib);
        copy_up(&format!("{lib}/json"));
        let writes = |upper: &Path| {
            let mut out = BTreeSet::new();
            walk_upper(upper, upper, &BTreeSet::new(), &mut out).unwrap();
            out.into_iter().collect::<Vec<_>>()
        };
        // Only the image's own bytecode: nothing is left, so no layer.
        pyc("__pycache__/os.cpython-312.pyc", 1_700_000_000, 11);
        pyc(
            "json/__pycache__/__init__.cpython-312.pyc",
            1_700_000_100,
            6,
        );
        pyc("__pycache__/site.cpython-312.opt-1.pyc", 1_700_000_200, 6);
        drop_image_bytecode(&image, &[], &upper).unwrap();
        assert!(writes(&upper).is_empty(), "{:?}", writes(&upper));

        // Anything else stays.
        copy_up(lib);
        pyc("__pycache__/os.cpython-312.pyc", 1_700_000_000, 11);
        // A header that does not validate against the image's source.
        pyc("__pycache__/site.cpython-312.pyc", 1_700_000_201, 6);
        // No source in the image.
        pyc("__pycache__/evil.cpython-312.pyc", 1_700_000_000, 11);
        // A source the step replaced.
        std::fs::write(upper.join(lib).join("mod3.py"), "z = 4\n").unwrap();
        pyc("__pycache__/mod3.cpython-312.pyc", 1_700_000_300, 6);
        // Not named as Python names its cache, and not bytecode at all.
        pyc("__pycache__/os.pyc", 1_700_000_000, 11);
        std::fs::write(upper.join(lib).join("__pycache__/notes.txt"), "n").unwrap();
        // A hash-based pyc (flags 3) is not checked against a timestamp.
        let mut hashed = vec![0xcb, 0x0d, b'\r', b'\n', 3, 0, 0, 0];
        hashed.extend_from_slice(&[0; 8]);
        std::fs::write(
            upper.join(lib).join("__pycache__/json.cpython-312.pyc"),
            hashed,
        )
        .unwrap();
        // A write elsewhere.
        std::fs::create_dir_all(upper.join("etc")).unwrap();
        std::fs::write(upper.join("etc/motd"), "hi").unwrap();
        drop_image_bytecode(&image, &[], &upper).unwrap();
        let w = writes(&upper);
        let p = |r: &str| format!("{lib}/{r}");
        for kept in [
            p("__pycache__/site.cpython-312.pyc"),
            p("__pycache__/evil.cpython-312.pyc"),
            p("mod3.py"),
            p("__pycache__/mod3.cpython-312.pyc"),
            p("__pycache__/os.pyc"),
            p("__pycache__/notes.txt"),
            p("__pycache__/json.cpython-312.pyc"),
            "etc/motd".to_string(),
        ] {
            assert!(w.contains(&kept), "{kept} kept: {w:?}");
        }
        assert!(
            !w.contains(&p("__pycache__/os.cpython-312.pyc")),
            "the image's own os bytecode dropped: {w:?}"
        );

        // A source an earlier step's layer replaced is not the image's.
        let layer = tmp.path().join("layer");
        std::fs::create_dir_all(layer.join(lib)).unwrap();
        std::fs::write(layer.join(lib).join("os.py"), "import abc\n").unwrap();
        let upper2 = tmp.path().join("upper2");
        std::fs::create_dir_all(upper2.join(lib).join("__pycache__")).unwrap();
        std::fs::copy(
            upper.join(lib).join("__pycache__/site.cpython-312.pyc"),
            upper2.join(lib).join("__pycache__/x.cpython-312.pyc"),
        )
        .unwrap();
        let mut b = vec![0xcb, 0x0d, b'\r', b'\n', 0, 0, 0, 0];
        b.extend_from_slice(&1_700_000_000u32.to_le_bytes());
        b.extend_from_slice(&11u32.to_le_bytes());
        std::fs::write(upper2.join(lib).join("__pycache__/os.cpython-312.pyc"), &b).unwrap();
        drop_image_bytecode(&image, &[layer], &upper2).unwrap();
        assert!(
            upper2
                .join(lib)
                .join("__pycache__/os.cpython-312.pyc")
                .is_file()
        );
    }

    /// M4 (Fable's review of 75ebbea): an image whose sources carry no
    /// mtime information (all clamped to one value, or to 0) keeps the
    /// bytecode a step wrote for them: a layer, as without the drop.
    #[test]
    fn uniform_mtime_images_keep_their_bytecode() {
        use std::time::{Duration, UNIX_EPOCH};
        let lib = "usr/local/lib/python3.12";
        let fixture = |mtimes: &[(&str, u64)]| {
            let tmp = tempfile::tempdir().unwrap();
            let (image, upper) = (tmp.path().join("image"), tmp.path().join("upper"));
            for (name, mtime) in mtimes {
                let p = image.join(lib).join(format!("{name}.py"));
                std::fs::create_dir_all(p.parent().unwrap()).unwrap();
                std::fs::write(&p, "x = 1\n").unwrap();
                std::fs::File::options()
                    .write(true)
                    .open(&p)
                    .unwrap()
                    .set_modified(UNIX_EPOCH + Duration::from_secs(*mtime))
                    .unwrap();
                let pyc = upper
                    .join(lib)
                    .join(format!("__pycache__/{name}.cpython-312.pyc"));
                std::fs::create_dir_all(pyc.parent().unwrap()).unwrap();
                let mut b = vec![0xcb, 0x0d, b'\r', b'\n', 0, 0, 0, 0];
                b.extend_from_slice(&(*mtime as u32).to_le_bytes());
                b.extend_from_slice(&6u32.to_le_bytes());
                std::fs::write(pyc, b).unwrap();
            }
            drop_image_bytecode(&image, &[], &upper).unwrap();
            let left: Vec<String> = mtimes
                .iter()
                .map(|(n, _)| n.to_string())
                .filter(|n| {
                    upper
                        .join(lib)
                        .join(format!("__pycache__/{n}.cpython-312.pyc"))
                        .exists()
                })
                .collect();
            (tmp, left)
        };
        let t = 1_700_000_000;
        // Every source has the same mtime (a build's SOURCE_DATE_EPOCH).
        let (_t, left) = fixture(&[("a", t), ("b", t), ("c", t), ("d", t)]);
        assert_eq!(left, ["a", "b", "c", "d"]);
        // Clamped to the epoch, or to 1980: kept even alone.
        let (_t, left) = fixture(&[("z", 0)]);
        assert_eq!(left, ["z"]);
        let (_t, left) = fixture(&[("z", CLAMPED_MTIME as u64)]);
        assert_eq!(left, ["z"]);
        // Informative mtimes (the official image: a few seconds of `make
        // install`, the most common held by well under 90%): dropped.
        let (_t, left) = fixture(&[("a", t), ("b", t), ("c", t + 1), ("d", t + 2)]);
        assert!(left.is_empty(), "{left:?}");
        // On old code (no mtime check) the uniform image's were dropped.
    }

    /// CPython's `importlib.util.source_hash`, as 3.9, 3.10, 3.11, 3.12 and
    /// 3.14 print it (keyed with their magic numbers).
    #[test]
    fn pyc_source_hashes_are_cpythons() {
        let hex = |b: [u8; 8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
        let magic = |h: &str| -> [u8; 4] {
            let v: Vec<u8> = (0..4)
                .map(|i| u8::from_str_radix(&h[2 * i..2 * i + 2], 16).unwrap())
                .collect();
            v.try_into().unwrap()
        };
        let long = "import os\nprint(os.getcwd())\n".repeat(7);
        for (m, src, want) in [
            ("610d0d0a", long.as_str(), "a16a3b2ed40727c6"),
            ("6f0d0d0a", "x = 1\n", "d94aade28c8cbebf"),
            ("a70d0d0a", "x = 1\n", "4c0372aa93f75252"),
            ("cb0d0d0a", "x = 1\n", "1522561917f5df08"),
            ("cb0d0d0a", "", "baf18eedc9387a25"),
            ("cb0d0d0a", long.as_str(), "012632851a9fbc45"),
            ("2b0e0d0a", "x = 1\n", "1db2b6325ca282a7"),
        ] {
            assert_eq!(
                pyc_source_hash(magic(m), src.as_bytes())
                    .map(hex)
                    .as_deref(),
                Some(want),
                "{m} {src:?}"
            );
        }
        // 3.11's pre-releases, pre-3.7, and no magic at all: not known.
        assert_eq!(pyc_source_hash(magic("840d0d0a"), b""), None);
        assert_eq!(pyc_source_hash(magic("330d0d0a"), b""), None);
        assert_eq!(pyc_source_hash(*b"abcd", b""), None);
    }

    /// M2 (Fable's review of 75ebbea): the timestamp pyc an install step
    /// writes with its source becomes a checked-hash pyc of the same code,
    /// the same bytes on every run; anything else is left as written.
    #[test]
    fn install_pycs_become_checked_hash() {
        use std::os::unix::fs::MetadataExt;
        use std::time::{Duration, UNIX_EPOCH};
        let run = |mtime: u64| {
            let tmp = tempfile::tempdir().unwrap();
            let upper = tmp.path().to_path_buf();
            let pkg = upper.join(".venv/lib/python3.12/site-packages/pkg");
            std::fs::create_dir_all(pkg.join("__pycache__")).unwrap();
            let src = pkg.join("mod.py");
            std::fs::write(&src, "x = 1\n").unwrap();
            std::fs::File::options()
                .write(true)
                .open(&src)
                .unwrap()
                .set_modified(UNIX_EPOCH + Duration::from_secs(mtime))
                .unwrap();
            let header = |mtime: u64| {
                let mut b = vec![0xcb, 0x0d, b'\r', b'\n', 0, 0, 0, 0];
                b.extend_from_slice(&(mtime as u32).to_le_bytes());
                b.extend_from_slice(&6u32.to_le_bytes());
                b.extend_from_slice(b"code");
                b
            };
            let pyc = pkg.join("__pycache__/mod.cpython-312.pyc");
            std::fs::write(&pyc, header(mtime)).unwrap();
            // Does not validate against its source: left alone.
            std::fs::write(pkg.join("other.py"), "y = 22\n").unwrap();
            let stale = pkg.join("__pycache__/other.cpython-312.pyc");
            std::fs::write(&stale, header(mtime)).unwrap();
            // No source written by the step: left alone.
            let orphan = pkg.join("__pycache__/gone.cpython-312.pyc");
            std::fs::write(&orphan, header(mtime)).unwrap();
            assert_eq!(hash_install_pycs(&upper).unwrap(), 1);
            let read = |p: &Path| std::fs::read(p).unwrap();
            assert_eq!(read(&stale), header(mtime));
            assert_eq!(read(&orphan), header(mtime));
            assert_eq!(std::fs::metadata(&pyc).unwrap().nlink(), 1);
            (tmp, read(&pyc))
        };
        let (_a, a) = run(1_800_000_000);
        let (_b, b) = run(1_800_000_777);
        assert_eq!(a, b, "the same bytes whenever the install ran");
        let mut want = vec![0xcb, 0x0d, b'\r', b'\n', 3, 0, 0, 0];
        want.extend_from_slice(&0x08df_f517_1956_2215u64.to_le_bytes());
        want.extend_from_slice(b"code");
        assert_eq!(
            a, want,
            "flags 3 (checked hash), CPython 3.12's source hash"
        );
    }

    #[test]
    fn pyc_names() {
        assert_eq!(pyc_source_stem("os.cpython-312.pyc"), Some("os"));
        assert_eq!(pyc_source_stem("a.b.cpython-312.opt-2.pyc"), Some("a.b"));
        assert_eq!(pyc_source_stem("__init__.pypy-73.pyc"), Some("__init__"));
        for no in [
            "os.pyc",
            "os.cpython.pyc",
            "os.cpython-312.py",
            ".cpython-312.pyc",
        ] {
            assert_eq!(pyc_source_stem(no), None, "{no}");
        }
    }
}
