use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use clap::Args;
use sp_cas::{Cas, Hash, Tree};
use sp_key::{Image, StepSpec, Toolchain, step_key};
use sp_ledger::{Ledger, Lookup, Observed, now_rfc3339};
use sp_manifest::{Manifest, StepResult};
use sp_replay::{fmt_duration, group};
use sp_sandbox::{Mount, RunSpec};

use crate::rootfs::{self, Rootfs};
use crate::seal;
use crate::snapshot;
use crate::state::State;

#[derive(Args)]
pub struct RunArgs {
    /// The workspace: the repo checkout, plus outputs of earlier steps.
    #[arg(long)]
    repo: PathBuf,
    /// Expected HEAD of --repo; refused if the checkout differs.
    #[arg(long)]
    commit: Option<String>,
    /// The step script, run as /bin/sh -e -c <script>.
    #[arg(long)]
    step: String,
    /// Allow network. Such a step runs directly on --repo (its writes
    /// persist, e.g. a module cache) and is never memoized.
    #[arg(long)]
    allow_net: bool,
    /// Run inside a Firecracker microVM (0b).
    #[arg(long)]
    vm: bool,
    /// The toolchain profile (go, node, python): the step's fixed env, its
    /// sealed dependency tree and its cache rules. Detected from the repo's
    /// top level (go.mod, package.json, pyproject.toml, ...) when omitted.
    #[arg(long)]
    toolchain: Option<Toolchain>,
    /// Extra environment for the step key, NAME=VALUE (repeatable).
    #[arg(long = "env", value_name = "NAME=VALUE")]
    env: Vec<String>,
    #[arg(long)]
    ledger: Option<PathBuf>,
    #[arg(long)]
    cas: Option<PathBuf>,
    /// Also write the step's outputs into this directory (on execute and on
    /// replay). By default the step's workspace is discarded after the step.
    #[arg(long)]
    outputs: Option<PathBuf>,
    /// Warm state: start the tool with the cache earlier warm runs left
    /// behind (a host directory bound over the profile's cache dir, GOCACHE
    /// for Go) instead of an empty one, so only what changed is rebuilt.
    /// Not observed and not an output: the cache is keyed by content
    /// hashes, so a correct cache cannot change a result (see DECISIONS.md).
    /// Uses its own step key: the profile's knobs turn off the parts of the
    /// cache keyed by path or mtime (for Go, GOFLAGS gains -count=1 and
    /// GODEBUG goindex=0). Refused for a profile without warm state;
    /// ignored with --allow-net.
    #[arg(long)]
    warm: bool,
    /// Keep the run directory (lower, upper, merged) after the step. Output
    /// files are moved into the CAS at step end, so the upper keeps only
    /// directories, symlinks, whiteouts and duplicates of existing blobs.
    #[arg(long)]
    keep: bool,
    /// Run in this pinned root filesystem instead of on the host's: a rootfs
    /// digest (b3:...) or the OCI digest (sha256:...) of the image it was
    /// imported from (`stillpass rootfs import`). The key's image is then
    /// the digest, and the proof holds on any host with the same rootfs.
    /// Default: $STILLPASS_ROOTFS.
    #[arg(long, value_name = "DIGEST", conflicts_with = "host_root")]
    rootfs: Option<String>,
    /// Pin the host's own userland (its fingerprint) instead of a rootfs:
    /// the fallback for hosts without one. Such a proof does not travel.
    #[arg(long)]
    host_root: bool,
}

/// Wall time per phase, reported in the run event.
#[derive(Default, Debug)]
pub(crate) struct Phases {
    import: Duration,
    lookup: Duration,
    mount: Duration,
    exec: Duration,
    diff: Duration,
    record: Duration,
}

pub fn run(state: &State, args: RunArgs) -> anyhow::Result<u8> {
    if args.vm {
        bail!(
            "--vm needs the 0b Firecracker backend (T13-T16), which is not built: this host has no /dev/kvm"
        );
    }
    let repo = args
        .repo
        .canonicalize()
        .with_context(|| format!("repo {}", args.repo.display()))?;
    let head = git_head(&repo);
    if let Some(want) = &args.commit
        && head.as_deref() != Some(want.as_str())
    {
        bail!(
            "--commit {want} but {} is at {}",
            repo.display(),
            head.as_deref().unwrap_or("(not a git checkout)")
        );
    }
    let wall_start = Instant::now();
    let extra_env = parse_env(&args.env)?;
    let cas = Cas::open(args.cas.clone().unwrap_or_else(|| state.cas_dir()))?;
    // The environment pin: a rootfs (its tree, checked servable from the
    // store) or the host fingerprint, with the host class.
    // Where run directories (the workspace overlay's upper) live, for the
    // host class's filesystem types; State::run_dir puts them here.
    let runs = state.dir.join("runs");
    let pin = rootfs::pin(&cas, args.rootfs.as_deref(), args.host_root, &runs)?;
    let image = pin.image.clone();
    let toolchain = match args.toolchain {
        Some(t) => t,
        None => detect_toolchain(&repo)?,
    };
    let warm = args.warm && !args.allow_net;
    let spec = if warm {
        StepSpec::new_warm(toolchain, image.clone(), &args.step, &extra_env)?
    } else {
        StepSpec::new(toolchain, image.clone(), &args.step, &extra_env)
    };
    let key = step_key(&spec)?;
    let env = spec.normalized_env()?;
    let run_id = ulid::Ulid::generate().to_string();

    if args.allow_net {
        return run_unobserved(state, &cas, &repo, &spec, env, &run_id, pin.rootfs.as_ref());
    }

    let mut phases = Phases::default();
    let ledger = Ledger::open(&args.ledger.clone().unwrap_or_else(|| state.ledger_path()))?;
    ledger.put_step_key(&key, &sp_key::canonical_key_json(&spec)?)?;

    let t = Instant::now();
    let tree = cas
        .import_dir(&repo)
        .context("import workspace")?
        .without_top_level(".git");
    phases.import = t.elapsed();

    let t = Instant::now();
    let lookup = ledger.find_match(&key, &tree)?;
    phases.lookup = t.elapsed();

    if let Some((manifest, result)) = &lookup.hit {
        let mid = result.manifest_id;
        if let Some(dir) = &args.outputs {
            // Results recorded before a pattern existed may still hold one.
            sp_replay::materialize_outputs(&without_forbidden(spec.profile(), result), &cas, dir)?;
        }
        eprint!(
            "{}",
            sp_replay::receipt(result, manifest.entries.len(), wall_start.elapsed())
        );
        let code = sp_replay::replay_log(
            result,
            &cas,
            &mut std::io::stdout().lock(),
            &mut std::io::stderr().lock(),
        )?;
        // A replay observes nothing: no transport.
        ledger.record_run(
            &run_id,
            head.as_deref(),
            hostname().as_deref(),
            Observed::default(),
        )?;
        emit_event(
            &run_id,
            &key,
            &image,
            Some(&mid),
            true,
            &lookup,
            &phases,
            wall_start.elapsed(),
            code,
            manifest.entries.len(),
            warm,
            None,
        );
        return Ok(exit_byte(code));
    }

    let start = Instant::now();
    let dir = state.run_dir(&run_id);
    let warm_dir = match (warm, &spec.profile().warm) {
        (true, Some(w)) => Some(state.warm_cache_dir(&image.id()?, w.cache_env)),
        _ => None,
    };
    let outcome = execute(
        &cas,
        &tree,
        &spec,
        env,
        &dir,
        warm_dir.as_deref(),
        pin.rootfs.as_ref(),
        &[],
        &mut phases,
    );
    let ex = match outcome {
        Ok(v) => v,
        Err(e) => {
            if !args.keep {
                let _ = sp_cas::remove_any(&dir);
            }
            return Err(e);
        }
    };
    if let Some(sig) = ex.signal {
        // Killed from outside (OOM, timeout, a signal): the step's own
        // result is unknown, so it is reported and never memoized.
        if !args.keep {
            sp_cas::remove_any(&dir)?;
        }
        eprintln!("STILLPASS  EXECUTED  killed by signal {sig}, not memoized");
        eprintln!("  ran {}", fmt_duration(ex.wall));
        return Ok(exit_byte(ex.exit_code));
    }
    if ex.sealed_write_failed {
        // The step reported a write the read-only bind of a sealed
        // dependency tree refused; in CI that write would have succeeded,
        // so this result may not be the step's. Shown, never memoized.
        if !args.keep {
            sp_cas::remove_any(&dir)?;
        }
        eprintln!(
            "STILLPASS  EXECUTED  a write hit a read-only sealed dependency tree (the step reported EROFS), not memoized"
        );
        eprintln!(
            "  a directory the tool writes under a sealed prefix belongs in the profile's carve-outs"
        );
        eprintln!("  ran {}", fmt_duration(ex.wall));
        tracing::warn!(target: "stillpass", run_id, "sealed prefix write failed; not memoized");
        return Ok(exit_byte(ex.exit_code));
    }

    let t = Instant::now();
    let mid = sp_manifest::manifest_id(&ex.manifest)?;
    let result = StepResult {
        v: sp_manifest::VERSION,
        step_key: key,
        manifest_id: mid,
        run_id: run_id.clone(),
        recorded_at: now_rfc3339(),
        exit_code: ex.exit_code,
        stdout: ex.stdout,
        stderr: ex.stderr,
        outputs: ex.outputs,
        root_outputs: ex.root_outputs,
        duration_ms: ex.wall.as_millis() as u64,
    };
    // One transaction: a result is never visible without the run row that
    // says which transport produced it (results_by_transport and
    // purge_transport find results through it).
    ledger.record_execution(
        &run_id,
        head.as_deref(),
        hostname().as_deref(),
        Observed {
            transport: Some(ex.transport),
            observer: Some(&ex.observer),
        },
        &ex.manifest,
        &result,
    )?;
    phases.record = t.elapsed();
    if let Some(out) = &args.outputs {
        sp_replay::materialize_outputs(&without_forbidden(spec.profile(), &result), &cas, out)?;
    }
    if !args.keep {
        sp_cas::remove_any(&dir)?;
    }

    eprint!(
        "{}",
        executed_banner(
            &key,
            &mid,
            &image,
            &ex.manifest,
            result.root_outputs.len(),
            &lookup,
            ex.wall,
            warm_dir.as_deref()
        )
    );
    emit_event(
        &run_id,
        &key,
        &image,
        Some(&mid),
        false,
        &lookup,
        &phases,
        start.elapsed(),
        ex.exit_code,
        ex.manifest.entries.len(),
        warm,
        Some((ex.transport, &ex.observer)),
    );
    Ok(exit_byte(ex.exit_code))
}

/// What one execution produced, before anything is recorded.
pub(crate) struct Executed {
    pub(crate) exit_code: i32,
    pub(crate) signal: Option<i32>,
    pub(crate) manifest: Manifest,
    pub(crate) outputs: Vec<sp_cas::Entry>,
    /// Writes outside the workspace (rootfs mode only), relative to `/`.
    pub(crate) root_outputs: Vec<sp_cas::Entry>,
    pub(crate) wall: Duration,
    pub(crate) stdout: Hash,
    pub(crate) stderr: Hash,
    /// The transport that served the observer: `fuse-uring` or `fuse`.
    pub(crate) transport: &'static str,
    /// How the inputs were observed: the transport and how it ran
    /// (`ObserverStats::transport_detail`: the ring queue threads' nice
    /// value, `+uring-fallback` when the ring was tried and abandoned before
    /// the step so classic served all of it), `+passthrough` when the kernel
    /// read blobs directly and `+sealed` when the module cache was a
    /// snapshot outside the observer.
    pub(crate) observer: String,
    /// The step ran with a sealed prefix and its output reports EROFS.
    pub(crate) sealed_write_failed: bool,
}

/// Serve the tree, overlay it, run the step in the sandbox, diff the outputs.
/// With `warm`, that host directory is bound over the step's GOCACHE, on top
/// of the overlay: Go reads and writes it directly, unobserved, and it
/// persists for the next warm run. With `rootfs`, the step's `/` is a
/// per-step overlay over that rootfs, served from the store, and its writes
/// outside the workspace and /tmp become `root_outputs`; without, the
/// host's system paths are
/// bound read-only. `extra_mounts` are bound as given and never observed:
/// only pinned trees whose digests are in `spec.image.mounts` belong there
/// (`crate::step`).
#[allow(clippy::too_many_arguments)] // two call sites; the inputs of a run, each named
pub(crate) fn execute(
    cas: &Cas,
    tree: &Tree,
    spec: &StepSpec,
    env: Vec<(String, String)>,
    dir: &Path,
    warm: Option<&Path>,
    rootfs: Option<&Rootfs>,
    extra_mounts: &[Mount],
    phases: &mut Phases,
) -> anyhow::Result<Executed> {
    let t = Instant::now();
    let lower = dir.join("lower");
    std::fs::create_dir_all(&lower)?;
    let served = sp_fs::serve(tree, cas, &lower)?;
    let ov = sp_fs::overlay(
        &served,
        &dir.join("upper"),
        &dir.join("work"),
        &dir.join("merged"),
    )?;
    let root = match rootfs {
        Some(r) => Some(rootfs::RootOverlay::mount(r, cas, dir)?),
        None => None,
    };
    let mut mounts = vec![Mount {
        src: ov.merged().to_path_buf(),
        dst: sp_key::WORKSPACE.into(),
        writable: true,
    }];
    mounts.extend(extra_mounts.iter().cloned());
    if let (Some(warm), Some(w)) = (warm, &spec.profile().warm) {
        let cache = env
            .iter()
            .find(|(k, _)| k == w.cache_env)
            .map(|(_, v)| v.clone())
            .with_context(|| format!("warm state needs {} in the step env", w.cache_env))?;
        std::fs::create_dir_all(warm).with_context(|| format!("create {}", warm.display()))?;
        mounts.push(Mount {
            src: warm.to_path_buf(),
            dst: cache.into(),
            writable: true,
        });
    }
    // The warm cache bind and the sealed prefixes are disjoint subtrees of
    // the workspace; all sit on top of the overlay. Snapshots are hard
    // links into the store, so they live in it: same filesystem by
    // construction, and shared by every state dir and bench run that
    // shares the store.
    let snapshots = cas.dir().join("snapshots");
    let sealed = seal::seal(spec.profile(), tree, cas, &snapshots);
    phases.mount = t.elapsed();

    let t = Instant::now();
    for s in &sealed {
        // A dependency tree is most of the observer's traffic and never
        // changes once installed: the step reads a read-only snapshot of
        // exactly the imported content, and the whole subtree is recorded
        // instead of what was touched. Where the tool writes under it (the
        // profile's carve-outs) stays on the observed overlay.
        mounts.extend(seal::mounts(s, ov.merged()).context("sealed prefix mounts")?);
    }
    let rs = RunSpec {
        cmd: spec.cmd.clone(),
        cwd: spec.cwd.clone(),
        env,
        mounts,
        egress: false,
        stdout: dir.join("stdout"),
        stderr: dir.join("stderr"),
        echo: true,
        rootfs: root.as_ref().map(|o| sp_sandbox::RootFs {
            dir: o.merged().to_path_buf(),
            writable: true,
        }),
        identity: None,
    };
    let r = sp_sandbox::run(&rs)?;
    phases.exec = t.elapsed();

    let t = Instant::now();
    let mut outputs = ov.finish(tree, cas)?;
    // mtime-keyed caches never become recorded outputs (see
    // Profile::no_outputs_under); their blobs stay unreferenced in the store.
    let before = outputs.len();
    outputs.retain(|o| !spec.profile().forbids_output(&o.p));
    if outputs.len() < before {
        tracing::info!(target: "stillpass", dropped = before - outputs.len(), "mtime-keyed cache outputs not recorded");
    }
    // The served rootfs is a second FUSE mount whose `observer` event is
    // written when it unmounts: it goes first, so the run's last `observer`
    // event is the workspace's (the bench and the transport tests read the
    // last one).
    let root_outputs = match root {
        Some(o) => o.finish(cas)?,
        None => Vec::new(),
    };
    let stats = served.stats();
    let mut log = served.unmount()?;
    for s in &sealed {
        log.extend(seal::accesses(tree, &s.seal));
    }
    if let Some(s) = sealed.first() {
        // Housekeeping never fails a completed run (gc warns instead).
        let removed = snapshot::gc(&snapshots, snapshot::KEEP, &s.snapshot.dir);
        if removed > 0 {
            tracing::info!(target: "stillpass", removed, "snapshot gc");
        }
    }
    if let (Some(_), Some(w)) = (warm, &spec.profile().warm) {
        // What a cached verdict may stand in for (for Go: vet's reads of a
        // package's ignored files, see sp_key::go_source_file).
        sp_manifest::record_children(tree, &mut log, w.over_record);
    }
    let manifest = Manifest::from_log(spec_key(spec)?, spec.roots.clone(), tree, log)?;
    // Blobs before the result that names them.
    let (stdout, _) = cas.put_file(&dir.join("stdout"))?;
    let (stderr, _) = cas.put_file(&dir.join("stderr"))?;
    phases.diff = t.elapsed();
    let observer = format!(
        "{}{}{}",
        stats.transport_detail(),
        if stats.passthrough {
            "+passthrough"
        } else {
            ""
        },
        seal::label(&sealed)
    );
    Ok(Executed {
        exit_code: r.exit_code,
        signal: r.signal,
        sealed_write_failed: !sealed.is_empty()
            && (reports_erofs(&dir.join("stdout"))? || reports_erofs(&dir.join("stderr"))?),
        manifest,
        outputs,
        root_outputs,
        wall: r.wall,
        stdout,
        stderr,
        transport: stats.transport(),
        observer,
    })
}

/// `result` without outputs under the profile's `no_outputs_under`.
fn without_forbidden(profile: &sp_key::Profile, result: &StepResult) -> StepResult {
    let mut r = result.clone();
    r.outputs.retain(|o| !profile.forbids_output(&o.p));
    r
}

/// Does a captured stream report a write refused for a read-only file
/// system? A sealed prefix is a read-only bind; a tool that writes there
/// (outside the carve-outs) gets EROFS, which in CI it would not. The
/// strerror text and the errno name are what Node, Python, Go and the
/// coreutils print. Conservative: a step that merely prints the words is
/// not memoized either. A tool that swallows the error silently is not
/// caught; its write was then, by its own choice, inessential to it.
fn reports_erofs(path: &Path) -> anyhow::Result<bool> {
    use std::io::Read as _;
    const NEEDLES: [&[u8]; 2] = [b"read-only file system", b"erofs"];
    let keep = NEEDLES.iter().map(|n| n.len()).max().unwrap_or(1) - 1;
    let mut f = std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    // Streamed in chunks (a step's output can be large), carrying the tail
    // of each chunk over so a match across a boundary is still found.
    let mut window: Vec<u8> = Vec::with_capacity(64 << 10);
    let mut buf = vec![0u8; 64 << 10];
    loop {
        let n = f
            .read(&mut buf)
            .with_context(|| format!("read {}", path.display()))?;
        if n == 0 {
            return Ok(false);
        }
        window.extend(buf[..n].iter().map(u8::to_ascii_lowercase));
        if NEEDLES
            .iter()
            .any(|needle| window.windows(needle.len()).any(|w| w == *needle))
        {
            return Ok(true);
        }
        let cut = window.len().saturating_sub(keep);
        window.drain(..cut);
    }
}

fn spec_key(spec: &StepSpec) -> anyhow::Result<Hash> {
    Ok(step_key(spec)?)
}

/// A step with network (step A, the dependency install): runs directly on
/// the repo, never memoized. The profile's download cache, if it has one,
/// is bound from persistent host state so the next install is fast without
/// the cache ever entering a tree. In a rootfs it runs on the served
/// rootfs read-only (its writes outside the workspace fail, /tmp is a
/// tmpfs) with the host's resolver bound in.
fn run_unobserved(
    state: &State,
    cas: &Cas,
    repo: &Path,
    spec: &StepSpec,
    mut env: Vec<(String, String)>,
    run_id: &str,
    rootfs: Option<&Rootfs>,
) -> anyhow::Result<u8> {
    let profile = spec.profile();
    // A network step fetches; let the tool reach its registry.
    for (name, value) in profile.net_env {
        match env.iter_mut().find(|(k, _)| k == name) {
            Some((_, v)) => *v = value.to_string(),
            None => env.push((name.to_string(), value.to_string())),
        }
    }
    let dir = state.run_dir(run_id);
    std::fs::create_dir_all(&dir)?;
    let root = match rootfs {
        Some(r) => Some(rootfs::ReadOnlyRoot::mount(r, cas, &dir)?),
        None => None,
    };
    let mut mounts = vec![Mount {
        src: repo.to_path_buf(),
        dst: sp_key::WORKSPACE.into(),
        writable: true,
    }];
    for rel in profile.setup_cache {
        let name = rel.rsplit('/').next().unwrap_or(rel);
        let host = state.setup_cache_dir(spec.toolchain).join(name);
        std::fs::create_dir_all(&host).with_context(|| format!("create {}", host.display()))?;
        // The mount point must exist in the repo (bubblewrap cannot create
        // it through the bind); it stays behind as an empty directory.
        std::fs::create_dir_all(repo.join(rel))?;
        mounts.push(Mount {
            src: host,
            dst: Path::new(sp_key::WORKSPACE).join(rel),
            writable: true,
        });
    }
    let rs = RunSpec {
        cmd: spec.cmd.clone(),
        cwd: spec.cwd.clone(),
        env,
        mounts,
        egress: true,
        stdout: dir.join("stdout"),
        stderr: dir.join("stderr"),
        echo: true,
        rootfs: root.as_ref().map(|r| sp_sandbox::RootFs {
            dir: r.dir().to_path_buf(),
            writable: false,
        }),
        identity: None,
    };
    let r = sp_sandbox::run(&rs);
    if let Some(root) = root {
        root.unmount()?;
    }
    let r = r?;
    sp_cas::remove_any(&dir)?;
    eprintln!("STILLPASS  EXECUTED  network allowed, not memoized");
    eprintln!("  {}", image_line(&spec.image));
    eprintln!("  ran {}", fmt_duration(r.wall));
    tracing::info!(target: "stillpass", run_id, memoized = false, egress = true, exit_code = r.exit_code, wall_ms = r.wall.as_millis() as u64, "run");
    Ok(exit_byte(r.exit_code))
}

/// The banner's environment line: what pinned the userland, and the
/// platform.
fn image_line(image: &Image) -> String {
    let p = &image.platform;
    let platform = format!("{} linux {} {} cpus", p.arch, p.kernel, p.cpus);
    match image.kind {
        sp_key::ImageKind::Rootfs => {
            format!("image rootfs b3:{}  {platform}", image.digest.short())
        }
        sp_key::ImageKind::Host => format!(
            "image host fingerprint b3:{}  {platform}  (not portable)",
            image.digest.short()
        ),
    }
}

#[allow(clippy::too_many_arguments)] // one banner, every line named
fn executed_banner(
    key: &Hash,
    mid: &Hash,
    image: &Image,
    m: &Manifest,
    root_outputs: usize,
    lookup: &Lookup,
    wall: Duration,
    warm: Option<&Path>,
) -> String {
    let mut s = String::from("STILLPASS  EXECUTED\n");
    s += &format!(
        "  step b3:{}  manifest b3:{}  {} inputs recorded\n",
        key.short(),
        mid.short(),
        group(m.entries.len())
    );
    s += &format!("  {}\n", image_line(image));
    if root_outputs > 0 {
        s += &format!("  {} outputs outside the workspace\n", group(root_outputs));
    }
    if let Some(w) = warm {
        s += &format!("  warm build cache {}\n", w.display());
    }
    match &lookup.closest {
        Some((id, mm)) => {
            s += &format!(
                "  closest manifest b3:{} differed at {}\n",
                id.short(),
                mm.path
            );
            s += &format!("    {mm}\n");
        }
        None if lookup.candidates == 0 => s += "  no manifests for this step key\n",
        None => {}
    }
    s += &format!("  ran {}\n", fmt_duration(wall));
    s
}

#[allow(clippy::too_many_arguments)] // one flat event, every field named
fn emit_event(
    run_id: &str,
    key: &Hash,
    image: &Image,
    manifest: Option<&Hash>,
    hit: bool,
    lookup: &Lookup,
    p: &Phases,
    total: Duration,
    exit_code: i32,
    inputs: usize,
    warm: bool,
    observed: Option<(&str, &str)>,
) {
    let (transport, observer) = (observed.map(|o| o.0), observed.map(|o| o.1));
    let ms = |d: Duration| d.as_secs_f64() * 1000.0;
    tracing::info!(
        target: "stillpass",
        run_id,
        step_key = %key,
        image_kind = %image.kind,
        image = %image.digest,
        arch = %image.platform.arch,
        kernel = %image.platform.kernel,
        cpus = image.platform.cpus,
        manifest_id = manifest.map(|m| m.to_string()),
        hit,
        warm,
        exit_code,
        inputs,
        transport,
        observer,
        candidates = lookup.candidates,
        first_mismatch = lookup.closest.as_ref().map(|(_, m)| m.path.clone()),
        recorded_ms = lookup.hit.as_ref().map(|(_, r)| r.duration_ms),
        import_ms = ms(p.import),
        lookup_ms = ms(p.lookup),
        mount_ms = ms(p.mount),
        exec_ms = ms(p.exec),
        diff_ms = ms(p.diff),
        record_ms = ms(p.record),
        total_ms = ms(total),
        "run"
    );
}

pub fn exit_byte(code: i32) -> u8 {
    u8::try_from(code).unwrap_or(1)
}

fn parse_env(pairs: &[String]) -> anyhow::Result<Vec<(String, String)>> {
    pairs
        .iter()
        .map(|p| match p.split_once('=') {
            Some((k, v)) => Ok((k.to_string(), v.to_string())),
            None => bail!("--env {p:?} is not NAME=VALUE"),
        })
        .collect()
}

/// The toolchain a repo is for, from the names at its top level. A repo
/// with no marker runs under Go's profile, the env every step had before
/// profiles existed (its rules act only on a module cache, which such a
/// repo lacks); markers for several toolchains are an error, since guessing
/// would run the wrong env.
pub fn detect_toolchain(repo: &Path) -> anyhow::Result<Toolchain> {
    let names: Vec<String> = std::fs::read_dir(repo)
        .with_context(|| format!("read {}", repo.display()))?
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .collect();
    match Toolchain::detect(names.iter().map(String::as_str)) {
        Ok(t) => Ok(t),
        Err(sp_key::profile::Detect::None) => Ok(Toolchain::Go),
        Err(e) => Err(e).with_context(|| format!("detect the toolchain of {}", repo.display())),
    }
}

fn git_head(repo: &Path) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn hostname() -> Option<String> {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .ok()
        .map(|s| s.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn erofs_reports_are_found_anywhere_in_a_stream() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("out");
        let check = |bytes: &[u8]| {
            std::fs::write(&p, bytes).unwrap();
            reports_erofs(&p).unwrap()
        };
        assert!(check(b"Error: EROFS: read-only file system, open 'x'\n"));
        assert!(check(b"OSError: [Errno 30] Read-only file system: 'x'"));
        assert!(!check(b"ok  \tpkg\t0.1s\n"));
        assert!(!check(b""));
        // Split across the 64 KiB read boundary.
        let mut big = vec![b'.'; (64 << 10) - 7];
        big.extend_from_slice(b"Read-only file system");
        assert!(check(&big));
    }
}
