use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use clap::Args;
use sp_cas::{Cas, Hash, Tree};
use sp_key::{StepSpec, step_key};
use sp_ledger::{Ledger, Lookup, now_rfc3339};
use sp_manifest::{Manifest, StepResult};
use sp_replay::{fmt_duration, group};
use sp_sandbox::{Mount, RunSpec};

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
        StepSpec::spike_warm(image, &args.step, &extra_env)
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
        &mut phases,
    );
    let (exit_code, signal, manifest, outputs, wall, stdout, stderr) = match outcome {
        Ok(v) => v,
        Err(e) => {
            if !args.keep {
                let _ = sp_cas::remove_any(&dir);
            }
            return Err(e);
        }
    };
    if let Some(sig) = signal {
        // Killed from outside (OOM, timeout, a signal): the step's own
        // result is unknown, so it is reported and never memoized.
        if !args.keep {
            sp_cas::remove_any(&dir)?;
        }
        eprintln!("STILLPASS  EXECUTED  killed by signal {sig}, not memoized");
        eprintln!("  ran {}", fmt_duration(wall));
        return Ok(exit_byte(exit_code));
    }

    let t = Instant::now();
    let mid = ledger.put_manifest(&manifest, &run_id)?;
    let result = StepResult {
        v: sp_manifest::VERSION,
        step_key: key,
        manifest_id: mid,
        run_id: run_id.clone(),
        recorded_at: now_rfc3339(),
        exit_code,
        stdout,
        stderr,
        outputs,
        duration_ms: wall.as_millis() as u64,
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
        executed_banner(&key, &mid, &manifest, &lookup, wall, warm_dir.as_deref())
    );
    emit_event(
        &run_id,
        &key,
        Some(&mid),
        false,
        &lookup,
        &phases,
        start.elapsed(),
        exit_code,
        manifest.entries.len(),
        warm,
    );
    Ok(exit_byte(exit_code))
}

type Executed = (
    i32,
    Option<i32>,
    Manifest,
    Vec<sp_cas::Entry>,
    Duration,
    Hash,
    Hash,
);

/// Serve the tree, overlay it, run the step in the sandbox, diff the outputs.
/// With `warm`, that host directory is bound over the step's GOCACHE, on top
/// of the overlay: Go reads and writes it directly, unobserved, and it
/// persists for the next warm run.
fn execute(
    cas: &Cas,
    tree: &Tree,
    spec: &StepSpec,
    env: Vec<(String, String)>,
    dir: &Path,
    warm: Option<&Path>,
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
    phases.mount = t.elapsed();

    let t = Instant::now();
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
    let mut log = served.unmount()?;
    if warm.is_some() {
        // What a cached vet verdict may stand in for (see sp_key::go_source_file).
        sp_manifest::record_children(tree, &mut log, sp_key::go_source_file);
    }
    let manifest = Manifest::from_log(spec_key(spec)?, spec.roots.clone(), tree, log)?;
    // Blobs before the result that names them.
    let (stdout, _) = cas.put_file(&dir.join("stdout"))?;
    let (stderr, _) = cas.put_file(&dir.join("stderr"))?;
    phases.diff = t.elapsed();
    Ok((
        r.exit_code,
        r.signal,
        manifest,
        outputs,
        r.wall,
        stdout,
        stderr,
    ))
}

fn spec_key(spec: &StepSpec) -> anyhow::Result<Hash> {
    Ok(step_key(spec)?)
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
