use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use clap::Args;
use sp_cas::{Cas, Hash, Kind, Tree};
use sp_key::{StepSpec, step_key};
use sp_ledger::{Ledger, Lookup, now_rfc3339};
use sp_manifest::{Manifest, StepResult};
use sp_replay::{fmt_duration, group};
use sp_sandbox::{Mount, RunSpec};

use crate::snapshot;
use crate::state::State;

const FINGERPRINT_SH: &str = include_str!("../../../scripts/host-fingerprint.sh");

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
    /// Warm state: start Go with the build cache earlier warm runs left
    /// behind (a host directory bound over GOCACHE) instead of an empty one,
    /// so only what changed is recompiled. Not observed and not an output:
    /// Go's cache is keyed by content hashes, so a correct cache cannot
    /// change a result (see DECISIONS.md). Uses its own step key: the env
    /// turns off Go's path-keyed caches (GOFLAGS gains -count=1,
    /// GODEBUG=goindex=0). Ignored with --allow-net.
    #[arg(long)]
    warm: bool,
    /// Keep the run directory (lower, upper, merged) after the step. Output
    /// files are moved into the CAS at step end, so the upper keeps only
    /// directories, symlinks, whiteouts and duplicates of existing blobs.
    #[arg(long)]
    keep: bool,
}

/// Wall time per phase, reported in the run event.
#[derive(Default, Debug)]
struct Phases {
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
    let image = host_image()?;
    let warm = args.warm && !args.allow_net;
    let spec = if warm {
        StepSpec::spike_warm(image, &args.step, &extra_env)?
    } else {
        StepSpec::spike(image, &args.step, &extra_env)
    };
    let key = step_key(&spec)?;
    let env = spec.normalized_env()?;
    let run_id = ulid::Ulid::generate().to_string();

    if args.allow_net {
        return run_unobserved(state, &repo, &spec, env, &run_id);
    }

    let mut phases = Phases::default();
    let cas = Cas::open(args.cas.clone().unwrap_or_else(|| state.cas_dir()))?;
    let ledger = Ledger::open(&args.ledger.clone().unwrap_or_else(|| state.ledger_path()))?;

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
            sp_replay::materialize_outputs(result, &cas, dir)?;
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
        ledger.record_run(&run_id, head.as_deref(), hostname().as_deref())?;
        emit_event(
            &run_id,
            &key,
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
    let warm_dir = warm.then(|| state.warm_gocache_dir(&image));
    let outcome = execute(
        &cas,
        &tree,
        &spec,
        env,
        &dir,
        warm_dir.as_deref(),
        &state.snapshots_dir(),
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

    let t = Instant::now();
    let mid = ledger.put_manifest(&ex.manifest, &run_id)?;
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
        duration_ms: ex.wall.as_millis() as u64,
    };
    ledger.put_result(&result)?;
    ledger.record_run(&run_id, head.as_deref(), hostname().as_deref())?;
    phases.record = t.elapsed();
    if let Some(out) = &args.outputs {
        sp_replay::materialize_outputs(&result, &cas, out)?;
    }
    if !args.keep {
        sp_cas::remove_any(&dir)?;
    }

    eprint!(
        "{}",
        executed_banner(
            &key,
            &mid,
            &ex.manifest,
            &lookup,
            ex.wall,
            warm_dir.as_deref()
        )
    );
    emit_event(
        &run_id,
        &key,
        Some(&mid),
        false,
        &lookup,
        &phases,
        start.elapsed(),
        ex.exit_code,
        ex.manifest.entries.len(),
        warm,
        Some(&ex.observer),
    );
    Ok(exit_byte(ex.exit_code))
}

/// What one execution produced, before anything is recorded.
struct Executed {
    exit_code: i32,
    signal: Option<i32>,
    manifest: Manifest,
    outputs: Vec<sp_cas::Entry>,
    wall: Duration,
    stdout: Hash,
    stderr: Hash,
    /// How the inputs were observed: `fuse`, plus `+passthrough` when the
    /// kernel read blobs directly and `+sealed` when the module cache was a
    /// snapshot outside the observer.
    observer: String,
}

/// Serve the tree, overlay it, run the step in the sandbox, diff the outputs.
/// With `warm`, that host directory is bound over the step's GOCACHE, on top
/// of the overlay: Go reads and writes it directly, unobserved, and it
/// persists for the next warm run.
#[allow(clippy::too_many_arguments)] // one call site; the inputs of a run, each named
fn execute(
    cas: &Cas,
    tree: &Tree,
    spec: &StepSpec,
    env: Vec<(String, String)>,
    dir: &Path,
    warm: Option<&Path>,
    snapshots: &Path,
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
    let mut mounts = vec![Mount {
        src: ov.merged().to_path_buf(),
        dst: sp_key::WORKSPACE.into(),
        writable: true,
    }];
    if let Some(warm) = warm {
        let gocache = env
            .iter()
            .find(|(k, _)| k == "GOCACHE")
            .map(|(_, v)| v.clone())
            .context("warm state needs GOCACHE in the step env")?;
        std::fs::create_dir_all(warm).with_context(|| format!("create {}", warm.display()))?;
        mounts.push(Mount {
            src: warm.to_path_buf(),
            dst: gocache.into(),
            writable: true,
        });
    }
    // The warm GOCACHE bind and the sealed module cache are disjoint
    // subtrees of the workspace; both sit on top of the overlay.
    let sealed = seal_modcache(spec, tree, cas, snapshots);
    phases.mount = t.elapsed();

    let t = Instant::now();
    if let Some(s) = &sealed {
        // The module cache is nine tenths of the observer's traffic and its
        // module@version directories never change once extracted: the step
        // reads a read-only snapshot of exactly the imported content, and
        // the whole subtree is recorded instead of what was touched. go
        // writes under its cache/ directory, so that part stays on the
        // observed overlay.
        let ws = Path::new(sp_key::WORKSPACE);
        mounts.push(Mount {
            src: s.snapshot.join(&s.prefix),
            dst: ws.join(&s.prefix),
            writable: false,
        });
        mounts.push(Mount {
            src: ov.merged().join(&s.carve_out),
            dst: ws.join(&s.carve_out),
            writable: true,
        });
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
        identity: None,
    };
    let r = sp_sandbox::run(&rs)?;
    phases.exec = t.elapsed();

    let t = Instant::now();
    let outputs = ov.finish(tree, cas)?;
    let stats = served.stats();
    let mut log = served.unmount()?;
    if let Some(s) = &sealed {
        log.extend(sealed_accesses(tree, &s.prefix, &s.carve_out));
        let removed = snapshot::gc(snapshots, snapshot::KEEP, &s.snapshot)?;
        if removed > 0 {
            tracing::info!(target: "stillpass", removed, "snapshot gc");
        }
    }
    if warm.is_some() {
        // What a cached vet verdict may stand in for (see sp_key::go_source_file).
        sp_manifest::record_children(tree, &mut log, sp_key::go_source_file);
    }
    let manifest = Manifest::from_log(spec_key(spec)?, spec.roots.clone(), tree, log)?;
    // Blobs before the result that names them.
    let (stdout, _) = cas.put_file(&dir.join("stdout"))?;
    let (stderr, _) = cas.put_file(&dir.join("stderr"))?;
    phases.diff = t.elapsed();
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
        manifest,
        outputs,
        wall: r.wall,
        stdout,
        stderr,
        observer,
    })
}

fn spec_key(spec: &StepSpec) -> anyhow::Result<Hash> {
    Ok(step_key(spec)?)
}

/// A subtree served from a snapshot outside the observer and recorded whole.
struct Sealed {
    /// Workspace-relative path of the subtree.
    prefix: String,
    /// A directory under it that stays observed (the step writes there).
    carve_out: String,
    /// The snapshot directory; the subtree is at `snapshot/prefix`.
    snapshot: PathBuf,
}

/// Seals the step's module cache (GOMODCACHE under the workspace) when the
/// tree has it with its cache/ directory (so go has somewhere to write):
/// the step reads an immutable snapshot of exactly the imported content
/// (see `snapshot`), built once per distinct subtree under `snapshots` and
/// reused across runs and commits. `None`, with a warning, when no snapshot
/// could be made: the module cache is then observed like everything else.
/// STILLPASS_SEAL_MODCACHE=0 turns sealing off (the bench's attribution
/// series).
fn seal_modcache(spec: &StepSpec, tree: &Tree, cas: &Cas, snapshots: &Path) -> Option<Sealed> {
    if std::env::var("STILLPASS_SEAL_MODCACHE").is_ok_and(|v| v == "0") {
        return None;
    }
    let modcache = spec.env.iter().find(|(k, _)| k == "GOMODCACHE")?.1.as_str();
    let prefix = modcache
        .strip_prefix(sp_key::WORKSPACE)?
        .strip_prefix('/')?
        .trim_end_matches('/')
        .to_string();
    let carve_out = format!("{prefix}/cache");
    let has_dir = |p: &str| tree.get(p).is_some_and(|e| e.k == Kind::Dir);
    if !has_dir(&prefix) || !has_dir(&carve_out) {
        return None;
    }
    // The carve-out directory itself is in the snapshot (empty) as the
    // mount point; its contents are not part of the key.
    let (under, carve) = (format!("{prefix}/"), format!("{carve_out}/"));
    let entries: Vec<&sp_cas::Entry> = tree
        .iter()
        .filter(|e| (e.p == prefix || e.p.starts_with(&under)) && !e.p.starts_with(&carve))
        .collect();
    let t = Instant::now();
    match snapshot::ensure(cas, &entries, snapshots) {
        Ok(s) => {
            tracing::info!(target: "stillpass", key = %s.key, reused = s.reused, entries = entries.len(), ms = t.elapsed().as_millis() as u64, "snapshot");
            Some(Sealed {
                prefix,
                carve_out,
                snapshot: s.dir,
            })
        }
        Err(e) => {
            tracing::warn!(error = %e, "module cache not sealed; observing it instead");
            None
        }
    }
}

/// Every tree entry at or under `prefix`, except those under `carve_out`,
/// as accesses: what a sealed subtree records. Over-records by construction:
/// any change under the prefix alters a recorded file, symlink or listing.
fn sealed_accesses(tree: &Tree, prefix: &str, carve_out: &str) -> Vec<sp_manifest::Access> {
    let (under, carve) = (format!("{prefix}/"), format!("{carve_out}/"));
    tree.iter()
        .filter(|e| e.p == prefix || e.p.starts_with(&under))
        .filter(|e| e.p != carve_out && !e.p.starts_with(&carve))
        .map(|e| sp_manifest::Access {
            path: e.p.clone(),
            kind: e.k,
        })
        .collect()
}

/// A step with network: runs directly on the repo, never memoized.
fn run_unobserved(
    state: &State,
    repo: &Path,
    spec: &StepSpec,
    mut env: Vec<(String, String)>,
    run_id: &str,
) -> anyhow::Result<u8> {
    // A network step fetches; let it reach the module proxy.
    for (k, v) in &mut env {
        if k == "GOPROXY" && v == "off" {
            *v = "https://proxy.golang.org,direct".into();
        }
    }
    let dir = state.run_dir(run_id);
    std::fs::create_dir_all(&dir)?;
    let rs = RunSpec {
        cmd: spec.cmd.clone(),
        cwd: spec.cwd.clone(),
        env,
        mounts: vec![Mount {
            src: repo.to_path_buf(),
            dst: sp_key::WORKSPACE.into(),
            writable: true,
        }],
        egress: true,
        stdout: dir.join("stdout"),
        stderr: dir.join("stderr"),
        echo: true,
        identity: None,
    };
    let r = sp_sandbox::run(&rs)?;
    sp_cas::remove_any(&dir)?;
    eprintln!("STILLPASS  EXECUTED  network allowed, not memoized");
    eprintln!("  ran {}", fmt_duration(r.wall));
    tracing::info!(target: "stillpass", run_id, memoized = false, egress = true, exit_code = r.exit_code, wall_ms = r.wall.as_millis() as u64, "run");
    Ok(exit_byte(r.exit_code))
}

fn executed_banner(
    key: &Hash,
    mid: &Hash,
    m: &Manifest,
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
    manifest: Option<&Hash>,
    hit: bool,
    lookup: &Lookup,
    p: &Phases,
    total: Duration,
    exit_code: i32,
    inputs: usize,
    warm: bool,
    observer: Option<&str>,
) {
    let ms = |d: Duration| d.as_secs_f64() * 1000.0;
    tracing::info!(
        target: "stillpass",
        run_id,
        step_key = %key,
        manifest_id = manifest.map(|m| m.to_string()),
        hit,
        warm,
        exit_code,
        inputs,
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

/// The 0a environment pin: BLAKE3 of the host fingerprint script's output.
fn host_image() -> anyhow::Result<Hash> {
    let out = Command::new("/bin/bash")
        .arg("-c")
        .arg(FINGERPRINT_SH)
        .env_clear()
        .env("PATH", "/usr/local/go/bin:/usr/bin:/bin")
        .output()
        .context("run host fingerprint")?;
    if !out.status.success() {
        bail!(
            "host fingerprint failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(Hash::of(&out.stdout))
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
    use sp_cas::Entry;

    #[test]
    fn sealed_accesses_cover_the_prefix_minus_the_carve_out() {
        let tree = Tree::from_leaves([
            Entry::file("go.mod", Hash::of(b"m"), 0o644),
            Entry::file(".stillpass/gomod/a.b/c@v1/x.go", Hash::of(b"x"), 0o444),
            Entry::symlink(".stillpass/gomod/a.b/c@v1/l", "x.go"),
            Entry::file(
                ".stillpass/gomod/cache/download/a.b/c/@v/v1.mod",
                Hash::of(b"d"),
                0o444,
            ),
            Entry::file(".stillpass/gomodx/y", Hash::of(b"y"), 0o644),
        ]);
        let spec = StepSpec::spike(Hash::of(b"i"), "go test ./...", &[]);
        let tmp = tempfile::tempdir().unwrap();
        let cas = Cas::open(tmp.path().join("cas")).unwrap();
        for b in [b"m".as_slice(), b"x", b"d", b"y"] {
            cas.put(b).unwrap();
        }
        let root = tmp.path().join("snapshots");
        let s =
            seal_modcache(&spec, &tree, &cas, &root).expect("GOMODCACHE is under the workspace");
        assert_eq!(
            (s.prefix.as_str(), s.carve_out.as_str()),
            (".stillpass/gomod", ".stillpass/gomod/cache")
        );
        // The snapshot holds the module dirs and an empty carve-out to mount over.
        let snap = &s.snapshot;
        assert_eq!(snap.parent(), Some(root.as_path()));
        assert_eq!(
            std::fs::read(snap.join(".stillpass/gomod/a.b/c@v1/x.go")).unwrap(),
            b"x"
        );
        assert_eq!(
            std::fs::read_dir(snap.join(".stillpass/gomod/cache"))
                .unwrap()
                .count(),
            0
        );
        assert!(!snap.join("go.mod").exists() && !snap.join(".stillpass/gomodx").exists());
        // A second tree with the same module cache reuses it.
        let s2 = seal_modcache(&spec, &tree, &cas, &root).unwrap();
        assert_eq!(&s2.snapshot, snap);
        let got: Vec<(String, Kind)> = sealed_accesses(&tree, &s.prefix, &s.carve_out)
            .into_iter()
            .map(|a| (a.path, a.kind))
            .collect();
        let want: Vec<(String, Kind)> = [
            (".stillpass/gomod", Kind::Dir),
            (".stillpass/gomod/a.b", Kind::Dir),
            (".stillpass/gomod/a.b/c@v1", Kind::Dir),
            (".stillpass/gomod/a.b/c@v1/l", Kind::Symlink),
            (".stillpass/gomod/a.b/c@v1/x.go", Kind::File),
        ]
        .iter()
        .map(|(p, k)| (p.to_string(), *k))
        .collect();
        assert_eq!(got, want);
    }
}
