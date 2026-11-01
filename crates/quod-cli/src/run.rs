use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use clap::Args;
use quod_cas::{Cas, Hash, Kind, Tree};
use quod_key::{Image, StepSpec, Toolchain, step_key};
use quod_ledger::{Ledger, Lookup, Observed, RunFacts, now_rfc3339};
use quod_manifest::{Manifest, StepResult};
use quod_replay::{fmt_duration, group};
use quod_sandbox::{Killed, Limits, Mount, RunSpec};
use quod_trust::{Decision, Replayed, Step, Trust, Verdict};

use crate::deps::{self, Deps};
use crate::gitdir;
use crate::rootfs::{self, Rootfs};
use crate::seal;
use crate::snapshot;
use crate::state::State;
use crate::trust::{PolicyArgs, ignored_outputs, parse_timeout, shell_join};

/// Exit code of a step Quod killed for running past --timeout (as
/// timeout(1) does). Never a recorded result. A step can exit 124 by itself;
/// the TIMED OUT banner on stderr, the run row's outcome `timeout` and the
/// `run` event's `outcome` in events.jsonl tell the two apart.
pub const EXIT_TIMEOUT: u8 = 124;

#[derive(Args)]
pub struct RunArgs {
    /// The step as a command line, after the flags: `quod run -- go
    /// test ./...` (joined into one script, run as /bin/sh -e -c). The
    /// alternative to --step.
    #[arg(
        trailing_var_arg = true,
        allow_hyphen_values = true,
        value_name = "CMD"
    )]
    cmd: Vec<String>,
    /// The workspace: the repo checkout, plus outputs of earlier steps.
    /// Defaults to the git toplevel of the current directory.
    #[arg(long)]
    repo: Option<PathBuf>,
    /// Expected HEAD of --repo; refused if the checkout differs.
    #[arg(long)]
    commit: Option<String>,
    /// The step script, run as /bin/sh -e -c <script>.
    #[arg(long)]
    step: Option<String>,
    /// Allow network. Such a step runs directly on --repo (its writes
    /// persist), with its caches (GOMODCACHE, GOCACHE, HOME) in the state
    /// dir instead of the checkout, and is never memoized.
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
    /// imported from (`quod rootfs import`). The key's image is then
    /// the digest, and the proof holds on any host with the same rootfs.
    /// Default: $QUOD_ROOTFS.
    #[arg(long, value_name = "DIGEST", conflicts_with = "host_root")]
    rootfs: Option<String>,
    /// Pin the host's own userland (its fingerprint) instead of a rootfs:
    /// the fallback for hosts without one. Such a proof does not travel.
    #[arg(long)]
    host_root: bool,
    #[command(flatten)]
    policy: PolicyArgs,
    /// Kill the step after this long (30m, 2s, 1h; 0 for no limit). A
    /// timed-out step exits 124, is reported, and is never memoized.
    #[arg(long, default_value = "30m", value_parser = parse_timeout, env = "QUOD_TIMEOUT")]
    timeout: Duration,
}

/// Wall time per phase, reported in the run event.
#[derive(Default, Debug)]
pub(crate) struct Phases {
    /// Before the import: store, environment pin (host fingerprint or
    /// rootfs), step key, ledger and trust.
    setup: Duration,
    import: Duration,
    lookup: Duration,
    mount: Duration,
    /// Parts of `mount`: the observer coming up (FUSE mount, ring queues,
    /// INIT), the overlay, and the sealed snapshots.
    serve: Duration,
    overlay: Duration,
    seal: Duration,
    exec: Duration,
    diff: Duration,
    /// Parts of `diff`: storing the overlay's upper (the step's outputs),
    /// the observer's unmount (with the log), and building the manifest.
    outputs: Duration,
    unmount: Duration,
    manifest: Duration,
    record: Duration,
    /// Removing the run directory after the run is recorded.
    cleanup: Duration,
}

/// What every run row of this invocation shares.
struct RunInfo {
    run_id: String,
    head: Option<String>,
    host: Option<String>,
    key: Hash,
    /// The environment pin (rootfs or host fingerprint, with the platform).
    image: Image,
    repo_id: String,
    repo_path: String,
    mode: &'static str,
}

impl RunInfo {
    fn facts<'a>(
        &'a self,
        outcome: &'a str,
        manifest: Option<&'a Hash>,
        exit_code: Option<i32>,
        wall: Duration,
        saved: Option<Duration>,
    ) -> RunFacts<'a> {
        RunFacts {
            id: &self.run_id,
            commit_sha: self.head.as_deref(),
            host: self.host.as_deref(),
            // A replay observes nothing; an execution sets its transport.
            observed: Observed::default(),
            step_key: Some(&self.key),
            repo: Some(&self.repo_id),
            repo_path: Some(&self.repo_path),
            mode: Some(self.mode),
            outcome: Some(outcome),
            manifest_id: manifest,
            exit_code,
            wall_ms: Some(wall.as_millis() as u64),
            saved_ms: saved.map(|d| d.as_millis() as u64),
        }
    }
}

/// The repo and the step script, from `--repo`/`--step` or their defaults
/// (the git toplevel of the current directory; the trailing command). With
/// the repo defaulted from a subdirectory, the script starts with a `cd`
/// there, so `quod run -- go test ./...` tests what `go test ./...`
/// would from that directory (and is its own step key).
fn resolve_target(args: &RunArgs) -> anyhow::Result<(PathBuf, String)> {
    let (repo, step) = resolve_parts(args)?;
    if args.repo.is_some() {
        return Ok((repo, step));
    }
    let cwd = std::env::current_dir()
        .and_then(|d| d.canonicalize())
        .context("current directory")?;
    let top = repo
        .canonicalize()
        .with_context(|| format!("repo {}", repo.display()))?;
    let rel = cwd.strip_prefix(&top).with_context(|| {
        format!(
            "{} is not under its git toplevel {}; give --repo",
            cwd.display(),
            top.display()
        )
    })?;
    if rel.as_os_str().is_empty() {
        return Ok((top, step));
    }
    let rel = rel
        .to_str()
        .context("the current directory's path is not UTF-8; give --repo")?;
    Ok((
        top,
        format!("cd {}; {step}", shell_join(&[rel.to_string()])),
    ))
}

fn resolve_parts(args: &RunArgs) -> anyhow::Result<(PathBuf, String)> {
    let step = match (&args.step, args.cmd.is_empty()) {
        (Some(s), true) => s.clone(),
        (None, false) => shell_join(&args.cmd),
        (Some(_), false) => {
            bail!("give the step either as --step or as a trailing command, not both")
        }
        (None, true) => bail!(
            "no step: give --step <script>, or the command after the flags (quod run -- go test ./...)"
        ),
    };
    let repo = match &args.repo {
        Some(r) => r.clone(),
        None => {
            let cwd = std::env::current_dir().context("current directory")?;
            deps::toplevel(&cwd).context(
                "--repo not given and the current directory is not inside a git checkout",
            )?
        }
    };
    Ok((repo, step))
}

pub fn run(state: &State, args: RunArgs) -> anyhow::Result<u8> {
    if args.vm {
        bail!(
            "--vm needs the 0b Firecracker backend (T13-T16), which is not built: this host has no /dev/kvm"
        );
    }
    let (repo, step) = resolve_target(&args)?;
    let repo = repo
        .canonicalize()
        .with_context(|| format!("repo {}", repo.display()))?;
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
        StepSpec::new_warm(toolchain, image.clone(), &step, &extra_env)?
    } else {
        StepSpec::new(toolchain, image.clone(), &step, &extra_env)
    };
    let key = step_key(&spec)?;
    let env = spec.normalized_env()?;
    let policy = args.policy.policy()?;
    let info = RunInfo {
        run_id: ulid::Ulid::generate().to_string(),
        head,
        host: hostname(),
        key,
        image: image.clone(),
        repo_id: deps::repo_id(&repo),
        repo_path: repo.display().to_string(),
        mode: policy.mode.as_str(),
    };
    let deps = Deps::new(state, &info.repo_id, &image.id()?, toolchain);
    let ledger = Ledger::open(&args.ledger.clone().unwrap_or_else(|| state.ledger_path()))?;

    if args.allow_net {
        let timeout = (args.timeout > Duration::ZERO).then_some(args.timeout);
        return run_unobserved(
            state,
            &cas,
            &repo,
            &spec,
            env,
            &info,
            pin.rootfs.as_ref(),
            &deps,
            &ledger,
            timeout,
        );
    }

    let mut phases = Phases::default();
    ledger.put_step_key(&key, &quod_key::canonical_key_json(&spec)?)?;
    let trust = Trust::new(&ledger, &cas, policy)?;
    phases.setup = wall_start.elapsed();

    let t = Instant::now();
    let tree = deps::import_workspace(&cas, &repo, &deps, spec.profile())?;
    // The .git a plain checkout of this commit has, its index refreshed for
    // the step's identity (`execute` runs it as the sandbox's default).
    let (uid, gid) = quod_sandbox::step_ids(None, None);
    let tree = gitdir::graft(&cas, &state.dir, &repo, tree, uid, gid)?;
    phases.import = t.elapsed();
    let ignore = ignored_outputs(spec.profile(), &env, &tree);
    let tstep = Step {
        key,
        cmd: &step,
        ignore: &ignore,
    };

    let t = Instant::now();
    let lookup = ledger.find_match(&key, &tree)?;
    let decision = trust.decide(&key, lookup.hit.as_ref())?;
    phases.lookup = t.elapsed();

    // A replay is decided again as it is recorded (another run may have
    // quarantined the key or taken the audit slot meanwhile); if that says
    // execute, it executes.
    // The trust update and the run row are one transaction.
    let replayed = match decision {
        Decision::Replay => {
            let hit = lookup.hit.as_ref().expect("a replay needs a hit");
            let result = &hit.1;
            ledger.atomically(|l| -> anyhow::Result<_> {
                Ok(match trust.record_replay(&tstep, hit)? {
                    Replayed::Replayed(facts) => {
                        let replay_wall = wall_start.elapsed();
                        let saved =
                            Duration::from_millis(result.duration_ms).saturating_sub(replay_wall);
                        l.record_run_facts(&info.facts(
                            "replayed",
                            Some(&result.manifest_id),
                            Some(result.exit_code),
                            replay_wall,
                            Some(saved),
                        ))?;
                        Ok((facts, replay_wall))
                    }
                    Replayed::Execute(reason) => Err(reason),
                })
            })?
        }
        Decision::Execute(reason) => Err(reason),
    };
    let reason = match replayed {
        Ok((facts, replay_wall)) => {
            let (manifest, result) = lookup.hit.as_ref().expect("a replay needs a hit");
            let mid = result.manifest_id;
            if let Some(dir) = &args.outputs {
                // Only what a proof vouches for, and never an mtime-keyed
                // cache (a result recorded before a pattern existed may
                // still hold one).
                quod_replay::materialize_outputs(
                    &compared_only(&without_forbidden(spec.profile(), result), &ignore),
                    &cas,
                    dir,
                )?;
            }
            eprint!(
                "{}",
                quod_replay::receipt(
                    result,
                    manifest.entries.len(),
                    replay_wall,
                    &std::iter::once(facts.receipt_line())
                        .chain(uncompared_line(result, &ignore))
                        .collect::<Vec<_>>()
                )
            );
            let code = quod_replay::replay_log(
                result,
                &cas,
                &mut std::io::stdout().lock(),
                &mut std::io::stderr().lock(),
            )?;
            emit_event(
                &info,
                Some(&mid),
                true,
                "replayed",
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
        Err(reason) => reason,
    };

    let start = Instant::now();
    let dir = state.run_dir(&info.run_id);
    let warm_dir = match (warm, &spec.profile().warm) {
        (true, Some(w)) => Some(state.warm_cache_dir(&image.id()?, w.cache_env)),
        _ => None,
    };
    let timeout = (args.timeout > Duration::ZERO).then_some(args.timeout);
    let outcome = execute(
        &cas,
        &tree,
        &spec,
        env,
        &dir,
        warm_dir.as_deref(),
        pin.rootfs.as_ref(),
        &[],
        timeout,
        &mut phases,
    );
    let ex = match outcome {
        Ok(v) => v,
        Err(e) => {
            if !args.keep {
                let _ = quod_cas::remove_any(&dir);
            }
            return Err(e);
        }
    };
    if let Some(killed) = ex.killed {
        if !args.keep {
            quod_cas::remove_any(&dir)?;
        }
        if killed == Killed::Aborted {
            bail!(
                "the observer failed while the step ran; the step was aborted and nothing was recorded"
            );
        }
        // Timed out: killed, reported, never memoized, and not a divergence
        // (load or a slow observer can do it as well as the step). If a
        // result was expected, the matched manifest proves again from 0.
        let what = format!("timed out after {}", fmt_duration(args.timeout));
        eprintln!(
            "QUOD  TIMED OUT  the step ran {} and was killed; not memoized",
            fmt_duration(ex.wall)
        );
        // The streak reset and the run row are one transaction.
        let line = ledger.atomically(|l| -> anyhow::Result<_> {
            let line = trust.record_incomplete(&tstep, lookup.hit.as_ref(), &what)?;
            l.record_run_facts(&RunFacts {
                observed: Observed {
                    transport: Some(ex.transport),
                    observer: Some(&ex.observer),
                },
                ..info.facts(
                    "timeout",
                    None,
                    Some(i32::from(EXIT_TIMEOUT)),
                    ex.wall,
                    None,
                )
            })?;
            Ok(line)
        })?;
        if let Some(line) = line {
            eprintln!("{}", indent(&line));
        }
        emit_event(
            &info,
            None,
            false,
            "timeout",
            &lookup,
            &phases,
            start.elapsed(),
            i32::from(EXIT_TIMEOUT),
            0,
            warm,
            Some((ex.transport, &ex.observer)),
        );
        return Ok(EXIT_TIMEOUT);
    }
    let killed_by = match (ex.signal, &ex.resource_failure) {
        (Some(sig), _) => Some(format!("killed by signal {sig}")),
        (None, Some(why)) => Some(why.clone()),
        _ => None,
    };
    if let Some(why) = killed_by {
        // Killed from outside (a signal), or the machine ran out of
        // something while it ran (an OOM kill of a child, a full disk, the
        // fork limit): the step's own result is unknown, so it is reported
        // and never memoized, and nothing is learned about the key.
        if !args.keep {
            quod_cas::remove_any(&dir)?;
        }
        eprintln!("QUOD  EXECUTED  {why}, not memoized");
        eprintln!("  ran {}", fmt_duration(ex.wall));
        ledger.record_run_facts(&RunFacts {
            observed: Observed {
                transport: Some(ex.transport),
                observer: Some(&ex.observer),
            },
            ..info.facts("killed", None, Some(ex.exit_code), ex.wall, None)
        })?;
        return Ok(exit_byte(ex.exit_code));
    }
    if ex.sealed_write_failed {
        // The step reported a write the read-only bind of a sealed
        // dependency tree refused; in CI that write would have succeeded,
        // so this result may not be the step's. Shown, never memoized.
        if !args.keep {
            quod_cas::remove_any(&dir)?;
        }
        eprintln!(
            "QUOD  EXECUTED  a write hit a read-only sealed dependency tree (the step reported EROFS), not memoized"
        );
        eprintln!(
            "  a directory the tool writes under a sealed prefix belongs in the profile's carve-outs"
        );
        eprintln!("  ran {}", fmt_duration(ex.wall));
        tracing::warn!(target: "quod", run_id = %info.run_id, "sealed prefix write failed; not memoized");
        // Nothing is learned about the key: the result is not the step's.
        ledger.record_run_facts(&RunFacts {
            observed: Observed {
                transport: Some(ex.transport),
                observer: Some(&ex.observer),
            },
            ..info.facts("tainted", None, Some(ex.exit_code), ex.wall, None)
        })?;
        return Ok(exit_byte(ex.exit_code));
    }

    let t = Instant::now();
    let mid = quod_manifest::manifest_id(&ex.manifest)?;
    let result = StepResult {
        v: quod_manifest::VERSION,
        step_key: key,
        manifest_id: mid,
        run_id: info.run_id.clone(),
        recorded_at: now_rfc3339(),
        exit_code: ex.exit_code,
        stdout: ex.stdout,
        stderr: ex.stderr,
        outputs: ex.outputs,
        root_outputs: ex.root_outputs,
        duration_ms: ex.wall.as_millis() as u64,
        commands: vec![],
    };
    // One transaction: the trust layer's verdict (proof streak, trust
    // state, audit row), the run row (with the transport that observed it,
    // which results_by_transport and purge_transport find results through),
    // the manifest and the result commit together or not at all.
    let verdict = ledger.atomically(|l| -> anyhow::Result<Verdict> {
        l.put_manifest(&ex.manifest, &info.run_id)?;
        let v = trust.record_execution(
            &tstep,
            &info.run_id,
            &ex.manifest,
            &result,
            lookup.hit.as_ref(),
            &reason,
        )?;
        l.record_execution_facts(
            &RunFacts {
                observed: Observed {
                    transport: Some(ex.transport),
                    observer: Some(&ex.observer),
                },
                ..info.facts(
                    v.outcome(),
                    Some(&mid),
                    Some(ex.exit_code),
                    ex.wall,
                    v.would_have_saved,
                )
            },
            &ex.manifest,
            &result,
        )?;
        Ok(v)
    })?;
    phases.record = t.elapsed();
    if let Some(out) = &args.outputs {
        quod_replay::materialize_outputs(
            &compared_only(&without_forbidden(spec.profile(), &result), &ignore),
            &cas,
            out,
        )?;
    }
    let t = Instant::now();
    if !args.keep {
        quod_cas::remove_any(&dir)?;
    }
    phases.cleanup = t.elapsed();

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
            warm_dir.as_deref(),
            &verdict,
            uncompared_line(&result, &ignore).as_deref(),
        )
    );
    emit_event(
        &info,
        Some(&mid),
        false,
        verdict.outcome(),
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
    /// Set when a limit killed the step (timeout, or the observer failing).
    pub(crate) killed: Option<Killed>,
    /// Why the outcome may be the machine's (OOM kills, a full disk, the
    /// fork limit; `quod_sandbox::RunResult::resource_failure`).
    pub(crate) resource_failure: Option<String>,
    pub(crate) manifest: Manifest,
    pub(crate) outputs: Vec<quod_cas::Entry>,
    /// Writes outside the workspace (rootfs mode only), relative to `/`.
    pub(crate) root_outputs: Vec<quod_cas::Entry>,
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
/// (`crate::step`). The sandbox kills the step after `timeout`, and as soon
/// as the observer raises its abort flag (`unmount` then reports the
/// failure as a Quod error).
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
    timeout: Option<Duration>,
    phases: &mut Phases,
) -> anyhow::Result<Executed> {
    let t = Instant::now();
    let lower = dir.join("lower");
    std::fs::create_dir_all(&lower)?;
    let served = quod_fs::serve(tree, cas, &lower)?;
    phases.serve = t.elapsed();
    let ov = quod_fs::overlay(
        &served,
        &dir.join("upper"),
        &dir.join("work"),
        &dir.join("merged"),
    )?;
    phases.overlay = t.elapsed() - phases.serve;
    let root = match rootfs {
        Some(r) => Some(rootfs::RootOverlay::mount(r, cas, dir)?),
        None => None,
    };
    let mut mounts = vec![Mount {
        src: ov.merged().to_path_buf(),
        dst: quod_key::WORKSPACE.into(),
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
    // Which prefixes are sealable is a function of the profile and the
    // tree; whether a snapshot could be made is not (and sealing can be
    // turned off for the bench). The manifest records every sealable prefix
    // whole either way, so its shape never depends on the observer's
    // configuration: a sealed and an observed run of one tree read
    // equivalent manifests, and neither can be mistaken for a divergence.
    let sealable = spec.profile().seals(tree);
    let ts = Instant::now();
    let sealed = seal::seal(spec.profile(), tree, cas, &snapshots);
    phases.seal = ts.elapsed();
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
        rootfs: root.as_ref().map(|o| quod_sandbox::RootFs {
            dir: o.merged().to_path_buf(),
            writable: true,
        }),
        identity: None,
        uid: None,
        gid: None,
        net_probe: false,
    };
    let limits = Limits {
        timeout,
        abort: Some(served.abort_flag()),
    };
    let r = quod_sandbox::run_with(&rs, &limits)?;
    phases.exec = t.elapsed();
    if served
        .abort_flag()
        .load(std::sync::atomic::Ordering::SeqCst)
    {
        // The step ran against a failed observer (a panicking classic
        // thread, a dying ring queue, any I/O error served) and was
        // aborted: its outputs and its log mean nothing. Unmount the
        // overlay, then the observer, which reports the failure
        // (FsError::Fatal or FsError::Io).
        drop(ov);
        served.unmount()?;
        bail!("the observer failed while the step ran; nothing was recorded");
    }

    let t = Instant::now();
    let mut outputs = ov.finish(tree, cas)?;
    phases.outputs = t.elapsed();
    // mtime-keyed caches never become recorded outputs (see
    // Profile::no_outputs_under); their blobs stay unreferenced in the store.
    let before = outputs.len();
    outputs.retain(|o| !spec.profile().forbids_output(&o.p));
    if outputs.len() < before {
        tracing::info!(target: "quod", dropped = before - outputs.len(), "mtime-keyed cache outputs not recorded");
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
    let tu = Instant::now();
    let mut log = served.unmount()?;
    phases.unmount = tu.elapsed();
    let tm = Instant::now();
    for s in &sealable {
        log.extend(seal::accesses(tree, s));
        // Each carve-out too, as the tree has it (a listing, or absent): a
        // sealed run's mounting of it is a lookup the observer records, so
        // an unsealed run records it as well.
        log.extend(s.carve_outs.iter().map(|c| quod_manifest::Access {
            path: c.clone(),
            kind: tree.get(c).map_or(Kind::Absent, |e| e.k),
        }));
    }
    if let Some(s) = sealed.first() {
        // Housekeeping never fails a completed run (gc warns instead).
        let removed = snapshot::gc(&snapshots, snapshot::KEEP, &s.snapshot.dir);
        if removed > 0 {
            tracing::info!(target: "quod", removed, "snapshot gc");
        }
    }
    if let (Some(_), Some(w)) = (warm, &spec.profile().warm) {
        // What a cached verdict may stand in for (for Go: vet's reads of a
        // package's ignored files, see quod_key::go_source_file).
        quod_manifest::record_children(tree, &mut log, w.over_record);
    }
    let manifest = Manifest::from_log(spec_key(spec)?, spec.roots.clone(), tree, log)?;
    phases.manifest = tm.elapsed();
    // Blobs before the result that names them.
    let (stdout, _) = cas.put_file(&dir.join("stdout"))?;
    let (stderr, _) = cas.put_file(&dir.join("stderr"))?;
    phases.diff = t.elapsed();
    let observer = format!(
        "{}{}{}{}{}",
        stats.transport_detail(),
        if stats.passthrough {
            "+passthrough"
        } else {
            ""
        },
        if stats.zero_open { "+zero-open" } else { "" },
        if stats.zero_opendir {
            "+zero-opendir"
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
        killed: r.killed,
        resource_failure: r.resource_failure,
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
fn without_forbidden(profile: &quod_key::Profile, result: &StepResult) -> StepResult {
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
    // The strerror phrase in any case; the errno name only as a whole,
    // upper-case word, so identifiers like `numberOfSides` don't match.
    const PHRASE: &[u8] = b"read-only file system";
    const ERRNO: &[u8] = b"EROFS";
    let keep = PHRASE.len() + 1;
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
        window.extend_from_slice(&buf[..n]);
        let word = |b: Option<&u8>| b.is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_');
        let errno_at = |i: usize| {
            window[i..].starts_with(ERRNO)
                && !word(i.checked_sub(1).and_then(|j| window.get(j)))
                && !word(window.get(i + ERRNO.len()))
        };
        if window
            .windows(PHRASE.len())
            .any(|w| w.eq_ignore_ascii_case(PHRASE))
            || (0..window.len()).any(errno_at)
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
/// Its `.quod` directory (dependency trees and scratch) is bound from
/// the state dir (`crate::deps`), so the checkout receives only the step's
/// own writes.
#[allow(clippy::too_many_arguments)] // one call site; the inputs of a run, each named
fn run_unobserved(
    state: &State,
    cas: &Cas,
    repo: &Path,
    spec: &StepSpec,
    mut env: Vec<(String, String)>,
    info: &RunInfo,
    rootfs: Option<&Rootfs>,
    deps: &Deps,
    ledger: &Ledger,
    timeout: Option<Duration>,
) -> anyhow::Result<u8> {
    let profile = spec.profile();
    // A network step fetches; let the tool reach its registry.
    for (name, value) in profile.net_env {
        match env.iter_mut().find(|(k, _)| k == name) {
            Some((_, v)) => *v = value.to_string(),
            None => env.push((name.to_string(), value.to_string())),
        }
    }
    let dir = state.run_dir(&info.run_id);
    std::fs::create_dir_all(&dir)?;
    let root = match rootfs {
        Some(r) => Some(rootfs::ReadOnlyRoot::mount(r, cas, &dir)?),
        None => None,
    };
    let mut mounts = vec![
        Mount {
            src: repo.to_path_buf(),
            dst: quod_key::WORKSPACE.into(),
            writable: true,
        },
        deps::network_mount(deps, repo, profile)?,
    ];
    for rel in profile.setup_cache {
        let name = rel.rsplit('/').next().unwrap_or(rel);
        let host = state.setup_cache_dir(spec.toolchain).join(name);
        std::fs::create_dir_all(&host).with_context(|| format!("create {}", host.display()))?;
        // The mount point must exist on the host (bubblewrap cannot create
        // it through a bind): in the state dir's `.quod` for a path
        // under it, in the checkout otherwise.
        std::fs::create_dir_all(deps::mount_point(deps, repo, rel))?;
        mounts.push(Mount {
            src: host,
            dst: Path::new(quod_key::WORKSPACE).join(rel),
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
        rootfs: root.as_ref().map(|r| quod_sandbox::RootFs {
            dir: r.dir().to_path_buf(),
            writable: false,
        }),
        identity: None,
        uid: None,
        gid: None,
        net_probe: false,
    };
    let r = quod_sandbox::run_with(
        &rs,
        &Limits {
            timeout,
            abort: None,
        },
    );
    if let Some(root) = root {
        root.unmount()?;
    }
    deps::after_network_step(repo);
    let r = r?;
    quod_cas::remove_any(&dir)?;
    if r.killed == Some(Killed::Timeout) {
        eprintln!(
            "QUOD  TIMED OUT  the network step ran {} and was killed",
            fmt_duration(r.wall)
        );
        eprintln!(
            "  caches in {} (whatever it fetched stays)",
            deps.root.display()
        );
        ledger.record_run_facts(&info.facts(
            "timeout",
            None,
            Some(i32::from(EXIT_TIMEOUT)),
            r.wall,
            None,
        ))?;
        tracing::info!(target: "quod", run_id = %info.run_id, memoized = false, egress = true, outcome = "timeout", exit_code = i32::from(EXIT_TIMEOUT), wall_ms = r.wall.as_millis() as u64, "run");
        return Ok(EXIT_TIMEOUT);
    }
    eprintln!("QUOD  EXECUTED  network allowed, not memoized");
    eprintln!("  {}", image_line(&spec.image));
    eprintln!("  caches in {}", deps.root.display());
    eprintln!("  ran {}", fmt_duration(r.wall));
    ledger.record_run_facts(&info.facts("net", None, Some(r.exit_code), r.wall, None))?;
    tracing::info!(target: "quod", run_id = %info.run_id, memoized = false, egress = true, exit_code = r.exit_code, wall_ms = r.wall.as_millis() as u64, "run");
    Ok(exit_byte(r.exit_code))
}

/// The banner's environment line: what pinned the userland, and the
/// platform.
fn image_line(image: &Image) -> String {
    let p = &image.platform;
    let platform = format!("{} linux {} {} cpus", p.arch, p.kernel, p.cpus);
    match image.kind {
        quod_key::ImageKind::Rootfs => {
            format!("image rootfs b3:{}  {platform}", image.digest.short())
        }
        quod_key::ImageKind::Host => format!(
            "image host fingerprint b3:{}  {platform}  (not portable)",
            image.digest.short()
        ),
    }
}

/// The result with only the outputs the trust layer compares. `--outputs`
/// writes these, on execute and on replay alike: an uncompared path (a
/// cache, a telemetry counter) is not vouched for by any proof, so a replay
/// must not hand it out as the step's, and an execution writes the same set
/// so the two leave identical directories.
fn compared_only(r: &StepResult, ignore: &[String]) -> StepResult {
    StepResult {
        outputs: quod_trust::comparable_outputs(&r.outputs, ignore)
            .into_iter()
            .cloned()
            .collect(),
        ..r.clone()
    }
}

/// The scratch prefixes (`ignored_outputs`) this result wrote under, as a
/// banner line: what the trust layer did not compare and `--outputs` does
/// not write. `None` when it wrote under none of them.
fn uncompared_line(r: &StepResult, ignore: &[String]) -> Option<String> {
    let under: Vec<&str> = ignore
        .iter()
        .filter(|pre| {
            r.outputs
                .iter()
                .any(|e| e.p == **pre || e.p.starts_with(&format!("{pre}/")))
        })
        .map(String::as_str)
        .collect();
    (!under.is_empty()).then(|| {
        format!(
            "not compared, and not written by --outputs: {}",
            under.join(", ")
        )
    })
}

/// Trust lines are indented under the banner; a DIVERGENCE block comes
/// with its own header and indentation.
fn indent(line: &str) -> String {
    if line.starts_with("QUOD") || line.starts_with("  ") {
        line.to_string()
    } else {
        format!("  {line}")
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
    verdict: &Verdict,
    uncompared: Option<&str>,
) -> String {
    let mut s = format!("QUOD  EXECUTED{}\n", verdict.headline());
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
    if lookup.hit.is_none() {
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
    }
    for l in verdict.lines() {
        s += &indent(&l);
        s.push('\n');
    }
    if let Some(u) = uncompared {
        s += &indent(u);
        s.push('\n');
    }
    s += &format!("  ran {}\n", fmt_duration(wall));
    s
}

#[allow(clippy::too_many_arguments)] // one flat event, every field named
fn emit_event(
    info: &RunInfo,
    manifest: Option<&Hash>,
    hit: bool,
    outcome: &str,
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
        target: "quod",
        run_id = %info.run_id,
        step_key = %info.key,
        image_kind = %info.image.kind,
        image = %info.image.digest,
        arch = %info.image.platform.arch,
        kernel = %info.image.platform.kernel,
        cpus = info.image.platform.cpus,
        manifest_id = manifest.map(|m| m.to_string()),
        hit,
        mode = info.mode,
        outcome,
        warm,
        exit_code,
        inputs,
        transport,
        observer,
        candidates = lookup.candidates,
        first_mismatch = lookup.closest.as_ref().map(|(_, m)| m.path.clone()),
        recorded_ms = lookup.hit.as_ref().map(|(_, r)| r.duration_ms),
        setup_ms = ms(p.setup),
        import_ms = ms(p.import),
        lookup_ms = ms(p.lookup),
        mount_ms = ms(p.mount),
        serve_ms = ms(p.serve),
        overlay_ms = ms(p.overlay),
        seal_ms = ms(p.seal),
        exec_ms = ms(p.exec),
        diff_ms = ms(p.diff),
        outputs_ms = ms(p.outputs),
        unmount_ms = ms(p.unmount),
        manifest_ms = ms(p.manifest),
        record_ms = ms(p.record),
        cleanup_ms = ms(p.cleanup),
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
        Err(quod_key::profile::Detect::None) => Ok(Toolchain::Go),
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
        // The errno name counts only as a whole upper-case word.
        assert!(check(b"open x: EROFS\n"));
        assert!(!check(b"--- PASS: TestNumberOfSides (0.00s)\n"));
        assert!(!check(b"powerOfSum erofs_helper SEROFS EROFS_X\n"));
    }
}
