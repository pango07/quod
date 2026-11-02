//! Deferred audits (docs/trust-v2.md, "Deferred audits"): the executor of
//! the audits a replay left pending, and what a replay stores for it.
//!
//! When replay mode serves a would-be hit that the audit schedule selects
//! in the decay phase, the step's replay is served at once and a row in
//! the ledger's `audits_pending` says what is owed: the key, the manifest
//! and result that were served, the exact input tree as a document in the
//! store (`rootfs::put_tree`: content-addressed, so a tree seen before
//! costs nothing to store again), and a `Request` with everything else
//! `run::execute` needs. `quod audit` is the worker: it takes the state's
//! audit lock (one worker per state dir), lowers its own CPU and I/O
//! priority, expires what nobody ran in time, and drains the queue oldest
//! first within a time budget. Each audit rebuilds the tree from the store,
//! checks it still matches the manifest, executes the step with go test's
//! cache expired (as every inline audit does), and records the execution
//! through `Trust::record_execution` under `Reason::DeferredAudit`, which
//! compares it with the served result, settles the pending row, and on a
//! divergence quarantines the key and marks the replays suspect.
//!
//! `quod run` and `quod step` start a worker, detached, after serving a
//! deferred replay; `QUOD_AUDIT_SPAWN=0` leaves that to the caller (tests,
//! a cron job, a runner's post-job hook).

use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::Context;
use clap::Args;
use quod_cas::{Cas, Hash};
use quod_key::{Image, StepSpec, Toolchain, step_key};
use quod_ledger::{
    Ledger, NewPending, Observed, PendingAudit, PendingState, RunFacts, format_rfc3339, now_rfc3339,
};
use quod_manifest::{Manifest, StepResult};
use quod_sandbox::Killed;
use quod_trust::{
    LEASE, MAX_ATTEMPTS, PENDING_TTL, Policy, Reason, Step, Trust, Verdict, stale_cutoff,
};
use serde::{Deserialize, Serialize};

use crate::gotest::{self, GoTestGuard, GoTests};
use crate::rootfs;
use crate::run::{self, EXIT_TIMEOUT, Phases};
use crate::state::State;
use crate::trust::parse_timeout;

/// The request format version.
pub const REQUEST_VERSION: u32 = 1;

/// What the worker needs to execute a step again, beyond the tree and the
/// manifest the pending row names. Written by the replay that owes the
/// audit, read by the worker; both are this binary.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Request {
    pub v: u32,
    /// `run` (`quod run`: host root or a rootfs, warm state, the go test
    /// guard) or `step` (the runner API: a rootfs with pinned mounts).
    pub kind: Kind,
    pub spec: SpecDoc,
    /// The script as `quod run` was given it (for the go test guard's probe).
    #[serde(default)]
    pub script: Option<String>,
    /// The step's own env on top of the profile's (`--env`).
    #[serde(default)]
    pub extra_env: Vec<(String, String)>,
    pub policy: PolicyDoc,
    #[serde(default)]
    pub repo: Option<RepoDoc>,
    /// The rootfs the step ran in, if any (else the host root).
    #[serde(default)]
    pub rootfs: Option<Hash>,
    /// Warm state: the step's build cache is the state dir's warm cache.
    pub warm: bool,
    /// The step ran with go test's cache under Quod's guard.
    pub guard: bool,
    /// Output prefixes the comparison skips (`trust::ignored_outputs`).
    pub ignore: Vec<String>,
    /// The step's timeout, 0 for none.
    pub timeout_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Run,
    Step,
}

/// `StepSpec`, as JSON.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SpecDoc {
    pub image: Image,
    pub toolchain: Toolchain,
    pub cmd: Vec<String>,
    pub cwd: String,
    pub env: Vec<(String, String)>,
    pub roots: Vec<String>,
    pub warm: bool,
}

impl From<&StepSpec> for SpecDoc {
    fn from(s: &StepSpec) -> SpecDoc {
        SpecDoc {
            image: s.image.clone(),
            toolchain: s.toolchain,
            cmd: s.cmd.clone(),
            cwd: s.cwd.clone(),
            env: s.env.clone(),
            roots: s.roots.clone(),
            warm: s.warm,
        }
    }
}

impl From<SpecDoc> for StepSpec {
    fn from(d: SpecDoc) -> StepSpec {
        StepSpec {
            image: d.image,
            toolchain: d.toolchain,
            cmd: d.cmd,
            cwd: d.cwd,
            env: d.env,
            roots: d.roots,
            warm: d.warm,
        }
    }
}

/// The trust policy the replay was decided under: the audit is judged
/// under the same one (replay mode).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PolicyDoc {
    pub proofs: u32,
    pub shadow: u32,
    pub audit: f64,
    pub replay_failures: bool,
}

impl From<&Policy> for PolicyDoc {
    fn from(p: &Policy) -> PolicyDoc {
        PolicyDoc {
            proofs: p.proofs,
            shadow: p.shadow,
            audit: p.audit,
            replay_failures: p.replay_failures,
        }
    }
}

impl PolicyDoc {
    fn policy(&self) -> Policy {
        Policy {
            mode: quod_trust::Mode::Replay,
            proofs: self.proofs,
            shadow: self.shadow,
            audit: self.audit,
            replay_failures: self.replay_failures,
            defer: true,
        }
    }
}

/// The run row's repository facts, as the replay's run row had them.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RepoDoc {
    pub id: String,
    pub path: String,
    pub head: Option<String>,
    pub host: Option<String>,
}

impl Request {
    pub fn to_json(&self) -> anyhow::Result<Vec<u8>> {
        Ok(serde_json::to_vec(self)?)
    }
}

/// Records the audit a replay just served owes, in the caller's
/// transaction. `tree` is the exact input tree, already in the store.
pub fn enqueue(
    ledger: &Ledger,
    key: &Hash,
    hit: &(Manifest, StepResult),
    replay_run: &str,
    nth: u64,
    tree: &Hash,
    request: &Request,
) -> anyhow::Result<i64> {
    let (_, r) = hit;
    let id = ledger.enqueue_audit(&NewPending {
        step_key: key,
        manifest_id: &r.manifest_id,
        hit_run: &r.run_id,
        replay_run,
        nth,
        tree,
        request: &request.to_json()?,
    })?;
    tracing::info!(target: "quod", pending = id, step_key = %key, manifest_id = %r.manifest_id, nth, replay_run, "audit deferred");
    Ok(id)
}

/// The receipt line of a replay that owes a deferred audit.
pub fn receipt_line(nth: u64, id: i64) -> String {
    format!(
        "trust: audit deferred: would-be hit {nth} is re-executed and compared in the background (pending audit {id}); a divergence marks this replay suspect"
    )
}

/// Starts `quod audit` for this state dir, detached (its own session, so
/// a caller that kills its process group at the end of a step leaves it
/// alone), with its output appended to `<state>/audit.log`. Nothing waits
/// for it; a worker that finds another holding the lock exits at once.
/// `QUOD_AUDIT_SPAWN=0` turns this off (the caller runs `quod audit`).
/// `ledger` and `cas` are the run's overrides of the state's own, passed
/// on so the worker opens the ledger the audit was enqueued in.
pub fn spawn_worker(state: &State, ledger: Option<&Path>, cas: Option<&Path>) {
    if std::env::var("QUOD_AUDIT_SPAWN").is_ok_and(|v| v == "0") {
        return;
    }
    if let Err(e) = try_spawn(state, ledger, cas) {
        tracing::warn!(target: "quod", error = %format!("{e:#}"), "audit worker not started");
    }
}

fn try_spawn(state: &State, ledger: Option<&Path>, cas: Option<&Path>) -> anyhow::Result<()> {
    use std::os::unix::process::CommandExt as _;
    let exe = std::env::current_exe().context("locate this binary")?;
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(state.dir.join("audit.log"))
        .context("open audit.log")?;
    let mut cmd = Command::new(exe);
    cmd.arg("--state").arg(&state.dir).arg("audit");
    if let Some(l) = ledger {
        cmd.arg("--ledger").arg(l);
    }
    if let Some(c) = cas {
        cmd.arg("--cas").arg(c);
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log));
    // SAFETY: setsid(2) is async-signal-safe and touches no memory of the
    // forked child but its own session id.
    #[allow(unsafe_code)] // pre_exec is unsafe by signature
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let child = cmd.spawn().context("start quod audit")?;
    tracing::info!(target: "quod", pid = child.id(), "audit worker started");
    Ok(())
}

/// This process to the idle CPU class (SCHED_IDLE: it runs only when no
/// other runnable task wants the CPU; nice 19 as the fallback where the
/// policy is refused) and the idle I/O class: a deferred audit must never
/// slow the job running beside it. The step it runs inherits both. Best
/// effort: a kernel or container that refuses leaves the default.
fn lower_priority() {
    // SAFETY: the calls take plain integers and a zeroed sched_param
    // (SCHED_IDLE takes priority 0), and change only this process's
    // scheduling attributes.
    #[allow(unsafe_code)]
    // sched_setscheduler(2), setpriority(2) and ioprio_set(2) have no std wrappers
    unsafe {
        let param: libc::sched_param = std::mem::zeroed();
        if libc::sched_setscheduler(0, libc::SCHED_IDLE, &param) != 0 {
            libc::setpriority(libc::PRIO_PROCESS, 0, 19);
        }
        // IOPRIO_WHO_PROCESS = 1, IOPRIO_CLASS_IDLE = 3 in bits 13..16.
        libc::syscall(libc::SYS_ioprio_set, 1, 0, 3 << 13);
    }
}

#[derive(Args)]
pub struct AuditArgs {
    /// Print the pending, running and lost audits instead of executing.
    #[arg(long)]
    list: bool,
    /// Stop claiming audits after this long (the one running finishes).
    #[arg(long, default_value = "1h", value_parser = parse_timeout)]
    budget: Duration,
    /// Execute at most one audit.
    #[arg(long)]
    one: bool,
    #[arg(long)]
    ledger: Option<PathBuf>,
    #[arg(long)]
    cas: Option<PathBuf>,
}

/// `quod audit`: executes the deferred audits of this state dir.
pub fn audit(state: &State, args: AuditArgs) -> anyhow::Result<u8> {
    let ledger = Ledger::open(&args.ledger.clone().unwrap_or_else(|| state.ledger_path()))?;
    let cas = Cas::open(args.cas.clone().unwrap_or_else(|| state.cas_dir()))?;
    if args.list {
        return list(&ledger);
    }
    let lock_path = state.dir.join("audit.lock");
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .with_context(|| format!("open {}", lock_path.display()))?;
    if lock.try_lock().is_err() {
        say!(
            "QUOD  AUDIT  another worker holds {}; nothing to do",
            lock_path.display()
        );
        return Ok(0);
    }
    lower_priority();
    let worker = format!(
        "{}:{}",
        run::hostname().unwrap_or_else(|| "?".into()),
        std::process::id()
    );
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?;
    let cutoff = |d: Duration| format_rfc3339(now.saturating_sub(d).as_secs(), 0);
    let lost = ledger.expire_audits(&cutoff(PENDING_TTL), &cutoff(LEASE), MAX_ATTEMPTS)?;
    for id in &lost {
        let a = ledger.pending_audit(*id)?;
        say!(
            "QUOD  AUDIT  pending audit {id} lost: {}; the next would-be hit of its key is audited inline",
            a.and_then(|a| a.detail).unwrap_or_default()
        );
        tracing::warn!(target: "quod", pending = id, "deferred audit lost");
    }
    let start = Instant::now();
    let (mut done, mut diverged) = (0u32, 0u32);
    while start.elapsed() < args.budget {
        let Some(p) = ledger.claim_audit(&worker)? else {
            break;
        };
        match run_one(state, &ledger, &cas, &p) {
            Ok(Some(v)) => {
                done += 1;
                diverged += u32::from(v.divergence.is_some());
            }
            Ok(None) => {}
            Err(e) => {
                let state_now = ledger.release_audit(
                    p.id,
                    MAX_ATTEMPTS,
                    &format!("attempt {} failed: {e:#}", p.attempts),
                )?;
                say!(
                    "QUOD  AUDIT  pending audit {} attempt {} failed: {e:#}; now {}",
                    p.id,
                    p.attempts,
                    state_now.as_str()
                );
                tracing::warn!(target: "quod", pending = p.id, attempt = p.attempts, error = %format!("{e:#}"), state = state_now.as_str(), "deferred audit attempt failed");
            }
        }
        if args.one {
            break;
        }
    }
    let o = ledger.outstanding(&stale_cutoff())?;
    say!(
        "QUOD  AUDIT  {done} executed ({diverged} diverged), {} lost; {} pending, {} running, {} lost and not repaid; {}",
        lost.len(),
        o.pending + o.overdue,
        o.running,
        o.lost,
        quod_replay::fmt_duration(start.elapsed())
    );
    Ok(0)
}

fn list(ledger: &Ledger) -> anyhow::Result<u8> {
    use std::io::Write as _;
    let mut o = std::io::stdout().lock();
    for state in [
        PendingState::Pending,
        PendingState::Running,
        PendingState::Lost,
    ] {
        for a in ledger.pending_audits(None, state)? {
            writeln!(
                o,
                "{:<8} {:>5}  hit {:<4} key b3:{}  replay {}  enqueued {}  attempts {}{}{}",
                state.as_str(),
                a.id,
                a.nth,
                a.step_key.short(),
                &a.replay_run[..a.replay_run.len().min(8)],
                quod_replay::human_time(&a.enqueued_at),
                a.attempts,
                a.claimed_by
                    .as_deref()
                    .map(|w| format!("  claimed by {w}"))
                    .unwrap_or_default(),
                a.detail
                    .as_deref()
                    .map(|d| format!("  {d}"))
                    .unwrap_or_default(),
            )?;
        }
    }
    Ok(0)
}

/// A pending audit lost for a reason no retry would change.
fn lose(ledger: &Ledger, p: &PendingAudit, why: &str) -> anyhow::Result<Option<Verdict>> {
    ledger.lose_audit(p.id, why)?;
    say!(
        "QUOD  AUDIT  pending audit {} lost: {why}; the next would-be hit of its key is audited inline",
        p.id
    );
    tracing::warn!(target: "quod", pending = p.id, why, "deferred audit lost");
    Ok(None)
}

/// Executes one claimed audit. `Ok(Some)` with the verdict when it was
/// executed and compared (the pending row is settled); `Ok(None)` when it
/// ended otherwise (the row is lost or released, and says why); `Err` for
/// a Quod error (the caller releases the row for another attempt).
fn run_one(
    state: &State,
    ledger: &Ledger,
    cas: &Cas,
    p: &PendingAudit,
) -> anyhow::Result<Option<Verdict>> {
    let t0 = Instant::now();
    let req: Request = match serde_json::from_slice(&p.request) {
        Ok(r) => r,
        Err(e) => return lose(ledger, p, &format!("the audit request does not parse: {e}")),
    };
    if req.v != REQUEST_VERSION {
        return lose(
            ledger,
            p,
            &format!("request version {} is not {REQUEST_VERSION}", req.v),
        );
    }
    let key = p.step_key;
    // The key was quarantined since this audit was enqueued (the replay it
    // checks is suspect already): it would compare the step's behaviour
    // now against a result that quarantine declared wrong, and count the
    // one divergence again. Dropped, with any other outstanding audit of
    // the key, as repaid: the restarted shadow checks the key instead.
    if ledger
        .run(&p.replay_run)?
        .is_some_and(|r| r.suspect.is_some())
    {
        let n = ledger.supersede_pending(
            &key,
            None,
            "superseded: the replay it checks was marked suspect by a quarantine of its key",
        )?;
        say!(
            "QUOD  AUDIT  pending audit {}: its replay is suspect already (the key was quarantined); dropped with {} other outstanding audit(s) of the key",
            p.id,
            n.saturating_sub(1)
        );
        return Ok(None);
    }
    let Some(manifest) = ledger.manifest(&p.manifest_id)? else {
        return lose(ledger, p, &format!("manifest {} is gone", p.manifest_id));
    };
    let Some(hit_result) = ledger
        .results_for_run(&p.hit_run)?
        .into_iter()
        .find(|r| r.step_key == key && r.manifest_id == p.manifest_id)
    else {
        return lose(
            ledger,
            p,
            &format!("the served result (run {}) is gone", p.hit_run),
        );
    };
    let hit = (manifest, hit_result);
    // The exact tree the replay was decided on, from the store, and the
    // manifest must still match it (it did when the tree was stored; a
    // store that lost a blob fails here, loudly, never silently passes).
    let tree = rootfs::tree(cas, &p.tree)?;
    if let Err(mm) = quod_manifest::matches(&hit.0, &tree) {
        return lose(
            ledger,
            p,
            &format!("the stored tree does not match the manifest at {}", mm.path),
        );
    }
    let spec: StepSpec = req.spec.clone().into();
    if step_key(&spec)? != key {
        return lose(ledger, p, "the stored step spec does not key to the step");
    }
    let env = spec.normalized_env()?;
    let policy = req.policy.policy();
    let trust = Trust::new(ledger, cas, policy)?;
    let tstep = Step {
        key,
        cmd: &crate::trust::shell_join(&spec.cmd),
        ignore: &req.ignore,
    };
    let reason = Reason::DeferredAudit {
        nth: p.nth,
        pending: p.id,
    };
    let run_id = ulid::Ulid::generate().to_string();
    let run_dir = state.claim_run_dir(&run_id, false)?;
    let dir = run_dir.path.clone();
    let root = match &req.rootfs {
        Some(d) => Some(rootfs::open(cas, d)?),
        None => None,
    };
    // The step's own timeout, and never longer than the lease: a step that
    // hangs must not hold the worker's lock (one worker per state dir) past
    // the point where the next worker would treat this claim as abandoned.
    let cap = (req.timeout_ms > 0)
        .then(|| Duration::from_millis(req.timeout_ms))
        .map_or(LEASE, |t| t.min(LEASE));
    let timeout = Some(cap);
    let mut phases = Phases::default();
    say!(
        "QUOD  AUDIT  pending audit {}: would-be hit {} of step b3:{} (replay {}, enqueued {})",
        p.id,
        p.nth,
        key.short(),
        &p.replay_run[..p.replay_run.len().min(8)],
        quod_replay::human_time(&p.enqueued_at)
    );
    let ex = match req.kind {
        Kind::Run => {
            let warm_dir = match (req.warm, &spec.profile().warm) {
                (true, Some(w)) => Some(state.warm_cache_dir(&spec.image.id()?, w.cache_env)),
                _ => None,
            };
            let guard = if req.guard {
                let g = GoTestGuard::for_step(
                    req.script.as_deref().unwrap_or_default(),
                    &req.extra_env,
                    spec.profile(),
                    &cas.dir().join("go-test-guard"),
                );
                // The key names its layout's overlay; a probe that now
                // picks another (a package-layout check that failed for a
                // passing reason) cannot run this key. An error: the row
                // is released for another attempt (MAX_ATTEMPTS bounds it).
                if let Some(g) = &g
                    && Some(g.layout()) != spec.guard_layout()
                {
                    anyhow::bail!(
                        "the go test cache guard's layout is {} on this host now, not the key's",
                        g.layout().name()
                    );
                }
                if g.is_none() {
                    // The go on the profile's PATH changed (an upgrade):
                    // running the guarded key unguarded would let go test
                    // serve cached results. No retry changes that.
                    cleanup_dir(&dir);
                    return lose(
                        ledger,
                        p,
                        "the go test cache guard no longer applies to this step on this host",
                    );
                }
                g
            } else {
                None
            };
            // An audit checks what the tests do now: go test may store
            // results but not serve any from its cache.
            let go_tests = GoTests {
                guard: guard.as_ref(),
                expire: guard.is_some(),
            };
            run::execute(
                cas,
                &tree,
                &spec,
                env,
                &dir,
                warm_dir.as_deref(),
                root.as_ref(),
                &[],
                timeout,
                go_tests,
                &mut phases,
            )
        }
        Kind::Step => {
            let root = root
                .as_ref()
                .context("a runner-API step needs its rootfs")?;
            crate::step::execute_with_mounts(cas, &tree, &spec, env, &dir, root)
        }
    };
    let ex = match ex {
        Ok(v) => v,
        Err(e) => {
            let _ = quod_cas::remove_any(&dir);
            return Err(e);
        }
    };
    let cleanup = || cleanup_dir(&dir);
    // Not a verdict: the step did not finish, the machine failed it, or
    // the result is not the step's. None of these counts as passed. A
    // timeout here is the box's as likely as the step's (the worker runs
    // in the idle class and is starved by any job beside it), so it is
    // another attempt, not a loss, and the manifest's proof streak stays:
    // an inline audit that times out resets it because the user saw it.
    if let Some(killed) = ex.killed {
        cleanup();
        let why = match killed {
            Killed::Timeout => {
                let what = format!("timed out after {}", quod_replay::fmt_duration(cap));
                ledger.record_run_facts(&run_facts(
                    &run_id,
                    &key,
                    &req,
                    "timeout",
                    None,
                    Some(i32::from(EXIT_TIMEOUT)),
                    ex.wall,
                    (ex.transport, &ex.observer),
                ))?;
                format!("the step {what} under the worker (idle priority): not a verdict")
            }
            Killed::Aborted => "the observer failed while the step ran".to_string(),
            Killed::Terminated(sig) => format!("the worker was terminated by signal {sig}"),
        };
        let now = ledger.release_audit(p.id, MAX_ATTEMPTS, &why)?;
        say!(
            "QUOD  AUDIT  pending audit {}: {why}; now {}",
            p.id,
            now.as_str()
        );
        return Ok(None);
    }
    if let Some(why) = match (ex.signal, &ex.resource_failure) {
        (Some(sig), _) => Some(format!("the step was killed by signal {sig}")),
        (None, Some(why)) => Some(why.clone()),
        _ => None,
    } {
        cleanup();
        ledger.record_run_facts(&run_facts(
            &run_id,
            &key,
            &req,
            "killed",
            None,
            Some(ex.exit_code),
            ex.wall,
            (ex.transport, &ex.observer),
        ))?;
        let now = ledger.release_audit(p.id, MAX_ATTEMPTS, &why)?;
        say!(
            "QUOD  AUDIT  pending audit {}: {why}; now {}",
            p.id,
            now.as_str()
        );
        return Ok(None);
    }
    let tainted = if req.guard
        && let Some(p) = gotest::unguarded_toolchain(ex.manifest.entries.iter().map(|e| &e.p))
    {
        Some(format!(
            "a Go toolchain in the workspace ({p}) ran outside the test-cache guard"
        ))
    } else if ex.sealed_write_failed {
        Some("a write hit a read-only sealed dependency tree (EROFS)".to_string())
    } else {
        None
    };
    if let Some(why) = tainted {
        cleanup();
        ledger.record_run_facts(&run_facts(
            &run_id,
            &key,
            &req,
            "tainted",
            None,
            Some(ex.exit_code),
            ex.wall,
            (ex.transport, &ex.observer),
        ))?;
        return lose(ledger, p, &format!("{why}: not the step's result"));
    }

    let mid = quod_manifest::manifest_id(&ex.manifest)?;
    let result = StepResult {
        v: quod_manifest::VERSION,
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
        commands: vec![],
    };
    // One transaction: the verdict (which settles the pending row and, on a
    // divergence, quarantines and marks the suspects), the run row, the
    // manifest and the result.
    let verdict = ledger.atomically(|l| -> anyhow::Result<Verdict> {
        let v =
            trust.record_execution(&tstep, &run_id, &ex.manifest, &result, Some(&hit), &reason)?;
        l.record_execution_facts(
            &run_facts(
                &run_id,
                &key,
                &req,
                v.outcome(),
                Some(&mid),
                Some(ex.exit_code),
                ex.wall,
                (ex.transport, &ex.observer),
            ),
            &ex.manifest,
            &result,
        )?;
        Ok(v)
    })?;
    cleanup();
    say!("QUOD  EXECUTED{}", verdict.headline());
    for l in verdict.lines() {
        say!(
            "{}",
            if l.starts_with("QUOD") || l.starts_with("  ") {
                l
            } else {
                format!("  {l}")
            }
        );
    }
    say!("  ran {}", quod_replay::fmt_duration(ex.wall));
    tracing::info!(
        target: "quod",
        pending = p.id,
        run_id = %run_id,
        step_key = %key,
        manifest_id = %mid,
        nth = p.nth,
        replay_run = %p.replay_run,
        enqueued_at = %p.enqueued_at,
        outcome = verdict.outcome(),
        diverged = verdict.divergence.is_some(),
        suspects = verdict.suspects,
        exit_code = ex.exit_code,
        exec_ms = ex.wall.as_millis() as u64,
        total_ms = t0.elapsed().as_millis() as u64,
        "audit"
    );
    Ok(Some(verdict))
}

fn cleanup_dir(dir: &Path) {
    let _ = quod_cas::remove_any(dir);
}

#[allow(clippy::too_many_arguments)] // one row, every fact named
fn run_facts<'a>(
    run_id: &'a str,
    key: &'a Hash,
    req: &'a Request,
    outcome: &'a str,
    manifest: Option<&'a Hash>,
    exit_code: Option<i32>,
    wall: Duration,
    observed: (&'a str, &'a str),
) -> RunFacts<'a> {
    RunFacts {
        id: run_id,
        commit_sha: req.repo.as_ref().and_then(|r| r.head.as_deref()),
        host: req.repo.as_ref().and_then(|r| r.host.as_deref()),
        observed: Observed {
            transport: Some(observed.0),
            observer: Some(observed.1),
        },
        step_key: Some(key),
        repo: req.repo.as_ref().map(|r| r.id.as_str()),
        repo_path: req.repo.as_ref().map(|r| r.path.as_str()),
        mode: Some("replay"),
        outcome: Some(outcome),
        manifest_id: manifest,
        exit_code,
        wall_ms: Some(wall.as_millis() as u64),
        saved_ms: None,
    }
}

/// A line for `status`: the state dir's deferred audits, when any are
/// outstanding or lost.
pub fn status_line(ledger: &Ledger) -> anyhow::Result<Option<String>> {
    let o = ledger.outstanding(&stale_cutoff())?;
    if o.pending + o.overdue + o.running + o.lost == 0 {
        return Ok(None);
    }
    Ok(Some(format!(
        "  deferred audits: {} pending, {} running, {} lost and not yet repaid (a lost or overdue audit never counts as passed: the key's next would-be hit is audited inline); `quod audit` runs them{}",
        o.pending,
        o.running,
        o.lost + o.overdue,
        if o.overdue > 0 {
            format!(
                " ({} pending for longer than {} h: overdue)",
                o.overdue,
                PENDING_TTL.as_secs() / 3600
            )
        } else {
            String::new()
        }
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_round_trip() {
        let spec = StepSpec::new(
            Toolchain::Go,
            Image::rootfs(Hash::of(b"img")),
            "go test ./...",
            &[("A".into(), "b".into())],
        );
        let req = Request {
            v: REQUEST_VERSION,
            kind: Kind::Run,
            spec: (&spec).into(),
            script: Some("go test ./...".into()),
            extra_env: vec![("A".into(), "b".into())],
            policy: (&Policy::default()).into(),
            repo: Some(RepoDoc {
                id: "r".into(),
                path: "/r".into(),
                head: None,
                host: Some("box".into()),
            }),
            rootfs: None,
            warm: true,
            guard: false,
            ignore: vec![".quod/gocache".into()],
            timeout_ms: 1_800_000,
        };
        let back: Request = serde_json::from_slice(&req.to_json().unwrap()).unwrap();
        let spec2: StepSpec = back.spec.into();
        assert_eq!(spec2, spec);
        assert_eq!(back.kind, Kind::Run);
        assert!(back.warm && !back.guard && back.repo.unwrap().host.as_deref() == Some("box"));
        assert!(back.policy.policy().defer);
    }
}
